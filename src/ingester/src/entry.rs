// Copyright 2025 OpenObserve Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{
    io::{Cursor, Read},
    sync::Arc,
};

use arrow::{array::Int64Array, record_batch::RecordBatch};
use arrow_schema::Schema;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use config::utils::record_batch_ext::{RecordBatchExt, convert_json_to_record_batch};
use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::errors::*;
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub stream: Arc<str>,
    pub schema: Option<Arc<Schema>>,
    pub schema_key: Arc<str>,
    pub partition_key: Arc<str>, // 2023/12/18/00/country=US/state=CA
    pub data: Vec<Arc<serde_json::Value>>,
    pub data_size: usize,
}

impl Entry {
    pub fn new() -> Self {
        Self {
            stream: "".into(),
            schema: None,
            schema_key: "".into(),
            partition_key: "".into(),
            data: Vec::new(),
            data_size: 0,
        }
    }
    pub fn into_bytes(&mut self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(4096);
        let stream = self.stream.as_bytes();
        let schema_key = self.schema_key.as_bytes();
        let partition_key = self.partition_key.as_bytes();
        let data = serde_json::to_vec(&self.data).context(JSONSerializationSnafu)?;
        let data_size = data.len();
        self.data_size = data_size; // reset data size
        buf.write_u16::<BigEndian>(stream.len() as u16)
            .context(WriteDataSnafu)?;
        buf.extend_from_slice(stream);
        buf.write_u16::<BigEndian>(schema_key.len() as u16)
            .context(WriteDataSnafu)?;
        buf.extend_from_slice(schema_key);
        buf.write_u16::<BigEndian>(partition_key.len() as u16)
            .context(WriteDataSnafu)?;
        buf.extend_from_slice(partition_key);
        buf.write_u32::<BigEndian>(data_size as u32)
            .context(WriteDataSnafu)?;
        buf.extend_from_slice(&data);
        Ok(buf)
    }

    pub fn from_bytes(value: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(value);
        let stream_len = cursor.read_u16::<BigEndian>().context(ReadDataSnafu)?;
        let mut stream = vec![0; stream_len as usize];
        cursor.read_exact(&mut stream).context(ReadDataSnafu)?;
        let stream = String::from_utf8(stream).context(FromUtf8Snafu)?;
        let schema_key_len = cursor.read_u16::<BigEndian>().context(ReadDataSnafu)?;
        let mut schema_key = vec![0; schema_key_len as usize];
        cursor.read_exact(&mut schema_key).context(ReadDataSnafu)?;
        let schema_key = String::from_utf8(schema_key).context(FromUtf8Snafu)?;
        let partition_key_len = cursor.read_u16::<BigEndian>().context(ReadDataSnafu)?;
        let mut partition_key = vec![0; partition_key_len as usize];
        cursor
            .read_exact(&mut partition_key)
            .context(ReadDataSnafu)?;
        let partition_key = String::from_utf8(partition_key).context(FromUtf8Snafu)?;
        let data_len = cursor.read_u32::<BigEndian>().context(ReadDataSnafu)?;
        let mut data = vec![0; data_len as usize];
        cursor.read_exact(&mut data).context(ReadDataSnafu)?;
        let data = serde_json::from_slice(&data).context(JSONSerializationSnafu)?;
        Ok(Self {
            stream: stream.into(),
            schema: None,
            schema_key: schema_key.into(),
            partition_key: partition_key.into(),
            data,
            data_size: data_len as usize,
        })
    }

    pub fn into_batch(
        &self,
        stream_type: Arc<str>,
        schema: Arc<Schema>,
    ) -> Result<Arc<RecordBatchEntry>> {
        let batch =
            convert_json_to_record_batch(&schema, &self.data).context(ArrowJsonEncodeSnafu)?;

        let arrow_size = batch.size();
        Ok(RecordBatchEntry::new(
            stream_type,
            batch,
            self.data_size,
            arrow_size,
        ))
    }

    /// Batch convert multiple entries into RecordBatchEntry objects
    ///
    /// This is more efficient than calling into_batch() individually because:
    /// 1. Groups entries by schema to minimize schema validation overhead
    /// 2. Converts all data with same schema in one call
    /// 3. Better memory locality and cache utilization
    ///
    /// # Arguments
    /// * `entries` - Slice of entries to convert
    /// * `stream_type` - The stream type (e.g., "traces", "logs")
    ///
    /// # Returns
    /// Vector of RecordBatchEntry in the same order as input entries
    pub fn into_batch_bulk(
        entries: &[Entry],
        stream_type: Arc<str>,
    ) -> Result<Vec<Arc<RecordBatchEntry>>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Group entries by schema to batch convert entries with same schema
        let mut schema_groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();

        for (idx, entry) in entries.iter().enumerate() {
            let schema_key = entry.schema_key.to_string();
            schema_groups.entry(schema_key).or_default().push(idx);
        }

        // Prepare result vector with placeholders
        let mut results: Vec<Option<Arc<RecordBatchEntry>>> = vec![None; entries.len()];

        // Process each schema group
        for (_schema_key, indices) in schema_groups {
            if indices.is_empty() {
                continue;
            }

            // Get schema from first entry in group (all entries in group have same schema)
            let first_idx = indices[0];
            let schema = entries[first_idx].schema.clone().ok_or_else(|| {
                crate::errors::Error::ArrowJsonEncodeError {
                    source: arrow_schema::ArrowError::SchemaError(
                        "Entry missing schema".to_string(),
                    ),
                }
            })?;

            // Collect all JSON data from entries in this group
            let mut all_data: Vec<Arc<serde_json::Value>> = Vec::new();
            let mut entry_data_counts: Vec<usize> = Vec::new();

            for &idx in &indices {
                let entry = &entries[idx];
                entry_data_counts.push(entry.data.len());
                all_data.extend(entry.data.iter().cloned());
            }

            // Batch convert all data at once
            let combined_batch =
                convert_json_to_record_batch(&schema, &all_data).context(ArrowJsonEncodeSnafu)?;

            let arrow_size = combined_batch.size();

            // Split the combined batch back into individual entry batches
            let mut row_offset = 0;
            for (&idx, &count) in indices.iter().zip(entry_data_counts.iter()) {
                if count == 0 {
                    // Handle empty entry
                    let empty_batch = RecordBatch::new_empty(schema.clone());
                    results[idx] = Some(RecordBatchEntry::new(
                        stream_type.clone(),
                        empty_batch,
                        0,
                        0,
                    ));
                    continue;
                }

                // Slice the combined batch for this entry
                let entry_batch = combined_batch.slice(row_offset, count);
                row_offset += count;

                // Calculate proportional sizes
                let entry_json_size = entries[idx].data_size;
                let entry_arrow_size = (arrow_size * count) / all_data.len().max(1);

                results[idx] = Some(RecordBatchEntry::new(
                    stream_type.clone(),
                    entry_batch,
                    entry_json_size,
                    entry_arrow_size,
                ));
            }
        }

        // Convert to final result, handling any missing entries as errors
        results
            .into_iter()
            .enumerate()
            .map(|(idx, opt)| {
                opt.ok_or_else(|| crate::errors::Error::ArrowJsonEncodeError {
                    source: arrow_schema::ArrowError::SchemaError(format!(
                        "Failed to convert entry at index {idx}",
                    )),
                })
            })
            .collect()
    }
}

impl Default for Entry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RecordBatchEntry {
    pub data: RecordBatch,
    pub data_json_size: usize,
    pub data_arrow_size: usize,
    pub min_ts: i64,
    pub max_ts: i64,
}

impl RecordBatchEntry {
    pub fn new(
        stream_type: Arc<str>,
        data: RecordBatch,
        data_json_size: usize,
        data_arrow_size: usize,
    ) -> Arc<RecordBatchEntry> {
        let (min_ts, max_ts) = if stream_type == Arc::from("index") {
            pop_time_range(&data, Some("min_ts"), Some("max_ts"))
        } else {
            pop_time_range(&data, None, None)
        };
        Arc::new(Self {
            data,
            data_json_size,
            data_arrow_size,
            min_ts,
            max_ts,
        })
    }
}

fn pop_time_range(
    batch: &RecordBatch,
    min_field: Option<&str>,
    max_field: Option<&str>,
) -> (i64, i64) {
    let mut min_ts = 0;
    let mut max_ts = 0;
    let time_field = config::TIMESTAMP_COL_NAME.to_string();
    let min_field = min_field.unwrap_or(time_field.as_str());
    let max_field = max_field.unwrap_or(time_field.as_str());
    if min_field == max_field {
        let Some(col) = batch.column_by_name(min_field) else {
            return (0, 0);
        };
        let Some(col) = col.as_any().downcast_ref::<Int64Array>() else {
            return (0, 0);
        };
        for v in col.values() {
            if min_ts == 0 || min_ts > *v {
                min_ts = *v;
            }
            if max_ts < *v {
                max_ts = *v;
            }
        }
        return (min_ts, max_ts);
    }

    // min_ts
    let Some(col) = batch.column_by_name(min_field) else {
        return (0, 0);
    };
    let Some(col) = col.as_any().downcast_ref::<Int64Array>() else {
        return (0, 0);
    };
    for v in col.values() {
        if min_ts == 0 || min_ts > *v {
            min_ts = *v;
        }
    }

    // max_ts
    let Some(col) = batch.column_by_name(max_field) else {
        return (0, 0);
    };
    let Some(col) = col.as_any().downcast_ref::<Int64Array>() else {
        return (0, 0);
    };
    for v in col.values() {
        if max_ts < *v {
            max_ts = *v;
        }
    }

    (min_ts, max_ts)
}

#[derive(Default)]
pub struct PersistStat {
    pub json_size: i64,
    pub arrow_size: usize,
    pub file_num: usize,
    pub batch_num: usize,
    pub records: usize,
}

impl std::ops::Add for PersistStat {
    type Output = PersistStat;

    fn add(self, other: PersistStat) -> PersistStat {
        PersistStat {
            json_size: self.json_size + other.json_size,
            arrow_size: self.arrow_size + other.arrow_size,
            file_num: self.file_num + other.file_num,
            batch_num: self.batch_num + other.batch_num,
            records: self.records + other.records,
        }
    }
}

impl std::ops::AddAssign for PersistStat {
    fn add_assign(&mut self, other: PersistStat) {
        self.json_size += other.json_size;
        self.arrow_size += other.arrow_size;
        self.file_num += other.file_num;
        self.batch_num += other.batch_num;
        self.records += other.records;
    }
}
