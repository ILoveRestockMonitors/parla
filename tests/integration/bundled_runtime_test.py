"""Exercise packaged runtimes on a public fixture with isolated settings and localhost ports."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]


def request(url, data=None, content_type=None):
    headers = {"Content-Type": content_type} if content_type else {}
    with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers), timeout=90) as response:
        return json.load(response)


def engine_test(command, payload, env, whisper=False):
    with socket.socket() as port_probe:
        port_probe.bind(("127.0.0.1", 0))
        port = port_probe.getsockname()[1]
    with tempfile.TemporaryFile() as log:
        process = subprocess.Popen([*command, "--port", str(port)], env=env,
                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                   creationflags=0x08000000)
        try:
            deadline = time.monotonic() + 80
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(log.read().decode("utf-8", errors="replace"))
                try:
                    health = request(f"http://127.0.0.1:{port}/health")
                    if health.get("ok") or health.get("status") == "ok":
                        break
                except Exception:
                    pass
                time.sleep(0.2)
            else:
                raise AssertionError("Packaged engine did not become ready")
            if whisper:
                boundary = "ParlaBundleVerification"
                body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="fixture.wav"\r\nContent-Type: audio/wav\r\n\r\n'.encode()
                        + payload + f'\r\n--{boundary}--\r\n'.encode())
                result = request(f"http://127.0.0.1:{port}/inference", body, f"multipart/form-data; boundary={boundary}")
            else:
                result = request(f"http://127.0.0.1:{port}/inference", payload, "audio/wav")
            text = result["text"].strip()
            assert len(text) > 80 and "observed" in text.lower() and "eyes" in text.lower(), text
            return text
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("bundle", type=Path)
    parser.add_argument("fixture", type=Path)
    args = parser.parse_args()
    bundle = args.bundle.resolve()
    binary = bundle / "parla.exe"
    with tempfile.TemporaryDirectory(prefix="parla-bundled-runtime-") as tmp:
        env = dict(os.environ, LOCALAPPDATA=tmp, APPDATA=tmp,
                   PATH=str(Path(os.environ["SystemRoot"]) / "System32"))
        for name in ("PYTHONHOME", "PYTHONPATH", "PYTHON", "PARLA_PARAKEET_PYTHON", "PARLA_MODEL", "PARLA_PARAKEET_GPU"):
            env.pop(name, None)
        def run(*command):
            return subprocess.run(command, env=env, capture_output=True, text=True,
                                  encoding="utf-8", timeout=30, creationflags=0x08000000, check=True)
        run(str(binary), "--initialize-bundle")
        settings_path = Path(tmp) / "Parla/settings.json"
        settings = json.loads(settings_path.read_text())
        assert settings["asr_backend"] == "parakeet"
        assert settings["cleanup_mode"] == "faithful"
        assert Path(settings["asr_model_path"]) == bundle / "models/whisper/ggml-small.bin"
        before = settings_path.read_bytes()
        run(str(binary), "--initialize-bundle")
        assert settings_path.read_bytes() == before
        setup = json.loads(run(str(binary), "--check-setup").stdout)
        rows = {item["id"]:item for item in setup["items"]}
        assert rows["python"]["state"] == "found", rows["python"]
        assert Path(rows["python"]["path"]) == bundle / "runtime/python/python.exe"
        assert rows["sherpa"]["state"] == "ready", rows["sherpa"]
        assert rows["parakeet-model"]["state"] == "found"
        assert not rows["ollama"]["required"]
        fixture = args.fixture.read_bytes()
        parakeet = engine_test([str(bundle / "runtime/python/python.exe"), "-B",
            str(ROOT / "src-tauri/assets/parakeet-shim.py"), "--models", str(bundle / "models/parakeet")], fixture, env)
        whisper = engine_test([str(bundle / "engines/whisper/whisper-server.exe"), "--host", "127.0.0.1",
            "-m", str(bundle / "models/whisper/ggml-small.bin"), "-l", "en", "-t", "4"], fixture, env, whisper=True)
        print(json.dumps({"settings_initialization":"passed","existing_settings_preserved":True,
            "private_python_and_packages":"passed","system_path_only":True,"microphone_or_typing_used":False,
            "parakeet_transcript":parakeet,"whisper_transcript":whisper}, indent=2))


if __name__ == "__main__":
    main()
