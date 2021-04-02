#![deny(warnings)]
use log::{debug, error, warn};
use argh::FromArgs;

use std::time::Duration;
use std::net::Ipv4Addr;
use std::process::exit;
use std::convert::Infallible;

use kube::Client;
use prometheus::{
    IntCounterVec, Encoder, Opts, Registry, TextEncoder,
};
use tokio;
use tokio::time::interval;
use env_logger;
use reqwest::header::HeaderValue;
use warp::log as http_log;
use warp::Filter;

// common routines
mod crud;
mod rules;

// ruler -> k8s sync component
mod tracker;
use tracker::tracker::tracker;

// k8s -> ruler sync component
mod updater;
use updater::updater::updater;

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
    #[argh(option, default = "default_tracker_interval()")]
    tracker_poll_interval_seconds: u32,

    /// start Kubernetes controller for IngestionTenant CRD
    #[argh(option, default = "default_updater_interval()")]
    updater_poll_interval_seconds: u32,

    /// enable removing rules for updater
    #[argh(switch)]
    enable_updater_remove_rules: bool,
}

// port
fn default_port() -> u16 {
    20093
}

// k8s -> ruler interval
fn default_updater_interval() -> u32 { 40 }

// ruler -> k8s interval
fn default_tracker_interval() -> u32 { 93 }


#[tokio::main]
pub async fn main() {
    env_logger::init();
    let http_log_wrapper = http_log("Open-Metrics-multi-tenancy-Informer");

    let args: OpenMetricsInformerArgs = argh::from_env();

    let interface = args.interface.clone().to_owned();
    let ruler_upstream_url = args.ruler_upstream_url.clone().to_owned();
    let tracker_poll_interval_seconds = args.tracker_poll_interval_seconds.clone().to_owned();
    let updater_poll_interval_seconds = args.updater_poll_interval_seconds.clone().to_owned();

    let enable_updater_remove_rules = args.enable_updater_remove_rules.clone();

    let r = Registry::new();

    let num_rules_opts = Opts::new("open_metrics_informer_tracker_rules", "rules detected");
    let num_rules = IntCounterVec::new(num_rules_opts, &["tenant_id"]).unwrap();
    r.register(Box::new(num_rules.clone())).unwrap();

    let tenants_detected_opts = Opts::new("open_metrics_informer_tracker_tenants", "tenants detetcted");
    let tenants_detected = IntCounterVec::new(tenants_detected_opts, &["tenant_id"]).unwrap();
    r.register(Box::new(tenants_detected.clone())).unwrap();

    let num_rules_updated_opts = Opts::new("open_metrics_informer_updater_rules", "rules updated");
    let num_rules_updated = IntCounterVec::new(num_rules_updated_opts, &["tenant_id"]).unwrap();
    r.register(Box::new(num_rules_updated.clone())).unwrap();

    let tenants_updated_opts = Opts::new("open_metrics_informer_updater_tenants", "tenants updated");
    let tenants_updated = IntCounterVec::new(tenants_updated_opts, &["tenant_id"]).unwrap();
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

        let cloned_client = k8s_client.unwrap().clone();

        if tracker_poll_interval_seconds > 0 {
            tokio::task::spawn(tracker(
                // It is safe to unwrap, since client should be inited by the point.
                cloned_client.clone(),
                ruler_client.clone(),
                ruler_upstream_url.clone(),
                Box::new(num_rules.clone()),
                Box::new(tenants_detected.clone()),
                (tracker_poll_interval_seconds * 1000 ).into()
            ));
        } else {
            warn!("tracker component disabled");
        };

        if updater_poll_interval_seconds > 0 {
            tokio::task::spawn(updater(
                // It is safe to unwrap, since client should be inited by the point.
                cloned_client.clone(),
                ruler_client.clone(),
                ruler_upstream_url.clone(),
                Box::new(num_rules_updated.clone()),
                Box::new(tenants_updated.clone()),
                (updater_poll_interval_seconds * 1000 ).into(),
                !enable_updater_remove_rules
            ));
        } else {
            warn!("updater component disabled");
        };

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
