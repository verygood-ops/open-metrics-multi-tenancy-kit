open-metrics-multi-tenancy-kit
=================================

![example workflow](https://github.com/verygood-ops/open-metrics-multi-tenancy-kit/actions/workflows/push.yml/badge.svg)


`open-metrics-multi-tenancy-kit` implements multi-tenancy for metrics collection and
rules management in Cortex environments, configured via `OpenMetricsRule` Kubernetes resource.

An example `OpenMetricsRule` is below.

```
apiVersion: "open-metrics.vgs.io/v1"
kind: "OpenMetricsRule"
metadata:
  name: open-metrics-test
  namespace: open-metrics-test
spec:
  tenants:
    - "backoffice"
    - "frontend"
  description: "A hello world rule for recording and alerting."
  groups:

    - name: open_metrics_rule_record_v0
      interval: 1m
      rules:
        - expr: histogram_quantile(0.99, rate(http_request_processing_duration_ms_bucket[5m]))
          record: p99:http_request_duration_ms:5m
        - expr: histogram_quantile(0.99, rate(http_request_processing_duration_ms_bucket[2m]))
          record: p99:http_request_duration_ms:2m

    - name: open_metrics_rule_alert_v0
      rules:
        - expr: p99:http_request_duration_ms:5m > 400
          for: 5m
          alert: HttpRequestProcessingP99TooLarge
          labels:
            severity: critical
```



Metrics collection multi-tenancy
--------------------------------
A proxy component introspects Prometheus GRPC metrics `remote_write` stream,
injects tenants in accordance to 
[Cortex Authentication and Authorization](https://cortexmetrics.io/docs/guides/auth/),
and forwards requests to specified upstream. Tenants can be specified either
via command line, or specified via `spec.tenants` property of 
`OpenMetricsRule` Kubernetes resource.

See [proxy/README.md](proxy/README.md) for futher details on `open-metrics-multi-tenancy-proxy` functioning.

Rule And Alert Management
-------------------------
Metrics rules, both tecording and alerting are set up via `OpenMetricsRule` Kubernetes resources.
An informer component loads rules into Ruler.

See [informer/README.md](informer/README.md) for futher details on `open-metrics-multi-tenancy-proxy` functioning.

Open metrics exposition
-----------------------
TBD.

Rule expression validation
--------------------------
TBD.

How to build
-------------

Local:

1) [Install Rust](https://doc.rust-lang.org/cargo/getting-started/installation.html)
2) [Install protobuf-compiler](https://grpc.io/docs/protoc-installation/)
3) `cargo build`

In docker,

`docker-compose build`

How to test
-----------

On localhost,

```
RUST_LOG=debug RUST_BACKTRACE=full cargo test -- --nocapture
```

Docker:

`docker-compose run test`

How to run
----------
On localhost,

Run proxy
```
RUST_LOG=debug RUST_BACKTRACE=full cargo run \
    --bin open-metrics-multi-tenancy-proxy
```

Run informer

```
RUST_LOG=debug RUST_BACKTRACE=full cargo run \
    --bin open-metrics-multi-tenancy-informer
```

In Docker,

`docker-compose up`


How to monitor
--------------

Use Prometheus!

A proxy component exposes following Prometheus metrics:

- `open_metrics_proxy_requests`           -- number of requests to distributor, per tenant
- `open_metrics_proxy_series`             -- number of forwarded series, per tenant
- `open_metrics_proxy_failures`           -- number of forwarding errors, per process
- `open_metrics_proxy_labels`             -- number of requests to distributor, per process
- `open_metrics_proxy_metadata`           -- number of metrics metadata seen (usually for each kind of metrics forwarded once)
- `open_metrics_proxy_processing_ms`      -- histogram of durations

An informer component exposes following prometheus metrics:

- `open_metrics_informer_tracker_rules`   -- number of rules seen by tracker, per tenant
- `open_metrics_informer_tracker_tenants` -- increases each time new tenant seen in tracker rules, per tenant
- `open_metrics_informer_updater_rules`   -- number of rules seen by updater, per tenant
- `open_metrics_informer_updater_tenants` -- increases each time new tenant seen in updater rules, per tenant


Known limitations
------------------
- no validation for duplicated recording rules or alerts
- no support for multiple Kubernetes namespaces
- no real health check for proxy or informer (can be remediated by using Prometheus metrics)
