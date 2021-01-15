.PHONY: test

proto_py:
	python3 -m grpc_tools.protoc -I . --python_out=. config/prometheus.proto && cp -f config/prometheus_pb2.py . && rm config/prometheus_pb2.py

test: pip proto_py
	python3 e2e_test.py

pip:
	python3 -m pip install aiohttp aiohttp_retry protobuf grpcio-tools python-snappy
