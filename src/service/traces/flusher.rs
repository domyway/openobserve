use std::{
    io::{Error, ErrorKind},
    time::Duration,
};

use config::meta::stream::StreamType;
use crossbeam_channel::{Receiver as CrossbeamReceiver, Sender as CrossbeamSender};
use hashbrown::HashMap;
use infra::errors::BufferWriteError;
use log::{info, warn};
use once_cell::sync::Lazy;
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use parking_lot::Mutex;
use tokio::{
    runtime::Runtime,
    select,
    sync::{mpsc, oneshot, watch},
    time::MissedTickBehavior,
};

use crate::{
    common::meta::stream::SchemaRecords,
    job::metrics::TraceMetricsItem,
    service::{
        ingestion::TriggerAlertData,
        metadata::MetadataItem,
        traces::{flusher, wal::wal_handle_trace_request},
    },
};

const BUFFER_FLUSH_INTERVAL: Duration = Duration::from_millis(10);
// The maximum number of buffered writes that can be queued up before backpressure is applied
const BUFFER_CHANNEL_LIMIT: usize = 10_000;

pub enum ExportRequest {
    TraceEntry(ExportRequestInnerEntry),
}
#[derive(Debug)]
pub struct BufferedWrite {
    pub request: tonic::Request<ExportTraceServiceRequest>,
    pub response_tx: oneshot::Sender<BufferedWriteResult>,
}
#[derive(Debug)]
pub enum BufferedWriteResult {
    Success(ExportTraceServiceResponse),
    Error(BufferWriteError),
}

type RequestOps = HashMap<String, tonic::Request<ExportTraceServiceRequest>>;
pub(crate) type WalTraceResponse = ExportTraceServiceResponse;
#[derive(Clone)]
pub struct ExportRequestInnerEntry {
    pub(crate) org_id: String,
    pub(crate) is_grpc: bool,
    pub(crate) stream_name: String,
    pub(crate) data_buf: std::collections::HashMap<String, SchemaRecords>,
    pub(crate) distinct_values: Vec<MetadataItem>,
    pub(crate) trace_index_values: Vec<MetadataItem>,
    pub(crate) trigger: Option<TriggerAlertData>,
    pub(crate) span_metrics: Vec<TraceMetricsItem>,
    pub(crate) partial_success:
        opentelemetry_proto::tonic::collector::trace::v1::ExportTracePartialSuccess,
}

pub struct WalTraceServiceResponse {
    response: Result<WalTraceResponse, BufferWriteError>,
    request: ExportRequestInnerEntry,
    trace_id: String,
}
type NotifyResult = HashMap<String, WalTraceServiceResponse>;
type IoFlushNotifyResult = Result<NotifyResult, BufferWriteError>;
pub static SYNC_RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().unwrap());

#[derive(Debug)]
pub struct WriteBufferFlusher {
    join_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    wal_io_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub shutdown_tx: Option<watch::Sender<()>>,
    buffer_tx: mpsc::Sender<BufferedWrite>,
}

impl Default for WriteBufferFlusher {
    fn default() -> Self {
        Self::new()
    }
}

impl WriteBufferFlusher {
    pub fn new() -> Self {
        let (trace_shutdown_tx, trace_shutdown_rx) = watch::channel(());
        let (buffer_tx, buffer_rx) = mpsc::channel::<BufferedWrite>(flusher::BUFFER_CHANNEL_LIMIT);
        let (io_flush_tx, io_flush_rx) = crossbeam_channel::bounded(1);
        let (io_flush_notify_tx, io_flush_notify_rx) = crossbeam_channel::bounded(1);

        let flusher = Self {
            join_handle: Default::default(),
            wal_io_handle: Default::default(),
            shutdown_tx: Some(trace_shutdown_tx),
            buffer_tx,
        };

        *flusher.wal_io_handle.lock() = Some(
            std::thread::Builder::new()
                .name("write buffer io flusher".to_string())
                .spawn(move || {
                    run_trace_io_flush(io_flush_rx, io_flush_notify_tx);
                })
                .expect("failed to spawn write buffer io flusher thread"),
        );

        *flusher.join_handle.lock() = Some(tokio::task::spawn(async move {
            run_trace_op_buffer(
                buffer_rx,
                io_flush_tx,
                io_flush_notify_rx,
                trace_shutdown_rx,
            )
            .await
        }));

        flusher
    }

    pub async fn write(
        &self,
        request: tonic::Request<ExportTraceServiceRequest>,
    ) -> Result<BufferedWriteResult, Error> {
        let (response_tx, response_rx) = oneshot::channel();
        let trace_id = get_request_trace_id(&request);
        match self
            .buffer_tx
            .send(BufferedWrite {
                request,
                response_tx,
            })
            .await
        {
            Ok(_) => {
                let resp = response_rx.await.expect("wal op buffer thread is dead");
                log::error!("{trace_id} flusher response_rx resp end");
                match resp {
                    BufferedWriteResult::Success(_) => Ok(resp),
                    BufferedWriteResult::Error(e) => {
                        log::error!("flusher inside write resp error {e}");
                        Err(Error::new(ErrorKind::Other, e))
                    }
                }
            }
            Err(e) => {
                log::error!("{trace_id} flusher inside write error {e}");
                Err(Error::new(ErrorKind::Other, e))
            }
        }
    }

    pub fn get_shutdown_tx(&self) -> Option<watch::Sender<()>> {
        self.shutdown_tx.clone()
    }
}

pub fn run_trace_io_flush(
    io_flush_rx: CrossbeamReceiver<RequestOps>,
    io_flush_notify_tx: CrossbeamSender<IoFlushNotifyResult>,
) {
    loop {
        let request = match io_flush_rx.recv() {
            Ok(request) => request,
            Err(e) => {
                // the buffer channel has closed, it's shutdown
                info!("stopping wal io thread: {e}");
                return;
            }
        };

        let mut res: NotifyResult = HashMap::new();
        // write the ops to the segment files, or return on first error
        let mut trace_id_list = "".to_string();
        for (session_id, request) in request {
            let mut sr = WalTraceServiceResponse {
                response: Ok(Default::default()),
                request: ExportRequestInnerEntry {
                    org_id: "".to_string(),
                    is_grpc: false,
                    stream_name: "".to_string(),
                    data_buf: Default::default(),
                    distinct_values: vec![],
                    trace_index_values: vec![],
                    trigger: None,
                    span_metrics: vec![],
                    partial_success: Default::default(),
                },
                trace_id: "".to_string(),
            };

            log::info!("[{session_id}] run_trace_io_flush start");
            sr.trace_id = session_id.to_string();
            sr.response = match SYNC_RT.block_on(async {
                let cfg = config::get_config();
                let metadata = request.metadata().clone();
                let org_id = metadata.get(&cfg.grpc.org_header_key);
                if org_id.is_none() {
                    let msg = format!(
                        "Please specify organization id with header key '{}' ",
                        &cfg.grpc.org_header_key
                    );
                    return Err(BufferWriteError::ForbiddenOrganization(msg));
                }
                let stream_name = metadata.get(&cfg.grpc.stream_header_key);
                let mut in_stream_name: Option<&str> = None;
                if let Some(stream_name) = stream_name {
                    in_stream_name = Some(stream_name.to_str().unwrap());
                };

                match wal_handle_trace_request(
                    org_id.unwrap().to_str().unwrap(),
                    request.into_inner(),
                    in_stream_name,
                )
                .await
                {
                    Ok(res) => {
                        sr.request = res.clone();
                        crate::service::traces::wal::wal_write_traces(
                            res.org_id.as_str(),
                            res.is_grpc,
                            res.stream_name.as_str(),
                            res.data_buf,
                            res.distinct_values,
                            res.trace_index_values,
                            res.trigger,
                            res.span_metrics,
                            res.partial_success,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                }
            }) {
                Ok(res) => Ok(res),
                Err(e) => Err(e),
            };
            log::info!("[{session_id}] run_trace_io_flush end");
            // Httpresponse may be partial_success, it must handle every response result
            res.insert(session_id.to_string(), sr);
            trace_id_list = format!("{trace_id_list}|{}", session_id);
        }

        io_flush_notify_tx
            .send(Ok(res))
            .expect("buffer flusher is dead");
        log::info!("[{trace_id_list}] io_flush_notify_tx end");
    }
}

pub async fn run_trace_op_buffer(
    mut buffer_rx: mpsc::Receiver<BufferedWrite>,
    io_flush_tx: CrossbeamSender<RequestOps>,
    io_flush_notify_rx: CrossbeamReceiver<IoFlushNotifyResult>,
    mut shutdown: tokio::sync::watch::Receiver<()>,
) {
    let mut ops = RequestOps::new();
    let mut notifies = Vec::new();
    let mut interval = tokio::time::interval(BUFFER_FLUSH_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        // select on either buffering an op, ticking the flush interval, or shutting down
        select! {
            Some(buffered_write) = buffer_rx.recv() => {
                let session_id = get_request_trace_id(&buffered_write.request);
                let _ = ops.insert(session_id.clone(), buffered_write.request);
                notifies.push((session_id, buffered_write.response_tx));
            },
            _ = interval.tick() => {
                if ops.is_empty() {
                    continue;
                }
                // send ops into IO flush channel and wait for response
                if let Err(e) = io_flush_tx.send(ops) {
                    log::error!("io_flush_tx send e : {}, len: {}", e, io_flush_tx.len());
                }

                match io_flush_notify_rx.recv().expect("wal io thread is dead") {
                    Ok(mut resp) => {
                        for (_, walresp) in &resp {
                            let in_trace_id = walresp.trace_id.as_str();
                            log::info!("[{in_trace_id}] run_trace_op_buffer start");
                            let writer = ingester::get_writer(walresp.request.org_id.as_str(), &StreamType::Traces.to_string(), walresp.request.stream_name.as_str()).await;
                            let _ = crate::service::ingestion::write_memtable(&writer, walresp.request.stream_name.as_str(), walresp.request.data_buf.clone()).await;
                            log::info!("[{in_trace_id}] run_trace_op_buffer start");
                        }
                        // notify the watchers of the write response
                        for (sid, response_tx) in notifies {
                            match resp.remove(&sid) {
                                Some(r) => {
                                    let bwr = match r.response {
                                        Ok(ets) => {
                                            BufferedWriteResult::Success(ets.clone())
                                        }
                                        Err(e) => {
                                            log::error!("{sid} io_flush_notify_rx resp error : {e}");
                                            BufferedWriteResult::Error(e)
                                        }
                                    };
                                    log::info!("[{sid}] BufferedWriteResult send start");
                                    let _ = response_tx.send(bwr);
                                    log::info!("[{sid}] BufferedWriteResult send end");
                                }
                                None => { warn!("[{sid}] ingest not found") }
                            }

                        }
                    },
                    Err(_) => unimplemented!(),
                };

                // reset the buffers
                ops = RequestOps::new();
                notifies = Vec::new();
                log::info!("reset the buffers");
            },
            _ = shutdown.changed() => {
                // shutdown has been requested
                info!("stopping wal op buffer thread");
                return;
            }
        }
    }
}

fn get_request_trace_id(request: &tonic::Request<ExportTraceServiceRequest>) -> String {
    let t = config::ider::uuid()
        .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        .unwrap();
    request
        .metadata()
        .get("in_trace_id")
        .unwrap_or(&t)
        .to_str()
        .unwrap_or("")
        .to_string()
}
