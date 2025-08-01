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

use std::collections::HashMap;

use once_cell::sync::Lazy;
use prometheus::{
    CounterVec, Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder,
};

pub const NAMESPACE: &str = "zo";
const HELP_SUFFIX: &str =
    "Please include 'organization, 'stream type', and 'stream' labels for this metric.";
pub const SPAN_METRICS_BUCKET: [f64; 15] = [
    0.1, 0.5, 1.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0,
    60000.0,
];
// http latency
pub static HTTP_INCOMING_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "http_incoming_requests",
            "HTTP incoming requests.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[
            "endpoint",
            "status",
            "organization",
            "stream_type",
            "search_type",
            "search_group",
        ],
    )
    .expect("Metric created")
});
pub static HTTP_RESPONSE_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "http_response_time",
            "HTTP response time.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[
            "endpoint",
            "status",
            "organization",
            "stream_type",
            "search_type",
            "search_group",
        ],
    )
    .expect("Metric created")
});

// grpc latency
pub static GRPC_INCOMING_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "grpc_incoming_requests",
            "gRPC incoming requests.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[
            "endpoint",
            "status",
            "organization",
            "stream_type",
            "search_type",
            "search_group",
        ],
    )
    .expect("Metric created")
});
pub static GRPC_RESPONSE_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "grpc_response_time",
            "gRPC response time.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[
            "endpoint",
            "status",
            "organization",
            "stream_type",
            "search_type",
            "search_group",
        ],
    )
    .expect("Metric created")
});

// ingester stats
pub static INGEST_RECORDS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "ingest_records",
            "Ingested records.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static INGEST_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("ingest_bytes", "Ingested bytes.".to_owned() + HELP_SUFFIX)
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static INGEST_ERRORS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "ingest_errors",
            "Errors while ingesting records".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "stream", "error_type"],
    )
    .expect("Metric created")
});
pub static INGEST_WAL_USED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "ingest_wal_used_bytes",
            "Ingestor WAL used bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static INGEST_WAL_WRITE_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "ingest_wal_write_bytes",
            "Ingestor WAL write bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static INGEST_WAL_READ_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "ingest_wal_read_bytes",
            "Ingestor WAL read bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static INGEST_MEMTABLE_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "ingest_memtable_bytes",
            "Ingestor in memory bytes.".to_owned(),
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static INGEST_MEMTABLE_ARROW_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "ingest_memtable_arrow_bytes",
            "Ingestor arrow format in memory bytes.".to_owned(),
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static INGEST_MEMTABLE_FILES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "ingest_memtable_files",
            "Ingestor in memory files.".to_owned(),
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static INGEST_MEMTABLE_LOCK_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("ingest_memtable_lock_time", "ingest memtable lock time")
            .namespace(NAMESPACE)
            .buckets(vec![
                0.2, 0.5, 1.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0,
            ])
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

pub static INGEST_WAL_LOCK_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("ingest_wal_lock_time", "ingest wal lock time")
            .namespace(NAMESPACE)
            .buckets(vec![
                0.2, 0.5, 1.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0,
            ])
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

// querier memory cache stats
pub static QUERY_MEMORY_CACHE_LIMIT_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_memory_cache_limit_bytes",
            "Querier memory cache limit bytes",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static QUERY_MEMORY_CACHE_USED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_memory_cache_used_bytes",
            "Querier memory cache used bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static QUERY_MEMORY_CACHE_FILES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_memory_cache_files",
            "Querier memory cached files.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});

// querier disk cache stats
pub static QUERY_DISK_CACHE_LIMIT_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_disk_cache_limit_bytes",
            "Querier disk cache limit bytes",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static QUERY_DISK_CACHE_USED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_disk_cache_used_bytes",
            "Querier diskcache used bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static QUERY_DISK_CACHE_FILES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_disk_cache_files",
            "Querier disk cached files.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});

// querier disk result cache stats
pub static QUERY_DISK_RESULT_CACHE_USED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_disk_result_cache_used_bytes",
            "Querier disk result cache used bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "cache_type"],
    )
    .expect("Metric created")
});
pub static QUERY_DISK_METRICS_CACHE_USED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_disk_metrics_cache_used_bytes",
            "Querier disk metrics result cached bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

// query cache ratio for parquet files
pub static QUERY_PARQUET_CACHE_RATIO: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "query_parquet_cache_ratio",
            "Querier parquet cache ratio.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .buckets(vec![
            0.01, 0.05, 0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 1.0,
        ])
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static QUERY_PARQUET_CACHE_RATIO_NODE: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "query_parquet_cache_ratio_node",
            "Querier parquet cache ratio for local node.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .buckets(vec![
            0.01, 0.05, 0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 1.0,
        ])
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});

// query cache ratio for metrics
pub static QUERY_METRICS_CACHE_RATIO: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "query_metrics_cache_ratio",
            "Querier metrics cache ratio.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .buckets(vec![
            0.01, 0.05, 0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 1.0,
        ])
        .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

// compactor stats
pub static COMPACT_USED_TIME: Lazy<CounterVec> = Lazy::new(|| {
    CounterVec::new(
        Opts::new(
            "compact_used_time",
            "Compactor used time.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static COMPACT_MERGED_FILES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "compact_merged_files",
            "Compactor merged files.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static COMPACT_MERGED_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "compact_merged_bytes",
            "Compactor merged bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static COMPACT_PENDING_JOBS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "compact_pending_jobs",
            "Compactor pending jobs count. Please include 'organization and 'stream type' labels for this metric.".to_owned(),
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
// TODO deletion / archiving stats

// storage stats
pub static STORAGE_ORIGINAL_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "storage_original_bytes",
            "Storage original bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_COMPRESSED_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "storage_compressed_bytes",
            "Storage compressed bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_FILES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("storage_files", "Storage files.".to_owned() + HELP_SUFFIX)
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_RECORDS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "storage_records",
            "Storage records.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_WRITE_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "storage_write_requests",
            "Storage write requests.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "storage_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_WRITE_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "storage_write_bytes",
            "Storage write bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "storage_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_READ_REQUESTS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "storage_read_requests",
            "Storage read requests.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "method_type", "storage_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_READ_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "storage_read_bytes",
            "Storage read bytes.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "method_type", "storage_type"],
    )
    .expect("Metric created")
});
pub static STORAGE_TIME: Lazy<CounterVec> = Lazy::new(|| {
    CounterVec::new(
        Opts::new(
            "storage_time",
            "Storage response time.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "method_type", "storage_type"],
    )
    .expect("Metric created")
});

// metadata stats
pub static META_STORAGE_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_storage_bytes", "Metadata storage used bytes")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static META_STORAGE_KEYS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_storage_keys", "Metadata storage item keys")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static META_NUM_NODES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_nodes", "Metadata node nums")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["node_role"],
    )
    .expect("Metric created")
});
pub static META_NUM_ORGANIZATIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_organizations", "Metadata organization nums")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static META_NUM_STREAMS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_streams", "Metadata stream nums")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static META_NUM_USERS_TOTAL: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_users_total", "Metadata user total")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static META_NUM_USERS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_users", "Metadata user nums")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});
pub static META_NUM_FUNCTIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "meta_num_functions",
            "Metadata function nums.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "function_type"],
    )
    .expect("Metric created")
});
pub static META_NUM_ALERTS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "meta_num_alerts",
            "Metadata alert nums.".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type"],
    )
    .expect("Metric created")
});
pub static META_NUM_DASHBOARDS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("meta_num_dashboards", "Metadata dashboard nums")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

// metrics for query manager
pub static QUERY_RUNNING_NUMS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("query_running_nums", "Running query numbers")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});
pub static QUERY_PENDING_NUMS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("query_pending_nums", "Pending query numbers")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});
pub static QUERY_TIMEOUT_NUMS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("query_timeout_nums", "Timeout query numbers")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});
pub static QUERY_CANCELED_NUMS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("query_canceled_nums", "Cancel query numbers")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["organization"],
    )
    .expect("Metric created")
});

// This corresponds to mysql or pgsql queries, not sqlite as that is local and can be ignored
pub static DB_QUERY_NUMS: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new("db_query_nums", "db query number")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["operation", "table", "key"],
    )
    .expect("Metric created")
});

pub static DB_QUERY_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("db_query_time", "db query time.".to_owned())
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["operation", "table"],
    )
    .expect("Metric created")
});

pub static FILE_LIST_ID_SELECT_COUNT: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "file_list_id_select_count",
            "total number of ids returned by file list query",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static FILE_LIST_CACHE_HIT_COUNT: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "file_list_cache_hit_count",
            "number of ids returned from file list cache",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

// Node status metrics
pub static NODE_UP: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_up", "Node is up")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["version"],
    )
    .expect("Metric created")
});
pub static NODE_CPU_TOTAL: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_cpu_total", "Total CPU usage")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static NODE_CPU_USAGE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_cpu_usage", "CPU usage")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static NODE_MEMORY_TOTAL: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_memory_total", "Total memory usage")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static NODE_MEMORY_USAGE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_memory_usage", "Memory usage")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});
pub static NODE_TCP_CONNECTIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_tcp_connections", "TCP connections")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["state"],
    )
    .expect("Metric created")
});
pub static NODE_CONSISTENT_HASH: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new("node_consistent_hash", "Consistent hash")
            .namespace(NAMESPACE)
            .const_labels(create_const_labels()),
        &["type"],
    )
    .expect("Metric created")
});

// query disk cache metrics
pub static QUERY_DISK_CACHE_HIT_COUNT: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "query_disk_cache_hit_count",
            "query disk cache hit count".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "file_type"],
    )
    .expect("Metric created")
});
pub static QUERY_DISK_CACHE_MISS_COUNT: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "query_disk_cache_miss_count",
            "query disk cache miss count".to_owned() + HELP_SUFFIX,
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &["organization", "stream_type", "file_type"],
    )
    .expect("Metric created")
});

// file downloader metrics
pub static FILE_DOWNLOADER_NORMAL_QUEUE_SIZE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "file_downloader_normal_queue_size",
            "file downloader normal queue size",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static FILE_DOWNLOADER_PRIORITY_QUEUE_SIZE: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "file_downloader_priority_queue_size",
            "file downloader priority queue size",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

// File access time bucket histogram
pub static FILE_ACCESS_TIME: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "file_access_time",
            "Histogram showing query counts within time windows from 1h to 1week (1h, 2h, 3h, 6h, 12h, 24h, 48h, 96h, 168h)"
        )
        .namespace(NAMESPACE)
        .buckets(vec![1.0, 2.0, 3.0, 6.0, 12.0, 24.0, 48.0, 96.0, 168.0])
        .const_labels(create_const_labels()),
        &["stream_type"],
    )
    .expect("Metric created")
});

// Metrics for pipeline wal writer
pub static PIPELINE_WAL_WRITER_DESTINATIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "pipeline_wal_writer_destinations",
            "Total number of pipeline wal writer destinations",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static PIPELINE_WAL_WRITERS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "pipeline_wal_writers",
            "Total number of wal writer across all pipelines",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static PIPELINE_WAL_FILES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "pipeline_wal_files",
            "Total number of wal files across all pipelines",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static PIPELINE_WAL_INGESTION_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "pipeline_wal_ingestion_bytes",
            "Bytes ingested across all pipelines",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static PIPELINE_EXPORTED_BYTES: Lazy<IntCounterVec> = Lazy::new(|| {
    IntCounterVec::new(
        Opts::new(
            "pipeline_http_exported_bytes",
            "Bytes exported across all pipelines",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static QUERY_AGGREGATION_CACHE_ITEMS: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_aggregation_cache_items",
            "Total number of aggregation cache items",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

pub static QUERY_AGGREGATION_CACHE_BYTES: Lazy<IntGaugeVec> = Lazy::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "query_aggregation_cache_bytes",
            "Total number of aggregation cache bytes",
        )
        .namespace(NAMESPACE)
        .const_labels(create_const_labels()),
        &[],
    )
    .expect("Metric created")
});

fn register_metrics(registry: &Registry) {
    // http latency
    registry
        .register(Box::new(HTTP_INCOMING_REQUESTS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(HTTP_RESPONSE_TIME.clone()))
        .expect("Metric registered");

    // grpc latency
    registry
        .register(Box::new(GRPC_INCOMING_REQUESTS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(GRPC_RESPONSE_TIME.clone()))
        .expect("Metric registered");

    // ingester stats
    registry
        .register(Box::new(INGEST_RECORDS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_ERRORS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_WAL_USED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_WAL_WRITE_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_WAL_READ_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_MEMTABLE_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_MEMTABLE_ARROW_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_MEMTABLE_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_MEMTABLE_LOCK_TIME.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(INGEST_WAL_LOCK_TIME.clone()))
        .expect("Metric registered");

    // querier stats
    registry
        .register(Box::new(QUERY_MEMORY_CACHE_LIMIT_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_MEMORY_CACHE_USED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_MEMORY_CACHE_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_CACHE_LIMIT_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_CACHE_USED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_CACHE_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_RESULT_CACHE_USED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_METRICS_CACHE_USED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_PARQUET_CACHE_RATIO.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_PARQUET_CACHE_RATIO_NODE.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_METRICS_CACHE_RATIO.clone()))
        .expect("Metric registered");

    // query manager
    registry
        .register(Box::new(QUERY_RUNNING_NUMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_PENDING_NUMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_TIMEOUT_NUMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_CANCELED_NUMS.clone()))
        .expect("Metric registered");

    // compactor stats
    registry
        .register(Box::new(COMPACT_USED_TIME.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(COMPACT_MERGED_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(COMPACT_MERGED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(COMPACT_PENDING_JOBS.clone()))
        .expect("Metric registered");

    // storage stats
    registry
        .register(Box::new(STORAGE_ORIGINAL_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_COMPRESSED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_RECORDS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_WRITE_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_READ_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_TIME.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_READ_REQUESTS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(STORAGE_WRITE_REQUESTS.clone()))
        .expect("Metric registered");
    // metadata stats
    registry
        .register(Box::new(META_STORAGE_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_STORAGE_KEYS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_NODES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_ORGANIZATIONS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_STREAMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_USERS_TOTAL.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_USERS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_FUNCTIONS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_ALERTS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(META_NUM_DASHBOARDS.clone()))
        .expect("Metric registered");

    // db stats
    registry
        .register(Box::new(DB_QUERY_NUMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(DB_QUERY_TIME.clone()))
        .expect("Metric registered");

    // file list specific metrics
    registry
        .register(Box::new(FILE_LIST_ID_SELECT_COUNT.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(FILE_LIST_CACHE_HIT_COUNT.clone()))
        .expect("Metric registered");

    // node status metrics
    registry
        .register(Box::new(NODE_UP.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_CPU_TOTAL.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_CPU_USAGE.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_MEMORY_TOTAL.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_MEMORY_USAGE.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_TCP_CONNECTIONS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(NODE_CONSISTENT_HASH.clone()))
        .expect("Metric registered");

    // query disk cache metrics
    registry
        .register(Box::new(QUERY_DISK_CACHE_HIT_COUNT.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_DISK_CACHE_MISS_COUNT.clone()))
        .expect("Metric registered");
    // file downloader metrics
    registry
        .register(Box::new(FILE_DOWNLOADER_NORMAL_QUEUE_SIZE.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(FILE_DOWNLOADER_PRIORITY_QUEUE_SIZE.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(FILE_ACCESS_TIME.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(PIPELINE_WAL_WRITER_DESTINATIONS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(PIPELINE_WAL_WRITERS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(PIPELINE_WAL_FILES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(PIPELINE_WAL_INGESTION_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(PIPELINE_EXPORTED_BYTES.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_AGGREGATION_CACHE_ITEMS.clone()))
        .expect("Metric registered");
    registry
        .register(Box::new(QUERY_AGGREGATION_CACHE_BYTES.clone()))
        .expect("Metric registered");
}

pub fn create_const_labels() -> HashMap<String, String> {
    let cfg = crate::config::get_config();
    let mut labels = HashMap::new();
    labels.insert("cluster".to_string(), cfg.common.cluster_name.clone());
    labels.insert("instance".to_string(), cfg.common.instance_name.clone());
    labels.insert("role".to_string(), cfg.common.node_role.clone());
    labels
}

pub fn gather() -> String {
    let registry = prometheus::default_registry();
    let mut buffer = vec![];
    TextEncoder::new()
        .encode(&registry.gather(), &mut buffer)
        .unwrap();
    String::from_utf8(buffer).unwrap()
}

pub fn init() {
    register_metrics(prometheus::default_registry());
}
