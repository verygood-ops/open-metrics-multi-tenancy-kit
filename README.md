open-metrics-multi-tenancy-proxy
=================================

OM-mt-P allows to implement multi-tenancy for metrics collection
in Cortex environments.

It introspects Prometheus GRPC metrics `remote_write` stream,
injects tenants in accordance to https://cortexmetrics.io/docs/guides/auth/,
and forwards requests to specified upstream.


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
