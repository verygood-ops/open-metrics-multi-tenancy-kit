#![deny(warnings)]
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;

use log::debug;
use protobuf::Message;
use snap;
use tokio::task::JoinError;
use warp::Buf;
use warp::http::StatusCode;

use crate::proto;
use crate::metrics;
use metrics::metrics::process_time_serie;
use proto::prometheus::{MetricMetadata,WriteRequest};


// unpacks Snappy payload
// parses Prometheus protobuf structure
// sends proxied data to upstream
pub async fn process_proxy_payload(
    _client: reqwest::Client,
    _tenant_labels: Vec<String>,
    _replicate_to: Vec<String>,
    _ingester_stream_url: String,
    _parallel_request_per_load: u16,
    _bytes: warp::hyper::body::Bytes
) -> Result<impl warp::Reply, Infallible> {

    return {

        // deserialize prom write request

        // FIXME: factor out deserialization code into separate fn

        let mut uncompressed_pb_message = match snap::raw::Decoder::new()
            .decompress_vec(_bytes.bytes()) {
            Ok(v) => v,
            Err(e) => {
                // return early to indicate error
                return Ok(
                    warp::reply::with_status(
                        warp::reply::html(e.to_string()),
                        StatusCode::BAD_REQUEST)
                )
            }
        };

        debug!("::: request length decompressed : {}b", uncompressed_pb_message.len().to_string());
        let write_request = match WriteRequest::parse_from_bytes(&mut uncompressed_pb_message) {
            Ok(req) => req,
            // invalid protobuf in request
            Err(e) => return Ok(
                // return early to indicate error
                warp::reply::with_status(
                    warp::reply::html(e.to_string()),
                    StatusCode::BAD_REQUEST))
        };

        // container for generated requests
        let mut tenant_data = HashMap::<String,WriteRequest>::new();

        // statistical values
        // FIXME: expose via Prometheus
        let mut num_series: u16 = 0;
        let mut num_metadata: u16 = 0;
        let mut num_labels: u64 = 0;
        let mut tenants_detected: u16 = 0;
        let mut total_requests: u16 = 0;

        // aggregate metrics by tenant
        for time_series in write_request.timeseries.into_iter() {
            let (tenants, labels) = process_time_serie(
                &time_series,
                &_tenant_labels,
                &_replicate_to,
                &mut tenant_data
            );
            tenants_detected += tenants;
            num_labels += labels as u64;
            num_series += 1;
        };

        // append metadata for each request
        for metadata in &write_request.metadata {
            for req in tenant_data.values_mut() {
                req.metadata.push(MetricMetadata::from(metadata.clone()));
            };
            num_metadata += 1;
        };

        for _ in tenant_data.values_mut() {
            total_requests += 1;
        };

        // log debug metadata
        debug!("::: request time series : {}", num_series.to_string());
        debug!("::: request metadata : {}", num_metadata.to_string());
        debug!("::: request labels : {}", num_labels.to_string());
        debug!("::: detected tenants from labels : {}", tenants_detected.to_string());
        debug!("::: forwarded requests: {}", total_requests.to_string());
        debug!("::: current thread is: {}", std::thread::current().name().unwrap());

        let responses = futures::stream::iter(tenant_data.into_iter() ).map(
            | (_tenant_id, tenant_request): (String, WriteRequest) | {
                // save necessary context on a stack
                let tenant_id_clone = _tenant_id.clone();
                let r_client = _client.clone();
                let url = _ingester_stream_url.clone();

                // serialize request
                let tenant_request_serialized = tenant_request.write_to_bytes().unwrap();
                let tenant_request_compressed = snap::raw::Encoder::new()
                    .compress_vec(&tenant_request_serialized).unwrap();

                // spawn origin request in async manner
                tokio::spawn(async move {
                    r_client.post(&url)
                        .body(tenant_request_compressed)
                        .header("X-Scope-OrgID", tenant_id_clone)
                        .send().await
                })
            }
            // keep limitation for number of parallel requests
        ).buffer_unordered(_parallel_request_per_load.into());

        let num_of_failures = responses.then(process_upstream_response)
            .fold(0,accumulate_errors).await;

        debug!("number of errors while processing: {}", num_of_failures);

        // determine processing status
        let expose_as = if num_of_failures == 0
            {StatusCode::OK}
        else
            {StatusCode::INTERNAL_SERVER_ERROR};

        Ok(
            warp::reply::with_status(
                warp::reply::html(num_of_failures.to_string()),
                expose_as
            )
        )
    };
}

// processes origin response, convert upstream status to integer
async fn process_upstream_response(
    resp: std::result::Result<std::result::Result<reqwest::Response, reqwest::Error>, JoinError>
)
    -> Result<u16,u16> {
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
        },
        Err(e) => {
            debug!("got join error while processing: {}", e);
            Err(500 as u16)
        }
    };
    status
}


// folds origin results
// var:acc accumulates origin error count
async fn accumulate_errors(acc: u16, result: Result<u16,u16>) -> u16   {
    if result.is_err()
    {
        acc + 1
    }
    else {
        if result.unwrap().to_string().starts_with("2") {
            // 2xx response
            acc
        } else {
            acc + 1
        }
    }
}
