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
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use arrow_schema::Schema;
use chrono::{Duration, Utc};
use config::{
    MEM_TABLE_INDIVIDUAL_STREAMS, get_config, metrics,
    utils::hash::{Sum64, gxhash},
};
use hashbrown::HashSet;
use once_cell::sync::Lazy;
use snafu::ResultExt;
use tokio::sync::{RwLock, mpsc};
use wal::{Writer as WalWriter, build_file_path};

use crate::{
    ReadRecordBatchEntry, WriterSignal,
    entry::Entry,
    errors::*,
    immutable::{IMMUTABLES, Immutable},
    memtable::MemTable,
    rwmap::RwMap,
};

static WRITERS: Lazy<Vec<RwMap<WriterKey, Arc<Writer>>>> = Lazy::new(|| {
    let cfg = get_config();
    let writer_num = cfg.limit.mem_table_bucket_num + MEM_TABLE_INDIVIDUAL_STREAMS.len();
    let mut writers = Vec::with_capacity(writer_num);
    for _ in 0..writer_num {
        writers.push(RwMap::default());
    }
    writers
});

static WAL_RUNTIMES: Lazy<Vec<Option<Arc<tokio::runtime::Runtime>>>> = Lazy::new(|| {
    let cfg = get_config();
    if !cfg.common.wal_dedicated_runtime_enabled {
        return vec![];
    }

    let writer_num = cfg.limit.mem_table_bucket_num + MEM_TABLE_INDIVIDUAL_STREAMS.len();
    let mut runtimes = Vec::with_capacity(writer_num);

    for idx in 0..writer_num {
        runtimes.push(create_dedicated_runtime_for_idx(idx));
    }

    log::info!(
        "[INGESTER:RUNTIME] Created {} dedicated runtimes for WAL writers",
        runtimes.iter().filter(|r| r.is_some()).count()
    );

    runtimes
});

pub struct Writer {
    idx: usize,
    key: WriterKey,
    wal: Arc<RwLock<WalWriter>>,
    memtable: Arc<RwLock<MemTable>>,
    next_seq: AtomicU64,
    created_at: AtomicI64,
    write_queue: Arc<mpsc::Sender<(WriterSignal, Vec<Entry>, bool)>>,
}

// check total memtable size
pub fn check_memtable_size() -> Result<()> {
    let cur_mem = metrics::INGEST_MEMTABLE_ARROW_BYTES
        .with_label_values(&[])
        .get();
    if cur_mem >= get_config().limit.mem_table_max_size as i64 {
        Err(Error::MemoryTableOverflowError {})
    } else {
        Ok(())
    }
}

// check total memory size
pub fn check_memory_circuit_breaker() -> Result<()> {
    let cfg = get_config();
    if !cfg.common.memory_circuit_breaker_enabled || cfg.common.memory_circuit_breaker_ratio == 0 {
        return Ok(());
    }
    let cur_mem = metrics::NODE_MEMORY_USAGE.with_label_values(&[]).get() as usize;
    if cur_mem > cfg.limit.mem_total / 100 * cfg.common.memory_circuit_breaker_ratio {
        Err(Error::MemoryCircuitBreakerError {})
    } else {
        Ok(())
    }
}

fn get_table_idx(thread_id: usize, stream_name: &str) -> usize {
    if let Some(idx) = MEM_TABLE_INDIVIDUAL_STREAMS.get(stream_name) {
        *idx
    } else {
        let hash_key = format!("{thread_id}_{stream_name}");
        let hash_id = gxhash::new().sum64(&hash_key);
        hash_id as usize % (WRITERS.len() - MEM_TABLE_INDIVIDUAL_STREAMS.len())
    }
}

/// Get a writer for a given org_id and stream_type
pub async fn get_writer(
    thread_id: usize,
    org_id: &str,
    stream_type: &str,
    stream_name: &str,
) -> Arc<Writer> {
    let start = std::time::Instant::now();
    let key = WriterKey::new(org_id, stream_type);
    let idx = get_table_idx(thread_id, stream_name);
    let r = WRITERS[idx].read().await;
    let data = r.get(&key);
    if start.elapsed().as_millis() > 500 {
        log::warn!(
            "get_writer from read cache took: {} ms",
            start.elapsed().as_millis()
        );
    }
    let mut is_existing_writer_channel_closed = false;
    if let Some(w) = data {
        if !w.is_channel_closed() {
            return w.clone();
        }
        is_existing_writer_channel_closed = true;
    }
    drop(r);

    if is_existing_writer_channel_closed {
        log::warn!(
            "[INGESTER:MEM:{idx}] Writer channel closed for {org_id}/{stream_type}, removing from cache",
        );
        let mut w = WRITERS[idx].write().await;
        w.remove(&key);
        drop(w);
    }

    // slow path
    let start = std::time::Instant::now();
    let mut rw = WRITERS[idx].write().await;
    let w = rw
        .entry(key.clone())
        .or_insert_with(|| Writer::new(idx, key));
    if start.elapsed().as_millis() > 500 {
        log::warn!(
            "get_writer from write cache took: {} ms",
            start.elapsed().as_millis()
        );
    }
    w.clone()
}

pub async fn read_from_memtable(
    org_id: &str,
    stream_type: &str,
    stream_name: &str,
    time_range: Option<(i64, i64)>,
    partition_filters: &[(String, Vec<String>)],
) -> Result<Vec<ReadRecordBatchEntry>> {
    let cfg = get_config();
    let key = WriterKey::new(org_id, stream_type);
    // fast past
    if cfg.limit.mem_table_bucket_num <= 1 {
        let idx = get_table_idx(0, stream_name);
        let w = WRITERS[idx].read().await;
        return match w.get(&key) {
            Some(r) => r.read(stream_name, time_range, partition_filters).await,
            None => Ok(Vec::new()),
        };
    }
    // slow path
    let mut batches = Vec::new();
    let mut visited = HashSet::with_capacity(cfg.limit.mem_table_bucket_num);
    for thread_id in 0..cfg.limit.http_worker_num {
        let idx = get_table_idx(thread_id, stream_name);
        if visited.contains(&idx) {
            continue;
        }
        visited.insert(idx);
        let w = WRITERS[idx].read().await;
        if let Some(r) = w.get(&key)
            && let Ok(data) = r.read(stream_name, time_range, partition_filters).await
        {
            batches.extend(data);
        }
    }
    Ok(batches)
}

pub async fn check_ttl() -> Result<()> {
    for w in WRITERS.iter() {
        let w = w.read().await;
        for r in w.values() {
            if let Err(e) = r
                .write_queue
                .send((WriterSignal::Rotate, vec![], false))
                .await
            {
                log::error!("[INGESTER:MEM:{}] writer queue rotate error: {e}", r.idx);
            }
        }
    }
    Ok(())
}

pub async fn flush_all() -> Result<()> {
    for w in WRITERS.iter() {
        let mut w = w.write().await;
        let keys = w.keys().cloned().collect::<Vec<_>>();
        for r in w.values() {
            r.close().await?; // close writer
            metrics::INGEST_MEMTABLE_FILES.with_label_values(&[]).dec();
        }
        for key in keys {
            w.remove(&key);
        }
    }
    Ok(())
}

impl Writer {
    pub(crate) fn new(idx: usize, key: WriterKey) -> Arc<Writer> {
        let now = Utc::now().timestamp_micros();
        let cfg = get_config();
        let next_seq = AtomicU64::new(now as u64);
        let wal_id = next_seq.fetch_add(1, Ordering::SeqCst);
        let wal_dir = PathBuf::from(&cfg.common.data_wal_dir)
            .join("logs")
            .join(idx.to_string());
        log::info!(
            "[INGESTER:MEM:{idx}] create file: {}/{}/{}/{}.wal",
            wal_dir.display(),
            &key.org_id,
            &key.stream_type,
            wal_id
        );

        let (tx, rx) = mpsc::channel(cfg.limit.wal_write_queue_size);

        let writer = Self {
            idx,
            key: key.clone(),
            wal: Arc::new(RwLock::new(
                WalWriter::new(
                    build_file_path(wal_dir, &key.org_id, &key.stream_type, wal_id.to_string()),
                    cfg.limit.max_file_size_on_disk as u64,
                    cfg.limit.wal_write_buffer_size,
                    None,
                )
                .expect("wal file create error")
                .0,
            )),
            memtable: Arc::new(RwLock::new(MemTable::new())),
            next_seq,
            created_at: AtomicI64::new(now),
            write_queue: Arc::new(tx),
        };
        let writer = Arc::new(writer);
        let writer_clone = writer.clone();

        log::info!("[INGESTER:MEM:{idx}] writer queue start consuming");
        let dedicated_runtime = if idx < WAL_RUNTIMES.len() {
            WAL_RUNTIMES[idx].clone()
        } else {
            None
        };

        // 在独立 runtime 上 spawn 消费任务，或使用默认 runtime
        if let Some(rt) = dedicated_runtime {
            rt.spawn(async move {
                Self::consume_loop(writer, rx, idx).await;
            });
        } else {
            tokio::spawn(async move {
                Self::consume_loop(writer, rx, idx).await;
            });
        }

        writer_clone
    }

    async fn consume_loop(
        writer: Arc<Writer>,
        mut rx: mpsc::Receiver<(WriterSignal, Vec<Entry>, bool)>,
        idx: usize,
    ) {
        let mut total: usize = 0;
        loop {
            match rx.recv().await {
                None => break,
                Some((sign, entries, fsync)) => match sign {
                    WriterSignal::Close => break,
                    WriterSignal::Rotate => {
                        if let Err(e) = writer.rotate(0, 0).await {
                            log::error!("[INGESTER:MEM:{idx}] writer rotate error: {e}");
                        }
                    }
                    WriterSignal::Produce => {
                        let (entries_records, _) = entries
                            .iter()
                            .map(|entry| (entry.data.len(), entry.data_size))
                            .fold((0, 0), |(acc_records, acc_size), (records, size)| {
                                (acc_records + records, acc_size + size)
                            });

                        metrics::WAL_CONSUME_INGEST_RECORDS
                            .with_label_values(&["default", "traces"])
                            .inc_by(entries_records as u64);
                        metrics::WAL_CONSUME_INGEST_COUNT
                            .with_label_values(&["default", "traces"])
                            .inc_by(1);
                        // if let Err(e) = writer.consume(entries, fsync).await {
                        //     log::error!("[INGESTER:MEM:{idx}] writer consume batch error: {e}");
                        // }
                    }
                },
            }
            total += 1;
            if total % 1000 == 0 {
                log::info!(
                    "[INGESTER:MEM:{idx}] writer queue consuming, total: {}, in queue: {}",
                    total,
                    rx.len()
                );
            }
        }
        log::info!("[INGESTER:MEM:{idx}] writer queue closed");
    }

    pub fn get_key_str(&self) -> String {
        format!("{}/{}", self.key.org_id, self.key.stream_type)
    }

    pub fn is_channel_closed(&self) -> bool {
        self.write_queue.is_closed()
    }

    // check_ttl is used to check if the memtable has expired
    pub async fn write(&self, schema: Arc<Schema>, mut entry: Entry, fsync: bool) -> Result<()> {
        if entry.data.is_empty() {
            return Ok(());
        }

        entry.schema = Some(schema);
        self.write_batch(vec![entry], fsync).await
    }

    pub async fn write_batch(&self, entries: Vec<Entry>, fsync: bool) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let cfg = get_config();
        if !cfg.common.wal_write_queue_enabled {
            return self.consume(entries, fsync).await;
        }

        if cfg.common.wal_write_queue_full_reject {
            if let Err(e) = self
                .write_queue
                .try_send((WriterSignal::Produce, entries, fsync))
            {
                log::error!(
                    "[INGESTER:MEM:{}] write queue full, reject write: {}",
                    self.idx,
                    e
                );
                return Err(Error::WalError {
                    source: wal::Error::WriteQueueFull { idx: self.idx },
                });
            }
        } else {
            self.write_queue
                .send((WriterSignal::Produce, entries, fsync))
                .await
                .context(TokioMpscSendEntriesSnafu)?;
        }

        Ok(())
    }

    async fn consume(&self, mut entries: Vec<Entry>, fsync: bool) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let bytes_entries = entries
            .iter_mut()
            .map(|entry| entry.into_bytes())
            .collect::<Result<Vec<_>>>()?;
        let batch_entries = entries
            .iter()
            .map(|entry| {
                entry.into_batch(self.key.stream_type.clone(), entry.schema.clone().unwrap())
            })
            .collect::<Result<Vec<_>>>()?;
        let (entries_json_size, entries_arrow_size) = batch_entries
            .iter()
            .map(|entry| (entry.data_json_size, entry.data_arrow_size))
            .fold(
                (0, 0),
                |(acc_json_size, acc_arrow_size), (json_size, arrow_size)| {
                    (acc_json_size + json_size, acc_arrow_size + arrow_size)
                },
            );

        // check rotation
        self.rotate(entries_json_size, entries_arrow_size).await?;

        // write into wal
        let start = std::time::Instant::now();
        let mut wal = self.wal.write().await;
        let wal_lock_time = start.elapsed().as_millis() as f64;
        metrics::INGEST_WAL_LOCK_TIME
            .with_label_values(&[&self.key.org_id])
            .observe(wal_lock_time);
        for entry in bytes_entries {
            if entry.is_empty() {
                continue;
            }
            wal.write(&entry).context(WalSnafu)?;
            tokio::task::coop::consume_budget().await;
        }
        drop(wal);

        // write into memtable
        let start = std::time::Instant::now();
        let mut mem = self.memtable.write().await;
        let mem_lock_time = start.elapsed().as_millis() as f64;
        metrics::INGEST_MEMTABLE_LOCK_TIME
            .with_label_values(&[&self.key.org_id])
            .observe(mem_lock_time);
        for (entry, batch) in entries.into_iter().zip(batch_entries) {
            if entry.data_size == 0 {
                continue;
            }
            mem.write(entry.schema.clone().unwrap(), entry, batch)?;
            tokio::task::coop::consume_budget().await;
        }
        drop(mem);

        // check fsync
        if fsync {
            let mut wal = self.wal.write().await;
            wal.sync().context(WalSnafu)?;
            drop(wal);
        }

        Ok(())
    }

    // rotate is used to rotate the wal and memtable if the size exceeds the threshold
    async fn rotate(&self, entry_bytes_size: usize, entry_batch_size: usize) -> Result<()> {
        if !self.check_wal_threshold(self.wal.read().await.size(), entry_bytes_size)
            && !self.check_mem_threshold(self.memtable.read().await.size(), entry_batch_size)
        {
            return Ok(());
        }

        // rotation wal
        let start = std::time::Instant::now();
        let mut wal = self.wal.write().await;
        let wal_lock_time = start.elapsed().as_millis() as f64;
        metrics::INGEST_WAL_LOCK_TIME
            .with_label_values(&[&self.key.org_id])
            .observe(wal_lock_time);
        if !self.check_wal_threshold(wal.size(), entry_bytes_size) {
            return Ok(()); // check again to avoid race condition
        }
        let cfg = get_config();
        let wal_id = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let wal_dir = PathBuf::from(&cfg.common.data_wal_dir)
            .join("logs")
            .join(self.idx.to_string());
        log::info!(
            "[INGESTER:MEM] create file: {}/{}/{}/{}.wal",
            wal_dir.display(),
            &self.key.org_id,
            &self.key.stream_type,
            wal_id
        );
        let (new_wal, _header_size) = WalWriter::new(
            build_file_path(
                wal_dir,
                &self.key.org_id,
                &self.key.stream_type,
                wal_id.to_string(),
            ),
            cfg.limit.max_file_size_on_disk as u64,
            cfg.limit.wal_write_buffer_size,
            None,
        )
        .context(WalSnafu)?;
        wal.sync().context(WalSnafu)?; // sync wal before rotation
        let old_wal = std::mem::replace(&mut *wal, new_wal);
        drop(wal);

        // rotation memtable
        let new_mem = MemTable::new();
        let start = std::time::Instant::now();
        let mut mem = self.memtable.write().await;
        let mem_lock_time = start.elapsed().as_millis() as f64;
        metrics::INGEST_MEMTABLE_LOCK_TIME
            .with_label_values(&[&self.key.org_id])
            .observe(mem_lock_time);
        let old_mem = std::mem::replace(&mut *mem, new_mem);
        drop(mem);

        // update created_at
        self.created_at
            .store(Utc::now().timestamp_micros(), Ordering::Release);

        let path = old_wal.path().clone();
        let path_str = path.display().to_string();
        let table = Arc::new(Immutable::new(self.idx, self.key.clone(), old_mem));
        log::info!("[INGESTER:MEM] start add to IMMUTABLES, file: {path_str}");
        IMMUTABLES.write().await.insert(path, table);
        log::info!("[INGESTER:MEM] dones add to IMMUTABLES, file: {path_str}");

        Ok(())
    }

    pub async fn close(&self) -> Result<()> {
        // wait for all messages to be processed
        if let Err(e) = self
            .write_queue
            .send((WriterSignal::Close, vec![], true))
            .await
        {
            log::error!("[INGESTER:MEM:{}] close writer error: {}", self.idx, e);
        }
        self.write_queue.closed().await;

        // rotation wal
        let mut wal = self.wal.write().await;
        wal.sync().context(WalSnafu)?;
        let path = wal.path().clone();
        drop(wal);

        // rotation memtable
        let mut mem = self.memtable.write().await;
        let new_mem = MemTable::new();
        let old_mem = std::mem::replace(&mut *mem, new_mem);
        drop(mem);

        let table = Arc::new(Immutable::new(self.idx, self.key.clone(), old_mem));
        IMMUTABLES.write().await.insert(path, table);
        Ok(())
    }

    pub async fn read(
        &self,
        stream_name: &str,
        time_range: Option<(i64, i64)>,
        partition_filters: &[(String, Vec<String>)],
    ) -> Result<Vec<ReadRecordBatchEntry>> {
        let memtable = self.memtable.read().await;
        memtable.read(stream_name, time_range, partition_filters)
    }

    /// Check if the wal file size is over the threshold or the file is too old
    fn check_wal_threshold(&self, written_size: (usize, usize), data_size: usize) -> bool {
        let cfg = get_config();
        let (compressed_size, uncompressed_size) = written_size;
        compressed_size > wal::FILE_TYPE_IDENTIFIER_LEN
            && (compressed_size + data_size > cfg.limit.max_file_size_on_disk
                || uncompressed_size + data_size > cfg.limit.max_file_size_on_disk
                || self.created_at.load(Ordering::Relaxed)
                    + Duration::try_seconds(cfg.limit.max_file_retention_time as i64)
                        .unwrap()
                        .num_microseconds()
                        .unwrap()
                    <= Utc::now().timestamp_micros())
    }

    /// Check if the memtable size is over the threshold
    fn check_mem_threshold(&self, written_size: (usize, usize), data_size: usize) -> bool {
        let cfg = get_config();
        let (json_size, arrow_size) = written_size;
        json_size > 0
            && (json_size + data_size > cfg.limit.max_file_size_in_memory
                || arrow_size + data_size > cfg.limit.max_file_size_in_memory)
    }
}

/// 创建独立的 runtime 并绑定 CPU 核心（模块级函数）
fn create_dedicated_runtime_for_idx(idx: usize) -> Option<Arc<tokio::runtime::Runtime>> {
    let cfg = get_config();

    // 检查是否启用独立 runtime（通过配置控制）
    if !cfg.common.wal_dedicated_runtime_enabled {
        return None;
    }

    let total_cpus = cfg.limit.cpu_num;

    // 安全检查：至少需要 2 个 CPU 才能做隔离（1 个给 HTTP，1 个给 WAL）
    if total_cpus < 2 {
        log::warn!(
            "[INGESTER:MEM:{idx}] Cannot enable dedicated runtime: need at least 2 CPUs, got {total_cpus}"
        );
        return None;
    }

    // 新策略：无论 HTTP workers 配置多少，都为 WAL writers 预留 CPU
    // 预留策略：
    // - 小系统（<= 8 核）：预留 1 个 CPU
    // - 中等系统（9-32 核）：预留 max(1, total_cpus / 8) 个 CPU
    // - 大系统（> 32 核）：预留 max(4, total_cpus / 8) 个 CPU
    let reserved_cpus_for_wal = if total_cpus <= 8 {
        1
    } else if total_cpus <= 32 {
        std::cmp::max(1, total_cpus / 8)
    } else {
        std::cmp::max(4, total_cpus / 8)
    };

    // 确保预留的 CPU 数量合理（不超过总数的一半）
    let reserved_cpus_for_wal = std::cmp::min(reserved_cpus_for_wal, total_cpus / 2);

    // WAL writers 使用最后的几个 CPU 核心
    // 例如：8 核系统，预留 1 个 -> WAL 使用 CPU 7
    //       32 核系统，预留 4 个 -> WAL 使用 CPU 28-31
    let wal_cpu_start = total_cpus - reserved_cpus_for_wal;
    let cpu_id = wal_cpu_start + (idx % reserved_cpus_for_wal);

    // 双重检查：确保 cpu_id 在有效范围内
    if cpu_id >= total_cpus {
        log::error!(
            "[INGESTER:MEM:{idx}] BUG: Calculated CPU ID {cpu_id} exceeds total CPUs {total_cpus}, falling back to shared runtime"
        );
        return None;
    }

    log::info!(
        "[INGESTER:MEM:{idx}] Creating dedicated runtime on CPU core {cpu_id} (total CPUs: {total_cpus}, reserved for WAL: {reserved_cpus_for_wal}, HTTP can use: 0-{})",
        wal_cpu_start - 1
    );

    // 创建单线程 runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1) // 每个 writer 一个专用线程
        .thread_name(format!("wal-writer-{idx}"))
        .on_thread_start(move || {
            // 在线程启动时绑定 CPU 核心
            if let Some(core_ids) = core_affinity::get_core_ids() {
                if cpu_id < core_ids.len() {
                    if core_affinity::set_for_current(core_ids[cpu_id]) {
                        log::info!(
                                "[INGESTER:MEM:{idx}] Successfully bound WAL writer thread to CPU core {cpu_id}"
                            );
                    } else {
                        log::warn!(
                                "[INGESTER:MEM:{idx}] Failed to bind WAL writer thread to CPU core {cpu_id}"
                            );
                    }
                } else {
                    log::warn!(
                            "[INGESTER:MEM:{idx}] CPU core {cpu_id} not available, total cores: {}",
                            core_ids.len()
                        );
                }
            } else {
                log::warn!(
                        "[INGESTER:MEM:{idx}] Failed to get CPU core IDs for binding"
                    );
            }
        })
        .enable_all()
        .build();

    match runtime {
        Ok(rt) => {
            log::info!("[INGESTER:MEM:{idx}] Created dedicated runtime successfully");
            Some(Arc::new(rt))
        }
        Err(e) => {
            log::error!(
                "[INGESTER:MEM:{idx}] Failed to create dedicated runtime: {e}, falling back to shared runtime"
            );
            None
        }
    }
}
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct WriterKey {
    pub(crate) org_id: Arc<str>,
    pub(crate) stream_type: Arc<str>,
}

impl WriterKey {
    pub(crate) fn new<T>(org_id: T, stream_type: T) -> Self
    where
        T: AsRef<str>,
    {
        Self {
            org_id: Arc::from(org_id.as_ref()),
            stream_type: Arc::from(stream_type.as_ref()),
        }
    }
}
