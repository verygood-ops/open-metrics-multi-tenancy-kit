open-metrics-multi-tenancy-informer
===================================

An informer is the service which is responsible for synchronizing Prometheus Rules state
between Kubernetes and Ruler. This service achieves its purpose by
  - continiously polling  of Kubernetes and Ruler states 
  - finding difference between two states,
  - creating patch updates for either Kubernetes or Ruler side.

A component that updates Kubernetes state is called "tracker".
A component that updates Ruler state is called "updater". 
Tracker never removes anything from Kubernetes,
  while for Updater it is possible to enable the rules removal. 
Generally the latter is not advised,
  unless you know what you are doing.


Command line options
--------------------

A proxy application always serves metrics on 127.0.0.1.

- `--port` -- A port for retrieving metrics (default: 20093)
- `--ruler-upstream-url` -- An upstream URL of Ruler service
- `--tracker-poll-interval-seconds` -- An interval of seconds between tracker polls.
- `--updater-poll-interval-seconds` -- An interval of seconds between updater polls.
- `--enable-updater-remove-rules` -- Updater does not remove k8s resources by default. Pass this flag to enable removal.
