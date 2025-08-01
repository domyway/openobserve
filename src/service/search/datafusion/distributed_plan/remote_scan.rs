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
    any::Any,
    collections::HashMap,
    ops::Sub,
    sync::Arc,
    task::{Context, Poll},
};

use arrow::record_batch::RecordBatch;
use arrow_flight::{
    FlightData, Ticket, flight_service_client::FlightServiceClient,
    utils::flight_data_to_arrow_batch,
};
use arrow_schema::{Schema, SchemaRef};
use config::{
    meta::{
        cluster::NodeInfo,
        search::{ScanStats, SearchEventType},
    },
    utils::{rand::generate_random_string, size::bytes_to_human_readable},
};
use datafusion::{
    common::{DataFusionError, Result, Statistics},
    execution::{RecordBatchStream, SendableRecordBatchStream, TaskContext},
    physical_expr::{EquivalenceProperties, Partitioning},
    physical_plan::{
        DisplayAs, DisplayFormatType, ExecutionPlan, PlanProperties,
        execution_plan::{Boundedness, EmissionType},
        stream::RecordBatchStreamAdapter,
    },
};
use datafusion_proto::bytes::physical_plan_to_bytes_with_extension_codec;
use futures::{Stream, StreamExt, TryStreamExt};
use opentelemetry::trace::{Span, TraceId, Tracer};
use parking_lot::Mutex;
use prost::Message;
use proto::cluster_rpc;
use rand::prelude::SliceRandom;
use tonic::{
    Streaming,
    codec::CompressionEncoding,
    metadata::{MetadataKey, MetadataValue},
};

use super::node::RemoteScanNode;
use crate::service::{
    grpc::get_cached_channel,
    search::{
        MetadataMap,
        datafusion::distributed_plan::codec::get_physical_extension_codec,
        inspector::{SearchInspectorFieldsBuilder, search_inspector_fields},
    },
};

/// Execution plan for empty relation with produce_one_row=false
#[derive(Debug)]
pub struct RemoteScanExec {
    input: Arc<dyn ExecutionPlan>,
    remote_scan_node: RemoteScanNode,
    partitions: usize,
    cache: PlanProperties,
    pub scan_stats: Arc<Mutex<ScanStats>>,
    pub partial_err: Arc<Mutex<String>>,
    pub enrich_mode_node_idx: usize,
}

impl RemoteScanExec {
    /// Create a new RemoteScanExec
    pub fn new(
        input: Arc<dyn ExecutionPlan>,
        mut remote_scan_node: RemoteScanNode,
    ) -> Result<Self> {
        let output_partitions = remote_scan_node.nodes.len();
        let cache = Self::compute_properties(Arc::clone(&input.schema()), output_partitions);

        // serialize the input plan and set it as the plan for the remote scan node
        let proto = get_physical_extension_codec();
        let physical_plan_bytes =
            physical_plan_to_bytes_with_extension_codec(input.clone(), &proto)?;
        remote_scan_node.set_plan(physical_plan_bytes.to_vec());

        // get the node ids for enrich mode
        let mut node_ids = remote_scan_node
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, n)| if n.is_querier() { Some(idx) } else { None })
            .collect::<Vec<_>>();
        // random shuffle the node ids
        node_ids.shuffle(&mut rand::thread_rng());
        let enrich_mode_node_idx = node_ids.pop().unwrap_or_default();

        Ok(RemoteScanExec {
            input,
            remote_scan_node,
            partitions: output_partitions,
            cache,
            scan_stats: Arc::new(Mutex::new(ScanStats::default())),
            partial_err: Arc::new(Mutex::new(String::new())),
            enrich_mode_node_idx,
        })
    }

    fn output_partitioning_helper(n_partitions: usize) -> Partitioning {
        Partitioning::UnknownPartitioning(n_partitions)
    }

    /// This function creates the cache object that stores the plan properties such as schema,
    /// equivalence properties, ordering, partitioning, etc.
    fn compute_properties(schema: SchemaRef, n_partitions: usize) -> PlanProperties {
        let eq_properties = EquivalenceProperties::new(schema);
        let output_partitioning = Self::output_partitioning_helper(n_partitions);
        PlanProperties::new(
            eq_properties,
            // Output Partitioning
            output_partitioning,
            // Execution Mode
            EmissionType::Incremental,
            Boundedness::Bounded,
        )
    }
}

impl DisplayAs for RemoteScanExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "RemoteScanExec: input_partitions=output_partitions={}",
            self.partitions,
        )
    }
}

impl ExecutionPlan for RemoteScanExec {
    fn name(&self) -> &'static str {
        "RemoteScanExec"
    }

    /// Return a reference to Any that can be used for downcasting
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.input.schema())
    }

    fn properties(&self) -> &PlanProperties {
        &self.cache
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            return Ok(self);
        }
        Ok(Arc::new(Self::new(
            children[0].clone(),
            self.remote_scan_node.clone(),
        )?))
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let fut = get_remote_batch(
            self.remote_scan_node.clone(),
            partition,
            partition == self.enrich_mode_node_idx,
            self.input.schema().clone(),
            self.scan_stats.clone(),
            self.partial_err.clone(),
        );
        let stream = futures::stream::once(fut).try_flatten();
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema().clone(),
            stream,
        )))
    }

    fn statistics(&self) -> Result<Statistics> {
        Ok(Statistics::new_unknown(&self.schema()))
    }
}

async fn get_remote_batch(
    remote_scan_node: RemoteScanNode,
    partition: usize,
    enrich_mode: bool,
    schema: SchemaRef,
    scan_stats: Arc<Mutex<ScanStats>>,
    partial_err: Arc<Mutex<String>>,
) -> Result<SendableRecordBatchStream> {
    let start = std::time::Instant::now();
    let cfg = config::get_config();
    let trace_id = remote_scan_node.query_identifier.trace_id.clone();
    let org_id = remote_scan_node.query_identifier.org_id.clone();
    let context = remote_scan_node.opentelemetry_context.clone();
    let node = remote_scan_node.nodes[partition].clone();
    let is_super = remote_scan_node.super_cluster_info.is_super_cluster;
    let is_querier = node.is_querier();
    let is_ingester = node.is_ingester();
    let search_type = remote_scan_node
        .super_cluster_info
        .search_event_type
        .as_ref()
        .and_then(|s| s.as_str().try_into().ok());

    // check timeout for ingester
    let mut timeout = remote_scan_node.search_infos.timeout;
    if matches!(search_type, Some(SearchEventType::UI)) && is_ingester {
        timeout = std::cmp::min(timeout, cfg.limit.query_ingester_timeout);
    }
    if timeout == 0 {
        timeout = cfg.limit.query_timeout;
    }

    // fast return for empty file list querier node
    if !is_super
        && is_querier
        && !is_ingester
        && !enrich_mode
        && remote_scan_node.is_file_list_empty(partition)
    {
        return Ok(get_empty_record_batch_stream(
            trace_id,
            schema,
            node.get_grpc_addr(),
            is_querier,
            partial_err,
            tonic::Status::new(tonic::Code::Ok, ""),
            start,
        ));
    }

    let mut request = remote_scan_node.get_flight_search_request(partition);
    request.set_job_id(generate_random_string(7));
    request.set_partition(partition);
    request.search_info.timeout = timeout as i64;
    request.query_identifier.enrich_mode = enrich_mode;

    log::info!(
        "[trace_id {}] flight->search: request node: {}, query_type: {}, is_super: {}, is_querier: {}, timeout: {}, files: {}",
        trace_id,
        &node.get_grpc_addr(),
        search_type.unwrap_or(SearchEventType::UI),
        is_super,
        is_querier,
        timeout,
        request.search_info.file_id_list.len(),
    );

    let request: cluster_rpc::FlightSearchRequest = request.into();
    let mut buf: Vec<u8> = Vec::new();
    request
        .encode(&mut buf)
        .map_err(|e| datafusion::common::DataFusionError::Internal(format!("{e:?}")))?;

    let mut request = tonic::Request::new(Ticket {
        ticket: buf.clone().into(),
    });

    let org_id: MetadataValue<_> = org_id
        .parse()
        .map_err(|_| DataFusionError::Internal("invalid org_id".to_string()))?;

    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut MetadataMap(request.metadata_mut()))
    });

    let org_header_key: MetadataKey<_> = cfg
        .grpc
        .org_header_key
        .parse()
        .map_err(|_| DataFusionError::Internal("invalid org_header_key".to_string()))?;
    let token: MetadataValue<_> = node
        .get_auth_token()
        .parse()
        .map_err(|_| DataFusionError::Internal("invalid token".to_string()))?;
    let channel = match get_cached_channel(&node.get_grpc_addr()).await {
        Ok(channel) => channel,
        Err(e) => {
            log::error!(
                "[trace_id {}] flight->search: node: {}, connect err: {:?}",
                trace_id.clone(),
                &node.get_grpc_addr(),
                e
            );
            return Ok(get_empty_record_batch_stream(
                trace_id,
                schema,
                node.get_grpc_addr(),
                is_querier,
                partial_err,
                e,
                start,
            ));
        }
    };

    let mut client =
        FlightServiceClient::with_interceptor(channel, move |mut req: tonic::Request<()>| {
            req.metadata_mut().insert("authorization", token.clone());
            req.metadata_mut()
                .insert(org_header_key.clone(), org_id.clone());
            req.set_timeout(std::time::Duration::from_secs(timeout));
            Ok(req)
        });
    client = client
        .send_compressed(CompressionEncoding::Gzip)
        .accept_compressed(CompressionEncoding::Gzip)
        .max_decoding_message_size(cfg.grpc.max_message_size * 1024 * 1024)
        .max_encoding_message_size(cfg.grpc.max_message_size * 1024 * 1024);

    log::info!(
        "[trace_id {}] flight->search: prepare to request node: {}, is_super: {}, is_querier: {}",
        trace_id,
        &node.get_grpc_addr(),
        is_super,
        is_querier,
    );

    let mut stream = match client.do_get(request).await {
        Ok(stream) => stream,
        Err(e) => {
            if e.code() == tonic::Code::Cancelled || e.code() == tonic::Code::DeadlineExceeded {
                return Ok(get_empty_record_batch_stream(
                    trace_id,
                    schema,
                    node.get_grpc_addr(),
                    is_querier,
                    partial_err,
                    e,
                    start,
                ));
            }
            log::error!(
                "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {:?}, took: {} ms",
                trace_id,
                node.get_grpc_addr(),
                is_super,
                is_querier,
                e,
                start.elapsed().as_millis(),
            );
            return Err(DataFusionError::Execution(e.to_string()));
        }
    }
    .into_inner();

    log::info!(
        "[trace_id {}] flight->search: prepare to response node: {}, is_super: {}, is_querier: {}",
        trace_id,
        &node.get_grpc_addr(),
        is_super,
        is_querier,
    );

    // the schema should be the first message returned, else client should error
    let flight_data = match stream.message().await {
        Ok(Some(flight_data)) => flight_data,
        Ok(None) => {
            log::error!(
                "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {}, took: {} ms",
                trace_id,
                node.get_grpc_addr(),
                is_super,
                is_querier,
                "No schema returned",
                start.elapsed().as_millis(),
            );
            return Err(DataFusionError::Execution("No schema returned".to_string()));
        }
        Err(e) => {
            if e.code() == tonic::Code::Cancelled || e.code() == tonic::Code::DeadlineExceeded {
                return Ok(get_empty_record_batch_stream(
                    trace_id,
                    schema,
                    node.get_grpc_addr(),
                    is_querier,
                    partial_err,
                    e,
                    start,
                ));
            }
            log::error!(
                "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {:?}, took: {} ms",
                trace_id,
                node.get_grpc_addr(),
                is_super,
                is_querier,
                e,
                start.elapsed().as_millis(),
            );
            return Err(DataFusionError::Execution(e.to_string()));
        }
    };

    // convert FlightData to a stream
    let schema = match Schema::try_from(&flight_data) {
        Ok(schema) => Arc::new(schema),
        Err(e) => {
            log::error!(
                "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {:?}, took: {} ms",
                trace_id,
                node.get_grpc_addr(),
                is_super,
                is_querier,
                e,
                start.elapsed().as_millis(),
            );
            return Err(DataFusionError::Execution("No schema returned".to_string()));
        }
    };

    let mut files = 0;
    let mut scan_size = 0;
    if let Some(stats) = schema.metadata().get("scan_stats") {
        let stats: ScanStats = serde_json::from_str(stats).unwrap_or_default();
        files = stats.files;
        scan_size = stats.original_size;
        scan_stats.lock().add(&stats);
    }

    Ok(Box::pin(FlightStream::new(
        trace_id,
        context,
        schema,
        stream,
        node,
        is_super,
        is_querier,
        files,
        scan_size,
        partial_err,
        start,
        timeout,
    )))
}

fn get_empty_record_batch_stream(
    trace_id: String,
    schema: SchemaRef,
    node_addr: String,
    is_querier: bool,
    partial_err: Arc<Mutex<String>>,
    e: tonic::Status,
    start: std::time::Instant,
) -> SendableRecordBatchStream {
    if e.code() != tonic::Code::Ok {
        log::info!(
            "[trace_id {}] flight->search: response node: {}, is_querier: {}, err: {:?}, took: {} ms",
            trace_id,
            node_addr,
            is_querier,
            e,
            start.elapsed().as_millis(),
        );
        process_partial_err(partial_err, e);
    }
    let stream = futures::stream::empty::<Result<RecordBatch>>();
    Box::pin(RecordBatchStreamAdapter::new(schema, stream))
}

fn process_partial_err(partial_err: Arc<Mutex<String>>, e: tonic::Status) {
    let mut guard = partial_err.lock();
    let partial_err = guard.clone();
    if partial_err.is_empty() {
        guard.push_str(e.to_string().as_str());
    } else {
        guard.push_str(format!(" \n {e}").as_str());
    }
}

struct FlightStream {
    trace_id: String,
    parent_cx: opentelemetry::Context,
    schema: SchemaRef,
    stream: Streaming<FlightData>,
    node: Arc<dyn NodeInfo>,
    is_super: bool,
    is_querier: bool,
    files: i64,
    scan_size: i64,
    num_rows: usize,
    partial_err: Arc<Mutex<String>>,
    start: std::time::Instant,
    timeout: u64,
}

impl FlightStream {
    #[allow(clippy::too_many_arguments)]
    fn new(
        trace_id: String,
        parent_cx: opentelemetry::Context,
        schema: SchemaRef,
        stream: Streaming<FlightData>,
        node: Arc<dyn NodeInfo>,
        is_super: bool,
        is_querier: bool,
        files: i64,
        scan_size: i64,
        partial_err: Arc<Mutex<String>>,
        start: std::time::Instant,
        timeout: u64,
    ) -> Self {
        Self {
            trace_id,
            parent_cx,
            schema,
            stream,
            node,
            is_super,
            is_querier,
            files,
            scan_size,
            num_rows: 0,
            partial_err,
            start,
            timeout,
        }
    }

    fn create_stream_end_span(
        &self,
    ) -> Result<opentelemetry::trace::SpanContext, infra::errors::Error> {
        let tracer = opentelemetry::global::tracer("FlightSenderStream");

        let now = std::time::SystemTime::now();
        let duration = self.start.elapsed();
        let start_time = now.sub(duration);

        let trace_id = self
            .trace_id
            .split('-')
            .next()
            .and_then(|id| TraceId::from_hex(id).ok());
        match trace_id {
            Some(trace_id) => {
                let mut span = tracer
                    .span_builder("service:search:flight::do_get_stream")
                    .with_trace_id(trace_id)
                    .with_start_time(start_time)
                    .with_attributes(vec![opentelemetry::KeyValue::new(
                        "duration",
                        duration.as_nanos() as i64,
                    )])
                    .start_with_context(&tracer, &self.parent_cx);

                let span_context = span.span_context().clone();
                let search_role = if self.is_super {
                    "leader".to_string()
                } else {
                    "follower".to_string()
                };
                let event = search_inspector_fields(
                    format!(
                        "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, files: {}, scan_size: {} mb, num_rows: {}, took: {} ms",
                        self.trace_id,
                        self.node.get_grpc_addr(),
                        self.is_super,
                        self.is_querier,
                        self.files,
                        self.scan_size / 1024 / 1024,
                        self.num_rows,
                        self.start.elapsed().as_millis(),
                    ),
                    SearchInspectorFieldsBuilder::new()
                        .node_name(self.node.get_name())
                        .region(self.node.get_region())
                        .cluster(self.node.get_cluster())
                        .component("remote scan streaming".to_string())
                        .search_role(search_role)
                        .duration(self.start.elapsed().as_millis() as usize)
                        .desc(format!(
                            "remote scan search files: {}, scan_size: {}, num_rows: {}",
                            self.files,
                            bytes_to_human_readable(self.scan_size as f64),
                            self.num_rows
                        ))
                        .build(),
                );

                span.add_event_with_timestamp(event, now, vec![]);
                span.end_with_timestamp(now);
                Ok(span_context)
            }
            None => Err(infra::errors::Error::Message(format!(
                "Invalid trace id: {}",
                self.trace_id
            ))),
        }
    }
}

impl Stream for FlightStream {
    type Item = Result<RecordBatch>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.start.elapsed().as_secs() > self.timeout {
            let e = tonic::Status::new(tonic::Code::DeadlineExceeded, "timeout");
            log::error!(
                "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {:?}, took: {} ms",
                self.trace_id,
                self.node.get_grpc_addr(),
                self.is_super,
                self.is_querier,
                e,
                self.start.elapsed().as_millis(),
            );
            process_partial_err(self.partial_err.clone(), e);
            return Poll::Ready(None);
        }

        let dictionaries_by_field = HashMap::new();
        match self.stream.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok(flight_data))) => {
                let record_batch = flight_data_to_arrow_batch(
                    &flight_data,
                    self.schema.clone(),
                    &dictionaries_by_field,
                )?;
                self.num_rows += record_batch.num_rows();
                Poll::Ready(Some(Ok(record_batch)))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Err(e))) => {
                log::error!(
                    "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, err: {:?}, took: {} ms",
                    self.trace_id,
                    self.node.get_grpc_addr(),
                    self.is_super,
                    self.is_querier,
                    e,
                    self.start.elapsed().as_millis(),
                );
                process_partial_err(self.partial_err.clone(), e);
                Poll::Ready(None)
            }
        }
    }
}

impl Drop for FlightStream {
    fn drop(&mut self) {
        let cfg = config::get_config();
        if (cfg.common.tracing_enabled || cfg.common.tracing_search_enabled)
            && let Err(e) = self.create_stream_end_span()
        {
            log::error!("error creating stream span: {e}");
        }
        log::info!(
            "[trace_id {}] flight->search: response node: {}, is_super: {}, is_querier: {}, files: {}, scan_size: {} mb, num_rows: {}, took: {} ms",
            self.trace_id,
            self.node.get_grpc_addr(),
            self.is_super,
            self.is_querier,
            self.files,
            self.scan_size / 1024 / 1024,
            self.num_rows,
            self.start.elapsed().as_millis()
        );
    }
}

impl RecordBatchStream for FlightStream {
    /// Get the schema
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}
