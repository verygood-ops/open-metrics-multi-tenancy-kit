#![deny(warnings)]
extern crate serde_derive;

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::process::exit;
use std::sync::Arc;

use argh::FromArgs;
use env_logger;
use log::error;
use prometheus::{
    register_histogram, Counter, CounterVec, Encoder, Histogram, Opts, Registry, TextEncoder,
};
use reqwest::header::HeaderValue;
use tokio;
use warp::log as http_log;
use warp::Filter;

mod forward;
mod metrics;
mod proto;
mod controller;
mod ingestiontenant;

use forward::forward::process_proxy_payload;
use forward::forward::ForwardingStatistics;

use controller::controller::CONTROLLER;
use controller::controller::worker;



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
    #[argh(option, default = "default_content_length_limit()")]
    content_length_limit: u64,

    /// comma-separated list of metrics which identify tenant
    #[argh(option, default = "String::from(\"tenant_id\")")]
    tenant_label_list: String,

    /// comma-separated list of tenants id to replicate whole stream
    #[argh(option, default = "String::from(\"0\")")]
    default_tenant_list: String,

    /// upstream url of cortex ingester deployment
    #[argh(option, default = "String::from(\"http://127.0.0.1:5000\")")]
    ingester_upstream_url: String,

    /// maximum number of requests per single payload to invoke in parallel
    #[argh(option, default = "default_parallel_requests_per_load()")]
    max_parallel_request_per_load: u16,

    /// allow listed tenants (optional, by default proxy all tenants)
    #[argh(option, default = "String::from(\"\")")]
    allow_listed_tenants: String,

    /// start Kubernetes controller for IngestionTenant CRD
    #[argh(option, default = "default_k8s_interval()")]
    kubernetes_poll_interval_seconds: u32,
}

// port
fn default_port() -> u16 {
    19093
}

// k8s interval
fn default_k8s_interval() -> u32 { 120 }

// requests per load
fn default_parallel_requests_per_load() -> u16 {
    64
}

// content length limit
fn default_content_length_limit() -> u64 {
    100 * 1024 * 1024
}


#[tokio::main]
async fn main() {
    env_logger::init();
    let http_log_wrapper = http_log("Open-Metrics-multi-tenancy-Proxy");

    let args: OpenMetricsProxyArgs = argh::from_env();

    // Shared variables
    let tenant_label_list = args.tenant_label_list.clone().to_owned();
    let replicate_to_list = args.default_tenant_list.clone().to_owned();
    let tenant_allow_list = args.allow_listed_tenants.clone().to_owned();
    let ingester_stream_url = args.ingester_upstream_url.clone().to_owned();
    let k8s_poll_interval_seconds = args.kubernetes_poll_interval_seconds.clone().to_owned();
    let _parallel_request_per_load = args.max_parallel_request_per_load.clone().to_owned();
    let interface = args.interface.clone().to_owned();

    let tenant_labels = tenant_label_list
        .split(",")
        .map(|s| s.to_string())
        .collect();
    let replicate_to: Vec<String> = replicate_to_list
        .split(",")
        .map(|s| s.to_string())
        .collect();

    let allowed_tenants: Vec<String> = tenant_allow_list
        .split(",")
        .map(|s| s.to_string())
        .collect();
    let uses_allow_listing = !allowed_tenants.clone().is_empty();

    let allow_listed_tenants = if uses_allow_listing {
        replicate_to
            .to_vec()
            .into_iter()
            .chain(allowed_tenants.into_iter())
            .collect()
    } else {
        allowed_tenants
    };

    let mut headers = reqwest::header::HeaderMap::new();

    // remote write protocol version header
    headers.insert(
        "X-Prometheus-Remote-Write-Version",
        HeaderValue::from_str("0.1.0").unwrap(),
    );
    headers.insert("User-Agent", HeaderValue::from_str("OM_mt_P").unwrap());

    // shared client instance
    let client = reqwest::ClientBuilder::new()
        .default_headers(headers)
        .http1_title_case_headers()
        .build()
        .unwrap();

    let r = Registry::new();

    // Prometheus metrics
    let num_series_opts = Opts::new("open_metrics_proxy_series", "number of series");
    let num_series = CounterVec::new(num_series_opts, &["tenant_id"]).unwrap();
    r.register(Box::new(num_series.clone())).unwrap();

    let total_requests_opts = Opts::new("open_metrics_proxy_requests", "number of series");
    let total_requests = CounterVec::new(total_requests_opts, &["tenant_id"]).unwrap();
    r.register(Box::new(total_requests.clone())).unwrap();

    let num_failures_opts = Opts::new("open_metrics_proxy_failures", "number of series");
    let num_failures = Counter::with_opts(num_failures_opts).unwrap();
    r.register(Box::new(num_failures.clone())).unwrap();

    let num_labels_opts = Opts::new("open_metrics_proxy_labels", "labels detected");
    let num_labels = Counter::with_opts(num_labels_opts).unwrap();
    r.register(Box::new(num_labels.clone())).unwrap();

    let tenants_detected_opts = Opts::new("open_metrics_proxy_tenants", "tenants detetcted");
    let tenants_detected = Counter::with_opts(tenants_detected_opts).unwrap();
    r.register(Box::new(tenants_detected.clone())).unwrap();

    let num_metadata_opts = Opts::new("open_metrics_proxy_metadata", "number of metadata");
    let num_metadata = Counter::with_opts(num_metadata_opts).unwrap();
    r.register(Box::new(num_metadata.clone())).unwrap();

    let histogram = register_histogram!(
        "open_metrics_proxy_processing_ms",
        "processing time milliseconds",
        vec![10.0, 50.0, 100.0, 250.0, 500.0, 800.0, 1200.0, 2000.0]
    )
    .unwrap();
    r.register(Box::new(histogram.clone())).unwrap();

    let mut counter_vecs = HashMap::<u8, CounterVec>::new();
    counter_vecs.insert(ForwardingStatistics::TotalRequests as u8, total_requests);
    counter_vecs.insert(ForwardingStatistics::NumSeries as u8, num_series);

    let mut counters = HashMap::<u8, Counter>::new();
    counters.insert(ForwardingStatistics::NumFailures as u8, num_failures);
    counters.insert(ForwardingStatistics::NumLabels as u8, num_labels);
    counters.insert(
        ForwardingStatistics::TenantsDetected as u8,
        tenants_detected,
    );
    counters.insert(ForwardingStatistics::NumMetadata as u8, num_metadata);

    let mut histograms = HashMap::<u8, Histogram>::new();
    histograms.insert(ForwardingStatistics::ProcessingTime as u8, histogram);

    fn with_parameter_bool(
        param_bool: bool,
    ) -> impl Filter<Extract = (bool,), Error = Infallible> + Clone {
        warp::any().map(move || param_bool.clone())
    };

    fn with_parameter_vec(
        param_vec: Vec<String>,
    ) -> impl Filter<Extract = (Vec<String>,), Error = Infallible> + Clone {
        warp::any().map(move || param_vec.clone())
    };

    fn with_ingester_url(
        ingester_url: String,
    ) -> impl Filter<Extract = (String,), Error = Infallible> + Clone {
        warp::any().map(move || ingester_url.clone())
    }

    fn with_counters(
        __counters: HashMap<u8, Counter>,
    ) -> impl Filter<Extract = (HashMap<u8, Counter>,), Error = Infallible> + Clone {
        warp::any().map(move || __counters.clone())
    }

    fn with_counters_vec(
        __counter_vecs: HashMap<u8, CounterVec>,
    ) -> impl Filter<Extract = (HashMap<u8, CounterVec>,), Error = Infallible> + Clone {
        warp::any().map(move || __counter_vecs.clone())
    }

    fn with_histograms(
        __histograms: HashMap<u8, Histogram>,
    ) -> impl Filter<Extract = (HashMap<u8, Histogram>,), Error = Infallible> + Clone {
        warp::any().map(move || __histograms.clone())
    }

    fn with_registry(
        __r: Registry,
    ) -> impl Filter<Extract = (Registry,), Error = Infallible> + Clone {
        warp::any().map(move || __r.clone())
    }

    // match any post request and perform proxying
    let proxy = warp::any()
        .and(warp::post())
        .and(warp::body::content_length_limit(args.content_length_limit))
        .map(move || client.clone())
        .and(with_parameter_vec(tenant_labels))
        //.and(with_tenants())
        .and(with_parameter_bool(uses_allow_listing))
        .and(with_parameter_vec(replicate_to))
        .and(with_ingester_url(ingester_stream_url))
        .and(with_counters(counters))
        .and(with_counters_vec(counter_vecs))
        .and(with_histograms(histograms))
        .and(warp::body::bytes())
        .and_then(
            move |_client,
                  _tenant_labels,
                  _uses_allow_listing,
                  _replicate_to,
                  _ingester_stream_url,
                  _counters,
                  _counter_vecs,
                  _histograms,
                  _bytes| async move {
                // This is safe since ARC have been cloned inside view once
                let _c = CONTROLLER.read().await;
                let _tnts = Arc::new(_c.get_tenants());
                process_proxy_payload(
                    _client,
                    _tenant_labels,
                    _tnts,
                    _uses_allow_listing,
                    _replicate_to,
                    _ingester_stream_url,
                    _parallel_request_per_load,
                    &_counters,
                    &_counter_vecs,
                    &_histograms,
                    _bytes,
                )
                    .await
                    .and_then(|response| Ok(response))
                    .map_err(|e| {
                        error!("Internal Error: {}", e);
                        warp::reject::reject()
                    })
            },
        )
        .with(http_log_wrapper);

    // match any get request and return status
    let health = warp::any().and(warp::get()).map(|| "Up\n");


    let metrics = warp::path!("metrics")
        .and(warp::get())
        .and(with_registry(r))
        .map(|_r: Registry| {
            // Gather the metrics.
            let mut buffer = vec![];
            let encoder = TextEncoder::new();
            let metric_families = _r.gather();
            encoder.encode(&metric_families, &mut buffer).unwrap();

            // Output to http body
            String::from_utf8(buffer).unwrap()
        });


    // init controller parameters
    let mut c = CONTROLLER.write().await;
    c.set_initial_allowed_tenants(allow_listed_tenants)
        .set_k8s_poll_delay((k8s_poll_interval_seconds * 1000) as u64);
    tokio::task::spawn(worker());
    drop(c);

    let listen_addr = interface.parse::<Ipv4Addr>();

    let exit_code = match listen_addr {
        Ok(ip) => {
            warp::serve(metrics.or(health).or(proxy))
                .run((ip, args.port)).await;
            0
        }
        Err(e) => {
            error!("Invalid IP address: {}, err={}", interface, e);
            2
        }
    };

    exit(exit_code);
}
