"""Hermetic release CLI checks; never starts microphone or model services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = Path(os.environ["PARLA_BIN"]).resolve()
with tempfile.TemporaryDirectory(prefix="parla-cli-smoke-") as tmp:
    env = dict(os.environ, LOCALAPPDATA=tmp)
    def run(*args):
        return subprocess.run([str(binary), *args], capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=10, env=env)
    result = run("--export-prompt")
    assert result.returncode == 0 and len(json.loads(result.stdout)["system"]) > 100
    bad_json = Path(tmp) / "bad.json"
    bad_json.write_text("{bad", encoding="utf-8")
    result = run("--format-json", str(bad_json))
    assert result.returncode == 1 and "JSON" in result.stderr, result.stderr
    bad_wav = Path(tmp) / "bad.wav"
    bad_wav.write_bytes(b"not a WAV")
    result = run("--replay", str(bad_wav))
    assert result.returncode == 2 and "RIFF" in result.stderr, result.stderr
print("3 CLI checks passed; no microphone, inference, or insertion used")
