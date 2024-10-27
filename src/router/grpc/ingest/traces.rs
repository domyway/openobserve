// Copyright 2024 OpenObserve Inc.
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

use std::collections::HashSet;
use opentelemetry::trace::TraceId;
use config::meta::cluster::get_internal_grpc_token;
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_client::TraceServiceClient, trace_service_server::TraceService,
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use tonic::{codec::CompressionEncoding, metadata::MetadataValue, Request, Response, Status};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::service::{grpc::get_ingester_channel, search::MetadataMap};

#[derive(Default)]
pub struct TraceServer;

#[tonic::async_trait]
impl TraceService for TraceServer {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let start = std::time::Instant::now();
        let cfg = config::get_config();
        let (metadata, extensions, message) = request.into_parts();
        // TODO: need remove for debug
        let mut trace_ids = HashSet::new();
        let res_spans = &message.resource_spans;
        for res_span in res_spans {
            let inst_resources = &res_span.scope_spans;
            for inst_span in inst_resources {
                let spans = &inst_span.spans;
                for span in spans {
                    let trace_id: String =
                        TraceId::from_bytes(span.trace_id.clone().try_into().unwrap()).to_string();
                    if !trace_ids.contains(&trace_id) {
                        trace_ids.insert(trace_id.clone());
                        log::info!("[ROUTER] found new trace_id: {}", trace_id);
                    }
                }
            }
        }
        // basic validation
        if !metadata.contains_key(&cfg.grpc.org_header_key) {
            return Err(Status::invalid_argument(format!(
                "Please specify organization id with header key '{}' ",
                &cfg.grpc.org_header_key
            )));
        }

        // call ingester
        let mut request = Request::from_parts(metadata, extensions, message);
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.inject_context(
                &tracing::Span::current().context(),
                &mut MetadataMap(request.metadata_mut()),
            )
        });

        let token: MetadataValue<_> = get_internal_grpc_token()
            .parse()
            .map_err(|_| Status::internal("invalid token".to_string()))?;
        let (addr, channel) = get_ingester_channel().await?;
        let client = TraceServiceClient::with_interceptor(channel, move |mut req: Request<()>| {
            req.metadata_mut().insert("authorization", token.clone());
            Ok(req)
        });
        match client
            .send_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Gzip)
            .max_decoding_message_size(cfg.grpc.max_message_size * 1024 * 1024)
            .max_encoding_message_size(cfg.grpc.max_message_size * 1024 * 1024)
            .export(request)
            .await
        {
            Ok(res) => {
                if res.get_ref().partial_success.is_some() {
                    log::error!(
                        "[Router:TRACES] export partial_success node: {addr}, response: {:?}",
                        res.get_ref()
                    );
                }
                Ok(res)
            }
            Err(e) => {
                let time = start.elapsed().as_millis() as usize;
                log::error!("[Router:TRACES] export node: {addr}, status: {e}, took: {time} ms");
                Err(e)
            }
        }
    }
}
