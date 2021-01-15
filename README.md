open-metrics-multi-tenancy-proxy
=================================

OM-mt-P allows to implement multi-tenancy for metrics collection
in Cortex environments.

It introspects Prometheus GRPC metrics `remote_write` stream,
injects tenants in accordance to https://cortexmetrics.io/docs/guides/auth/,
and forwards requests to specified upstream.

How it works
------------
<img src='https://g.gravizo.com/svg?
%40startuml%3B%0A%0Aactor%20DevOps%3B%0Aparticipant%20%22Prometheus%20Exporters%22%20as%20E%3B%0Aparticipant%20%22Prometheus%22%20as%20P%3B%0Aparticipant%20%22Open%20Metrics%20Proxy%22%20as%20O%3B%0Aparticipant%20%22Cortex%22%20as%20C%3B%0A%0AE%20-%3E%20P%3A%20%22Receive%20Metrics%22%3B%0AP%20-%3E%20O%3A%20%22Prometheus%20Remote%20Write%20send%22%3B%0AO%20-%3E%20O%3A%20%22Repack%20metrics%20tenant-wise%22%3B%0AO%20-%3E%20C%3A%20%22Send%20repacked%20metrics%20with%20X-Scope-OrgID%22%3B%0A%40enduml
'>

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
