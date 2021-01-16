#![deny(warnings)]

use std::convert::Infallible;
use std::net::Ipv4Addr;

use argh::FromArgs;
use env_logger;
use log::error;
use reqwest::header::HeaderValue;
use warp::Filter;
use warp::log as http_log;

mod forward;
mod metrics;
mod proto;

use forward::forward::process_proxy_payload;
use std::process::exit;


#[derive(FromArgs)]
/// Open Metrics multi tenancy Proxy
struct OpenMetricsProxyArgs {

    /// port for serving http (default 19093)
    #[argh(option, default = "default_port()")]
    port: u16,

    /// port for serving http (default 127.0.0.1)
    #[argh(option, default = "String::from(\"127.0.0.1\")")]
    interface: String,

    /// max content length allowed to be posted
    #[argh(option, default="default_content_length_limit()")]
    content_length_limit: u64,

    /// comma-separated list of metrics which identify tenant
    #[argh(option, default="String::from(\"tenant_id\")")]
    tenant_label_list: String,

    /// comma-separated list of tenants id to replicate whole stream
    #[argh(option, default="String::from(\"0\")")]
    default_tenant_list: String,

    /// upstream url of cortex ingester deployment
    #[argh(option, default="String::from(\"http://127.0.0.1:5000\")")]
    ingester_upstream_url: String,

    /// maximum number of requests per single payload to invoke in parallel
    #[argh(option, default="default_parallel_requests_per_load()")]
    max_parallel_request_per_load: u16

}

// port
fn default_port() -> u16 {
    19093
}

// requests per load
fn default_parallel_requests_per_load() -> u16 {
    64
}

// content length limit
fn default_content_length_limit() -> u64 {
    100 * 1024 * 1024
}


#[tokio::main(max_threads = 1024)]
async fn main() {

    env_logger::init();
    let http_log_wrapper = http_log("Open-Metrics-multi-tenancy-Proxy");

    let args: OpenMetricsProxyArgs = argh::from_env();

    let tenant_label_list = args.tenant_label_list.clone().to_owned();
    let replicate_to_list = args.default_tenant_list.clone().to_owned();
    let ingester_stream_url = args.ingester_upstream_url.clone().to_owned();
    let _parallel_request_per_load = args.max_parallel_request_per_load.clone().to_owned();
    let interface = args.interface.clone().to_owned();

    let tenant_labels = tenant_label_list.split(",").map(|s| s.to_string()).collect();
    let replicate_to = replicate_to_list.split(",").map(|s| s.to_string()).collect();

    let mut headers = reqwest::header::HeaderMap::new();

    // remote write protocol version header
    headers.insert("X-Prometheus-Remote-Write-Version", HeaderValue::from_str("0.1.0").unwrap());
    headers.insert("User-Agent", HeaderValue::from_str("OM_mt_P").unwrap());

    // shared client instance
    let client = reqwest::ClientBuilder::new()
        .default_headers(headers)
        .http1_title_case_headers()
        .build().unwrap();

    fn with_parameter_vec(param_vec: Vec<String>) -> impl Filter<Extract = (Vec<String>,), Error = Infallible> + Clone {
        warp::any().map(move || param_vec.clone())
    };

    fn with_ingester_url(ingester_url: String) -> impl Filter<Extract = (String,), Error = Infallible> + Clone {
        warp::any().map(move || ingester_url.clone())
    }

    // match any post request and perform proxying
    let proxy = warp::any().and(warp::post())
        .and(warp::body::content_length_limit(args.content_length_limit))
        .map(move || client.clone())
        .and(with_parameter_vec(tenant_labels))
        .and(with_parameter_vec(replicate_to))
        .and(with_ingester_url(ingester_stream_url))
        .and(warp::body::bytes()).and_then(
        move |_client,_tenant_labels,_replicate_to,_ingester_stream_url,_bytes| async move {
            process_proxy_payload(
                _client,
                _tenant_labels,
                _replicate_to,
                _ingester_stream_url,
                _parallel_request_per_load,
                _bytes
            ).await
                .and_then(|response| Ok(response))
                .map_err(|e| {error!("Internal Error: {}", e); warp::reject::reject()})
    }).with(http_log_wrapper);

    // match any get request and return status
    let health = warp::any().and(warp::get())
        .map(|| "Up\n");

    let listen_addr = interface.parse::<Ipv4Addr>();

    let exit_code = match listen_addr {
        Ok(ip) => {
            warp::serve(health.or(proxy)).run((ip, args.port)).await;
            0
        },
        Err(e) => {
            error!("Invalid IP address: {}, err={}", interface, e);
            2
        }
    };

    exit(exit_code);
}
