open-metrics-multi-tenancy-kit
=================================

OM-mt-K allows to implement multi-tenancy for metrics collection,
rule and alert management, and exposition in Cortex environments.

Metrics collection multi-tenancy
--------------------------------
A proxy component introspects Prometheus GRPC metrics `remote_write` stream,
injects tenants in accordance to https://cortexmetrics.io/docs/guides/auth/,
and forwards requests to specified upstream. Tenants can be either enabled
via command line, or specified via `IngestionTenant` k8s resources, managed by Controller.

Rule And Alert Management
-------------------------
Rules and Alerts are set up via k8s resources.
A controller component loads rules into Cortex.
A webhook component validates rules and alerts submitted.
`OpenMetricsRule` -- specifies open metrics recording rule
`OpenMetricsAlert` -- specifies open metrics alert

Open metrics exposition
-----------------------
TBD.

How to build
-------------

`cargo build`


How to test
-----------

```
RUST_BACKTRACE=full RUST_LOG=debug cargo run -- \
    --ingester-upstream-url <CORTEX_INGESTER_URL>
```

Command line options
--------------------

A proxy application always listens on 127.0.0.1.

- `--port`                          -- a port to listen on
- `--content-length-limit`          -- maximum incoming request body in bytes
- `--tenant-label-list`             -- a comma-separated list of labels with values to be recognized as tenant ID
- `--default-tenant-list`           -- a comma-separated list of tenants to replicate all metrics nevertheless the labels
- `--ingester-upstream-url`         -- an ingester upstream HTTP(s) URL
- `--max-parallel-request-per-load` -- max number of downstream requests to invoke in parallel when proxying single request
