open-metrics-multi-tenancy-proxy
================================

How it works
------------

`open-metrics-multi-tenancy-proxy` (OM-mt-P) exposes GRPC socket, that listens for Prometheus Remote Write metrics.
For each sample inside metrics batch, `OM-mt-P` makes a decision to replicate sample into some tenants, or drop
this sample and bar it from further processing. There are three ways to drive the decisions, one of the following:

1) Replicate all samples into particular tenant.
   In order to make every sample to be replicated into some tenant, pass comma-separated
   list of tenant identifiers as  `--default-tenant-list`, for example

   `--default-tenant-list tntX,tntY`

  will make metrics to replicate into `tntX` and `tntY` storages.

2) Replicate metrics, picking tenant ID based on a sample label.
  A tenant will be picked based on a label value in a sample. To specify a list of label names
   that will be used to drive such decisions, pass `--tenant-label-list` comma-separated

  For example,

    `--tenant-label-list tenant_id,tnt`

  will make metrics to replicate into storages of tenants named based on a value of a metric.


3) Allow replication of samples based on a sample label into pre-defined set of tenants only.

  This feature exists to support a use-case a cardinality of sample label value might induce large number of
   tenant storages being in play, while only a subset of these storages are going to be actually read.
  In order to activate replication based on a value in a label, the value should be either passed as a part
   of `--allow-listed-tenants` parameter, or set up as an entry in `.spec.tenants` field of `MetricsIngestionTenant` CRD.

  See `config/crd/proxy` for `MetricsIngestionTenant` custom resource definition and example uses.


It is possible to use `OM-mt-P` outside of Kubernetes.
For this use-case - `--kubernetes-poll-interval-seconds` should be zero.


Command line options
--------------------

A proxy application always listens on 127.0.0.1.

- `--port`                              -- a port to listen on (default: 19093)
- `--content-length-limit`              -- maximum incoming request body in bytes
- `--tenant-label-list`                 -- a comma-separated list of labels with values to be recognized as tenant ID
- `--default-tenant-list`               -- a comma-separated list of tenants to replicate all metrics nevertheless the labels
- `--ingester-upstream-url`             -- an ingester upstream HTTP(s) URL
- `--max-parallel-request-per-load`     -- max number of downstream requests to invoke in parallel when proxying single request
- `--allow-listed-tenants`              -- a comma-separated list of tenants to use for allow-listing
- `--kubernetes-poll-interval-seconds`  -- number of seconds between polling `MetricsIngestionTenant` resources. pass `0` to disable polling Kubernetes.

Environment variables
---------------------
- `OPEN_METRICS_PROXY_NAMESPACE`        -- a namespace to observe for `OpenMetricsRule` resources
