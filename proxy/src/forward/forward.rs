#![deny(warnings)]
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Instant;
use std::sync::Arc;

use log::debug;
use protobuf::Message;
use snap;
use tokio::task::JoinError;
use warp::http::StatusCode;

use crate::metrics;
use crate::proto;
use metrics::metrics::process_time_serie;
use prometheus::{Counter, CounterVec, Histogram};
use proto::prometheus::{MetricMetadata, WriteRequest};

pub enum ForwardingStatistics {
    NumSeries = 0,
    NumMetadata = 1,
    NumLabels = 2,
    TenantsDetected = 3,
    TotalRequests = 4,
    NumFailures = 5,
    ProcessingTime = 6,
}

// unpacks Snappy payload
// parses Prometheus protobuf structure
// sends proxied data to upstream
pub async fn process_proxy_payload(
    _client: reqwest::Client,
    _tenant_labels: Vec<String>,
    _allow_listed_tenants: Arc<&Vec<String>>,
    _does_allow_list: bool,
    _replicate_to: Vec<String>,
    _ingester_stream_url: String,
    _parallel_request_per_load: u16,
    _internal_stats: &HashMap<u8, Counter>,
    _internal_stats_vec: &HashMap<u8, CounterVec>,
    _internal_stats_histograms: &HashMap<u8, Histogram>,
    _bytes: warp::hyper::body::Bytes,
) -> Result<impl warp::Reply, Infallible> {
    return {
        // deserialize prom write request

        // FIXME: factor out deserialization code into separate fn

        let mut uncompressed_pb_message =
            match snap::raw::Decoder::new().decompress_vec(_bytes.to_vec().as_ref()) {
                Ok(v) => v,
                Err(e) => {
                    // return early to indicate error
                    return Ok(warp::reply::with_status(
                        warp::reply::html(e.to_string()),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            };

        debug!(
            "::: request length decompressed : {}b",
            uncompressed_pb_message.len().to_string()
        );
        let write_request = match WriteRequest::parse_from_bytes(&mut uncompressed_pb_message) {
            Ok(req) => req,
            // invalid protobuf in request
            Err(e) => {
                return Ok(
                    // return early to indicate error
                    warp::reply::with_status(
                        warp::reply::html(e.to_string()),
                        StatusCode::BAD_REQUEST,
                    ),
                )
            }
        };

        let in_ms = Instant::now();

        // container for generated requests
        let mut tenant_data = HashMap::<String, WriteRequest>::new();

        // statistical values

        let num_metadata: &Counter = _internal_stats
            .get(&(ForwardingStatistics::NumMetadata as u8))
            .unwrap();
        let num_labels: &Counter = _internal_stats
            .get(&(ForwardingStatistics::NumLabels as u8))
            .unwrap();
        let tenants_detected: &Counter = _internal_stats
            .get(&(ForwardingStatistics::TenantsDetected as u8))
            .unwrap();
        let num_failures: &Counter = _internal_stats
            .get(&(ForwardingStatistics::NumFailures as u8))
            .unwrap();

        let num_series: &CounterVec = _internal_stats_vec
            .get(&(ForwardingStatistics::NumSeries as u8))
            .unwrap();
        let total_requests: &CounterVec = _internal_stats_vec
            .get(&(ForwardingStatistics::TotalRequests as u8))
            .unwrap();

        let histogram: &Histogram = _internal_stats_histograms
            .get(&(ForwardingStatistics::ProcessingTime as u8))
            .unwrap();

        // aggregate metrics by tenant
        for time_series in write_request.timeseries.into_iter() {
            let (tenants, labels) = process_time_serie(
                &time_series,
                &_tenant_labels,
                &_allow_listed_tenants,
                _does_allow_list,
                &_replicate_to,
                &mut tenant_data,
            );
            tenants_detected.inc_by(tenants as f64);
            num_labels.inc_by(labels as f64);
        }

        // append metadata for each request
        for metadata in &write_request.metadata {
            for req in tenant_data.values_mut() {
                req.metadata.push(MetricMetadata::from(metadata.clone()));
            }
            num_metadata.inc()
        }

        for tenant_id in tenant_data.keys() {
            total_requests
                .with_label_values(&[tenant_id.as_str()])
                .inc();
            num_series
                .with_label_values(&[tenant_id.as_str()])
                .inc_by(tenant_data.get(tenant_id).unwrap().timeseries.len() as f64);
        }

        let responses = futures::stream::iter(tenant_data.into_iter())
            .map(
                |(_tenant_id, tenant_request): (String, WriteRequest)| {
                    // save necessary context on a stack
                    let tenant_id_clone = _tenant_id.clone();
                    let r_client = _client.clone();
                    let url = _ingester_stream_url.clone();

                    // serialize request
                    let tenant_request_serialized = tenant_request.write_to_bytes().unwrap();
                    let tenant_request_compressed = snap::raw::Encoder::new()
                        .compress_vec(&tenant_request_serialized)
                        .unwrap();

                    // spawn origin request in async manner
                    tokio::spawn(async move {
                        r_client
                            .post(&url)
                            .body(tenant_request_compressed)
                            .header("X-Scope-OrgID", tenant_id_clone)
                            .send()
                            .await
                    })
                }, // keep limitation for number of parallel requests
            )
            .buffer_unordered(_parallel_request_per_load.into());

        let num_of_failures = responses
            .then(process_upstream_response)
            .fold(0, accumulate_errors)
            .await;

        debug!("number of errors while processing: {}", num_of_failures);
        num_failures.inc_by(num_of_failures as f64);
        histogram.observe(in_ms.elapsed().as_millis() as f64);

        // determine processing status
        let expose_as = if num_of_failures == 0 {
            StatusCode::OK
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };

        Ok(warp::reply::with_status(
            warp::reply::html(num_of_failures.to_string()),
            expose_as,
        ))
    };
}

// processes origin response, convert upstream status to integer
async fn process_upstream_response(
    resp: std::result::Result<std::result::Result<reqwest::Response, reqwest::Error>, JoinError>,
) -> Result<u16, u16> {
    let status = match resp {
        Ok(res) => {
            if res.is_err() {
                debug!("request failed: {}", res.unwrap_err());
                Err(400 as u16)
            } else {
                let processing_response = res.unwrap();
                let status = processing_response.status();
                Ok(status.as_u16())
            }
        }
        Err(e) => {
            debug!("got join error while processing: {}", e);
            Err(500 as u16)
        }
    };
    status
}

// folds origin results
// var:acc accumulates origin error count
async fn accumulate_errors(acc: u16, result: Result<u16, u16>) -> u16 {
    if result.is_err() {
        acc + 1
    } else {
        let code_string = result.unwrap().to_string();
        if code_string.starts_with("2") || code_string.starts_with("4") {
            // 2xx response
            // 4xx ignored for dropped samples
            acc
        } else {
            acc + 1
        }
    }
}
