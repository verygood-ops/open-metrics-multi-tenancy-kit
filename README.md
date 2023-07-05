open-metrics-multi-tenancy-kit(aka Injection Proxy)
=================================

![example workflow](https://github.com/verygood-ops/open-metrics-multi-tenancy-kit/actions/workflows/push.yml/badge.svg)

Injection Proxy implements multi-tenancy for metrics collection and
rules management in Cortex environments, configured via `OpenMetricsRule` Kubernetes resource.

Injection Proxy has two parts
1) An informer is the service which is responsible for synchronizing Prometheus Rules state between Kubernetes and Ruler.
2) A Proxy exposing GRPC socket, that listens for Prometheus Remote Write metrics

Following is the flow
1) Metrics Vector reads audit log from Kinesis stream vault-sanitized-data
2) Metrics Vector uses grafana agent to ship metrics to Injection Proxy
3) Injection Proxy stores metrics in cortex in default tenant and per tenant basis
4) Metrics stored in default tenant is for internal purpose 
5) Metrics stored per tenant is exposed out to each customer 

Component Diagram: https://verygoodsecurity.atlassian.net/wiki/spaces/EN/pages/1002471427/What+is+Observability#Components-diagram

Sequence Diagram: https://verygoodsecurity.atlassian.net/wiki/spaces/EN/pages/1002471427/What+is+Observability#Open-metrics-multi-tenancy-kit-(Open-MMT-kit)

Metrics flow:  https://verygoodsecurity.atlassian.net/wiki/spaces/EN/pages/1002471427/What+is+Observability#Design

Cortex end point: https://tenant.cortex.ops.verygood.systems/


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

## Code

### Tech Stack
- Rust 
- Injection Proxy is deployed as OpenMetricsRule in external-metrics namespace: prod/logs cluster


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
