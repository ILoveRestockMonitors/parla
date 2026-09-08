#!/usr/bin/env python3
"""Local Parakeet TDT v2 transducer HTTP adapter.

GET /health reports readiness, backend, protocol_version=2 and model_directory.
POST /inference accepts bounded raw PCM16 mono 16 kHz WAV and returns JSON text.
One decode runs at a time. The current greedy decoder does not use dynamic
hotwords; explicit vocabulary normalization happens in the Rust application.
CPU is the baseline. GPU provider support requires a separately verified runtime.
The app deploys this regenerable shim to a temporary folder; weights are configured
independently. Imports: standard library, NumPy and (when loading) sherpa-onnx.
"""
import argparse
import io
import json
import os
import sys
import wave
import threading
from array import array
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import numpy as np

RECOGNIZER = None  # built once at startup
MODEL_DIRECTORY = None
INFERENCE_LOCK = threading.BoundedSemaphore(1)
MAX_WAV_BYTES = int(os.environ.get("PARLA_PARAKEET_MAX_WAV_BYTES", str(40 * 1024 * 1024)))


def _find_model_files(model_dir):
    """Locate Parakeet ONNX files by pattern (names vary across releases)."""
    enc = dec = join = tok = None
    for name in sorted(os.listdir(model_dir)):
        low = name.lower()
        p = os.path.join(model_dir, name)
        if low.endswith(".onnx") and "encoder" in low and enc is None:
            enc = p
        elif low.endswith(".onnx") and "decoder" in low and "joiner" not in low and dec is None:
            dec = p
        elif low.endswith(".onnx") and "joiner" in low and join is None:
            join = p
        elif low == "tokens.txt":
            tok = p
    missing = [n for n, v in [("encoder*.onnx", enc), ("decoder*.onnx", dec),
                              ("joiner*.onnx", join), ("tokens.txt", tok)] if v is None]
    if missing:
        raise FileNotFoundError(
            f"model dir {model_dir} is missing: {missing} (found: {sorted(os.listdir(model_dir))})"
        )
    return enc, dec, join, tok


def build_recognizer(model_dir):
    import sherpa_onnx

    enc, dec, join, tok = _find_model_files(model_dir)
    print(f"[parakeet-shim] encoder={os.path.basename(enc)}", flush=True)
    kwargs = dict(encoder=enc, decoder=dec, joiner=join, tokens=tok)
    if os.environ.get("PARLA_PARAKEET_GPU") == "1":
        kwargs["provider"] = "cuda"  # sherpa falls back to CPU if unavailable
    # Pin the loader contract used by the tested sherpa-onnx release. Avoid a
    # broad hasattr fallback that can silently select a changed future API.
    kwargs["num_threads"] = int(os.environ.get("PARLA_PARAKEET_THREADS", "4"))
    return sherpa_onnx.OfflineRecognizer.from_transducer(model_type="nemo_transducer", **kwargs)


def wav_bytes_to_f32(body: bytes):
    if not body:
        raise ValueError("empty WAV")
    with wave.open(io.BytesIO(body), "rb") as w:
        if w.getcomptype() != "NONE" or w.getsampwidth() != 2 or w.getframerate() != 16000 or w.getnchannels() != 1:
            raise ValueError(f"expected 16kHz mono, got {w.getframerate()}Hz x{w.getnchannels()}")
        frames = w.getnframes()
        raw = w.readframes(frames)
        if len(raw) != frames * w.getnchannels() * w.getsampwidth():
            raise ValueError("truncated WAV data")
    samples = array("h", raw)
    if not samples:
        raise ValueError("empty WAV data")
    return np.asarray(samples, dtype=np.float32) / 32768.0


def read_body(handler):
    """Read one bounded body with an exact, unambiguous Content-Length."""
    te = (handler.headers.get("Transfer-Encoding") or "").lower()
    if te:
        raise ValueError("Transfer-Encoding is unsupported")
    values = handler.headers.get_all("Content-Length", [])
    if len(values) != 1 or not values[0].strip().isdigit():
        raise ValueError("exactly one nonnegative Content-Length is required")
    n = int(values[0].strip())
    if n > MAX_WAV_BYTES:
        raise ValueError("WAV request exceeds configured size limit")
    body = handler.rfile.read(n) if n else b""
    if len(body) != n:
        raise ValueError("truncated request body")
    return body


class BoundedHTTPServer(ThreadingHTTPServer):
    admission = threading.BoundedSemaphore(4)
    def process_request(self, request, client_address):
        if not self.admission.acquire(blocking=False):
            request.close()
            return
        t = threading.Thread(target=self._bounded_request, args=(request, client_address), daemon=True)
        try:
            t.start()
        except Exception:
            self.admission.release()
            request.close()
            raise
    def _bounded_request(self, request, client_address):
        try: self.process_request_thread(request, client_address)
        finally: self.admission.release()


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(10)

    def log_message(self, fmt, *args):  # quiet
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self._json(200, {"ok": RECOGNIZER is not None, "backend": "parakeet", "model": "nemo_transducer", "model_directory": MODEL_DIRECTORY, "protocol_version": 2})
        else:
            self._json(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/inference":
            self._json(404, {"error": "not found"})
            return
        if not INFERENCE_LOCK.acquire(blocking=False):
            self._json(429, {"error": "inference busy"})
            return
        self.connection.settimeout(10)
        try:
            body = read_body(self)
            audio = wav_bytes_to_f32(body)
            if audio.size < 800:  # <50 ms
                self._json(200, {"text": ""})
                return
            # Serialize expensive work and bound memory under client retries.
            stream = RECOGNIZER.create_stream()
            stream.accept_waveform(16000, audio)
            RECOGNIZER.decode_stream(stream)
            self._json(200, {"text": stream.result.text.strip()})
        except Exception as e:  # noqa: BLE001 - report to engine, never crash shim
            self._json(500, {"error": str(e)})
        finally:
            INFERENCE_LOCK.release()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", required=True)
    ap.add_argument("--port", type=int, default=9293)
    args = ap.parse_args()

    global RECOGNIZER, MODEL_DIRECTORY
    MODEL_DIRECTORY = os.path.normcase(os.path.realpath(args.models))
    try:
        RECOGNIZER = build_recognizer(args.models)
    except Exception as e:  # noqa: BLE001
        print(f"[parakeet-shim] MODEL LOAD FAILED: {e}", flush=True)
        sys.exit(3)  # engine treats nonzero exit as actionable drop-list error
    print(f"[parakeet-shim] model loaded, serving 127.0.0.1:{args.port}", flush=True)

    srv = BoundedHTTPServer(("127.0.0.1", args.port), Handler)
    srv.serve_forever()


if __name__ == "__main__":
    main()
