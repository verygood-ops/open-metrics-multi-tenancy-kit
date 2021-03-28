use log::{debug, error};
use argh::FromArgs;

use std::time::Duration;
use std::net::Ipv4Addr;
use std::process::exit;
use std::convert::Infallible;

use kube::Client;
use prometheus::{
    Counter, Encoder, Opts, Registry, TextEncoder,
};
use tokio;
use tokio::time::interval;
use env_logger;
use reqwest::ClientBuilder;
use reqwest::header::HeaderValue;
use warp::log as http_log;
use warp::Filter;

mod tracker;
use tracker::tracker::tracker;


#[derive(FromArgs)]
/// Open Metrics multi tenancy Proxy
struct OpenMetricsInformerArgs {
    /// port for serving metrics (default 20093)
    #[argh(option, default = "default_port()")]
    port: u16,

    /// port for serving http (default 127.0.0.1)
    #[argh(option, default = "String::from(\"127.0.0.1\")")]
    interface: String,

    /// upstream url of cortex ruler deployment
    #[argh(option, default = "String::from(\"http://127.0.0.1:8080/\")")]
    ruler_upstream_url: String,

    /// start Kubernetes controller for IngestionTenant CRD
    #[argh(option, default = "default_k8s_interval()")]
    kubernetes_poll_interval_seconds: u32,
}

// port
fn default_port() -> u16 {
    20093
}

// k8s interval
fn default_k8s_interval() -> u32 { 30 }

#[tokio::main]
pub async fn main() {
    env_logger::init();
    let http_log_wrapper = http_log("Open-Metrics-multi-tenancy-Informer");

    let args: OpenMetricsInformerArgs = argh::from_env();

    let interface = args.interface.clone().to_owned();
    let ruler_upstream_url = args.ruler_upstream_url.clone().to_owned();
    let k8s_poll_interval_seconds = args.kubernetes_poll_interval_seconds.clone().to_owned();

    let r = Registry::new();

    let num_rules_opts = Opts::new("open_metrics_informer_tracker_rules", "rules detected");
    let num_rules = Counter::with_opts(num_rules_opts).unwrap();
    r.register(Box::new(num_rules.clone())).unwrap();

    let tenants_detected_opts = Opts::new("open_metrics_informer_tracker_tenants", "tenants detetcted");
    let tenants_detected = Counter::with_opts(tenants_detected_opts).unwrap();
    r.register(Box::new(tenants_detected.clone())).unwrap();

    let num_rules_updated_opts = Opts::new("open_metrics_informer_updater_rules", "rules updated");
    let num_rules_updated = Counter::with_opts(num_rules_updated_opts).unwrap();
    r.register(Box::new(num_rules_updated.clone())).unwrap();

    let tenants_updated_opts = Opts::new("open_metrics_informer_updater_tenants", "tenants updated");
    let tenants_updated = Counter::with_opts(tenants_updated_opts).unwrap();
    r.register(Box::new(tenants_updated.clone())).unwrap();

    fn with_registry(
        __r: Registry,
    ) -> impl Filter<Extract = (Registry,), Error = Infallible> + Clone {
        warp::any().map(move || __r.clone())
    }

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
        }).with(http_log_wrapper);

    let k8s_client = match Client::try_default().await {
        Ok(k_c) => {
            debug!("Initialized k8s client");
            Some(k_c)
        },
        Err(e) => {
            error!("Failed to instantiate k8s client: {}", e.to_string());
            None
        }
    };

    let e = if k8s_client.is_some() {
        let listen_addr = interface.parse::<Ipv4Addr>();

        // reqwest machinery all safe to unwrap since headers are static
        let mut headers = reqwest::header::HeaderMap::new();

        // remote write protocol version header
        headers.insert(
            "X-Prometheus-Remote-Write-Version",
            HeaderValue::from_str("0.1.0").unwrap(),
        );
        headers.insert("User-Agent", HeaderValue::from_str("OM_mt_I").unwrap());

        // shared client instance
        let ruler_client = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .http1_title_case_headers()
            .build()
            .unwrap();


        tokio::task::spawn(tracker(
            // It is safe to unwrap, since client should be inited by the point.
            k8s_client.unwrap().clone(),
            ruler_client.clone(),
            ruler_upstream_url.clone(),
            Box::new(num_rules.clone()),
            Box::new(tenants_detected.clone()),
            (k8s_poll_interval_seconds * 1000 ).into()));


        let exit_code = match listen_addr {
            Ok(ip) => {
                // Spawn rule tracker

                // Spawn rule updater
                // tokio::task::spawn(updater(Box::new(), Box::new()));
                warp::serve(metrics)
                    .run((ip, args.port)).await;
                0
            }
            Err(e) => {
                error!("Invalid IP address: {}, err={}", interface, e);
                2
            }
        };
        exit_code

    } else {
        error!("Can not initialize k8s client, will go down ...");
        let mut interval = interval(Duration::from_millis(7000));
        interval.tick().await;
        1
    };

    exit(e);

}
