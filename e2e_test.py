import logging
import random
import shlex
import socket
import subprocess
import sys
import time
import threading
import unittest

import requests
import snappy
from prometheus_pb2 import WriteRequest, TimeSeries, Label, Sample


logger = logging.getLogger(__name__)
PORT_RANGE = (48150, 48250)
UPSTREAM_PORT_RANGE = (48251, 48350)


def consume_socket(sock, chunks=65536):
    consumed = bytearray()
    while True:
        b = sock.recv(chunks)
        consumed += b
        if b.endswith(b"\r\n\r\n"):
            break
    return consumed


def wait_up(port):

    while True:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            s.connect(('127.0.0.1', int(port)))
            s.shutdown(2)
            return True
        except:
            logger.info('Failed sock open')
            time.sleep(10)


def wait_http(port):
    while True:
        try:
            requests.get('http://127.0.0.1:{}'.format(port))
            return True
        except:
            logger.info('Failed http request')
            time.sleep(10)


class SocketServerThread(threading.Thread):
    """
    :param socket_handler: Callable which receives a socket argument for one
        request.
    :param ready_event: Event which gets set when the socket handler is
        ready to receive requests.
    """

    USE_IPV6 = False
    UPSTREAM_PORT = random.randint(*UPSTREAM_PORT_RANGE)

    def __init__(self, socket_handler, host="localhost", ready_event=None):
        super().__init__()
        self.daemon = True

        self.socket_handler = socket_handler
        self.host = host
        self.ready_event = ready_event
        self.port = self.__class__.UPSTREAM_PORT

    def _start_server(self):
        if self.USE_IPV6:
            sock = socket.socket(socket.AF_INET6)
        else:
            sock = socket.socket(socket.AF_INET)
        if sys.platform != "win32":
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind((self.host, self.port))
        self.port = sock.getsockname()[1]

        # Once listen() returns, the server socket is ready
        sock.listen(1)

        if self.ready_event:
            self.ready_event.set()

        self.socket_handler(sock)
        sock.close()

    def run(self):
        self.server = self._start_server()


class BaseTestCase(unittest.TestCase):

    HOST = '127.0.0.1'
    PORT = random.randint(*PORT_RANGE)

    @classmethod
    def _start_server(cls, socket_handler):
        ready_event = threading.Event()
        cls.server_thread = SocketServerThread(
                socket_handler=socket_handler, ready_event=ready_event, host=cls.HOST
        )
        cls.server_thread.start()
        ready_event.wait(5)
        if not ready_event.is_set():
            raise Exception("most likely failed to start server")
        cls.UPSTREAM_PORT = cls.server_thread.port

    def metric(self, labels, value, timestamp=None):
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

    def send_metric(self, labels, value, timestamp=None):
        write_request = self.metric(labels, value, timestamp)
        return requests.post(data=snappy.compress(write_request.SerializeToString()), url=self.url())

    @classmethod
    def start_response_handler(cls, response, num=1, block_send=None):
        ready_event = threading.Event()

        def socket_handler(listener):
            for _ in range(num):
                ready_event.set()

                sock = listener.accept()[0]
                consume_socket(sock)
                if block_send:
                    block_send.wait()
                    block_send.clear()
                sock.send(response)
                sock.close()

        cls._start_server(socket_handler)
        return ready_event

    @classmethod
    def start_basic_handler(cls, **kw):
        return cls.start_response_handler(
                b"HTTP/1.1 200 OK\r\n" b"Content-Length: 0\r\n" b"\r\n", **kw
        )

    @classmethod
    def ingester_url(cls):
        return "http://{}:{}".format(cls.HOST, cls.PORT)

    @classmethod
    def url(cls):
        return "http://{}:{}".format(cls.HOST, cls.PORT)

    @classmethod
    def setUpClass(cls):
        logger.info('Start up OM-mt-P server.')
        cls.popen = subprocess.Popen(
                args=shlex.split(
                        "cargo run -- --port {} --ingester-upstream-url {}".format(
                                cls.PORT, cls.ingester_url())))
        wait_up(cls.PORT)
        wait_http(cls.PORT)
        logger.info('Started OM-mt-P server.')

    def setUp(self):
        self.start_response_handler(
                b"HTTP/1.1 200 OK\r\n" b"Content-Length: 0\r\n" b"\r\n",
        )
        wait_http(SocketServerThread.UPSTREAM_PORT)

    def tearDown(self):
        if hasattr(self.__class__, "server_thread"):
            self.__class__.server_thread.join(0.1)

    @classmethod
    def tearDownClass(cls) -> None:
        if hasattr(cls, 'popen'):
            cls.popen.terminate()

    def get(self, **kwargs):
        kwargs['url'] = self.url()
        return requests.get(**kwargs)

    def post(self, **kwargs):
        kwargs['url'] = self.url()
        return requests.post(**kwargs)

    def test_up(self):
        r = self.get()
        self.assertEqual(200, r.status_code)
        self.assertEqual('Up\n', r.text)

    def test_invalid_snap(self):
        r = self.post(data='testinvalidsnap')
        self.assertEqual(400, r.status_code)

    def test_invalid_protobuf(self):
        r = self.post(data=snappy.compress('testinvalidprotobuf'.encode()))
        self.assertEqual(400, r.status_code)

    def test_proxy_metric(self):
        r = self.send_metric({'tenant_id': 'test-vgs', '__name__': 'test_metric'}, 400)
        self.assertEqual(200, r.status_code)
        import pdb;pdb.set_trace()
        pass


if __name__ == '__main__':
    logging.basicConfig(level=logging.DEBUG, stream=sys.stderr)
    unittest.main()
