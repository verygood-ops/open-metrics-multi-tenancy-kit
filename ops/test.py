import time
import requests
import snappy
from prometheus_pb2 import Sample, TimeSeries, WriteRequest, Label


def metric(labels, value, timestamp=None):
    s = Sample()

    if not timestamp:
        timestamp = int(time.time())

    s.timestamp = timestamp
    s.value = value
    ts = TimeSeries()

    ts.samples.append(s)
    wr = WriteRequest()
    for label_name, label_value in labels.items():
        l = Label()
        l.name = label_name
        l.value = label_value
        ts.labels.append(l)
    wr.timeseries.append(ts)
    return wr


requests.post(data=snappy.compress(metric({'tenant_id': 'test-vgs', '__name__': 'test_metric'}, 400).SerializeToString()), url='http://127.0.0.1:19093')
