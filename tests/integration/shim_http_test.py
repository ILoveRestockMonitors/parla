import http.client
import importlib.util
import io
import json
import pathlib
import sys
import threading
import types
import unittest
import wave


class FakeArray:
    def __init__(self, values):
        self.values = list(values)
        self.size = len(self.values)

    def __truediv__(self, value):
        return FakeArray([x / value for x in self.values])


fake_numpy = types.SimpleNamespace(float32=float, asarray=lambda x, dtype=None: FakeArray(x))
sys.modules["numpy"] = fake_numpy
path = pathlib.Path(__file__).parents[2] / "src-tauri" / "assets" / "parakeet-shim.py"
spec = importlib.util.spec_from_file_location("parakeet_shim_http", path)
shim = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shim)


def wav(frames=1000, bits=16, channels=1, rate=16000):
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(bits // 8)
        w.setframerate(rate)
        w.writeframes(b"\x01\x00" * frames * channels)
    return out.getvalue()


class FakeStream:
    def __init__(self):
        self.result = types.SimpleNamespace(text="fake transcript")

    def accept_waveform(self, rate, audio):
        self.rate, self.audio = rate, audio


class FakeRecognizer:
    def create_stream(self):
        return FakeStream()

    def decode_stream(self, stream):
        pass


class ShimHttpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        shim.RECOGNIZER = FakeRecognizer()
        shim.MODEL_DIRECTORY = "c:\\fake-model"
        cls.server = shim.BoundedHTTPServer(("127.0.0.1", 0), shim.Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.port = cls.server.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=2)

    def request(self, method, path, body=None, headers=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=3)
        conn.request(method, path, body=body, headers=headers or {})
        response = conn.getresponse()
        payload = response.read()
        conn.close()
        return response.status, json.loads(payload)

    def test_health_is_typed(self):
        status, payload = self.request("GET", "/health")
        self.assertEqual(status, 200)
        self.assertEqual(payload["backend"], "parakeet")
        self.assertTrue(payload["ok"])
        self.assertEqual(payload["model"], "nemo_transducer")
        self.assertEqual(payload["protocol_version"], 2)
        self.assertEqual(payload["model_directory"], "c:\\fake-model")

    def test_valid_wav_reaches_fake_recognizer(self):
        body = wav()
        status, payload = self.request("POST", "/inference", body,
                                       {"Content-Type": "application/octet-stream",
                                        "Content-Length": str(len(body))})
        self.assertEqual(status, 200)
        self.assertEqual(payload, {"text": "fake transcript"})

    def test_bad_wav_is_rejected(self):
        for body in (wav(rate=8000), wav(bits=8), wav(channels=2), wav(frames=0), wav()[:-2]):
            status, payload = self.request("POST", "/inference", body,
                                           {"Content-Length": str(len(body))})
            self.assertEqual(status, 500)
            self.assertIn("error", payload)

    def test_ambiguous_or_oversized_framing_is_rejected_before_body(self):
        for headers in [[], [("Content-Length","-1")], [("Content-Length","0"),("Content-Length","0")], [("Content-Length",str(40*1024*1024+1))], [("Transfer-Encoding","chunked")]]:
            conn=http.client.HTTPConnection("127.0.0.1",self.port,timeout=3)
            conn.putrequest("POST","/inference")
            for key,value in headers:
                conn.putheader(key,value)
            conn.endheaders()
            response=conn.getresponse()
            self.assertEqual(response.status,500)
            self.assertIn("error",json.loads(response.read()))
            conn.close()

    def test_busy_rejected_before_body_read(self):
        self.assertTrue(shim.INFERENCE_LOCK.acquire(blocking=False))
        try:
            conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=3)
            conn.putrequest("POST", "/inference")
            conn.putheader("Content-Length", str(40 * 1024 * 1024))
            conn.endheaders()
            response = conn.getresponse()
            self.assertEqual(response.status, 429)
            conn.close()
        finally:
            shim.INFERENCE_LOCK.release()


if __name__ == "__main__":
    unittest.main()
