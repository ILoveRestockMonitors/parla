"""Add application, attribution, and an exact installed-file manifest to a prepared payload."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
PAYLOAD = ROOT / "release/bundle-payload"
VERSION = (ROOT / "VERSION").read_text(encoding="utf-8").strip()


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    shutil.copy2(ROOT / "release" / VERSION / "parla.exe", PAYLOAD / "parla.exe")
    shutil.copy2(ROOT / "docs/installer-welcome.txt", PAYLOAD / "INSTALL.txt")
    licenses = PAYLOAD / "licenses"
    licenses.mkdir(exist_ok=True)
    subprocess.run([__import__("sys").executable, str(ROOT / "scripts/collect-rust-notices.py"),
                    "--output", str(licenses / "Rust-dependencies.txt")], check=True)
    extra = {
        "Whisper-model-MIT.txt": "https://raw.githubusercontent.com/openai/whisper/86098128c0b4f24f0e2aa2994de830614b474227/LICENSE",
        "Sherpa-onnx-Apache-2.0.txt": "https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/917bed95c8e5c7c18aa4d69fea42e9ef8ef0a60e/LICENSE",
        "Parakeet-CC-BY-4.0.txt": "https://creativecommons.org/licenses/by/4.0/legalcode.txt",
    }
    for name, url in extra.items():
        if not (licenses / name).exists():
            subprocess.run(["curl.exe", "--location", "--fail", "--silent", "--show-error", "--max-time", "45", "--output", str(licenses / name), url], check=True)
    attribution = """Parla bundled speech tools and models

Parakeet TDT 0.6B v2: Copyright NVIDIA. Licensed under CC BY 4.0.
Original: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
License: https://creativecommons.org/licenses/by/4.0/
This distribution uses k2-fsa/sherpa-onnx's ONNX conversion and INT8 quantization.
Those are modifications of the original model; Parla does not further modify the weights.
Conversion: https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models

Whisper small: Copyright OpenAI, MIT license; ggml conversion by ggerganov/whisper.cpp.
https://github.com/openai/whisper
https://huggingface.co/ggerganov/whisper.cpp
whisper.cpp v1.8.7: MIT license, compiled here from official revision
48f628a84833905ee4a0658ee6d4a5c915ce1997 with static GNU runtime linkage and a portable CPU baseline.

Python 3.12.10: Python Software Foundation license, runtime/python/LICENSE.txt.
sherpa-onnx 1.13.7 and core: Apache-2.0; notices inside runtime/python/Lib/site-packages.
NumPy 2.4.6 and its bundled libraries: license notices inside their dist-info/licenses folder.
Rust and dependency notices: licenses/Rust-dependencies.txt (includes GNU runtime exception text).
Installer built with Inno Setup 6.6.1, Copyright Jordan Russell and Martijn Laan.
https://jrsoftware.org/

Ollama and language models for optional Polished cleanup are not bundled.
"""
    (PAYLOAD / "THIRD-PARTY-NOTICES.txt").write_text(attribution, encoding="utf-8")
    files = []
    for path in sorted(PAYLOAD.rglob("*")):
        if path.is_file() and path.name != "bundle-manifest.json":
            if "__pycache__" in path.parts or path.suffix == ".pyc":
                continue  # Development caches are preserved locally and excluded by Inno Setup.
            files.append({"path":path.relative_to(PAYLOAD).as_posix(),"bytes":path.stat().st_size,"sha256":sha(path)})
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    manifest = {"version":VERSION,"source_commit":revision,"platform":"windows-x64","default_engine":"parakeet",
                "default_cleanup":"faithful","ollama_included":False,
                "whisper_source_revision":"48f628a84833905ee4a0658ee6d4a5c915ce1997",
                "downloads":json.loads((ROOT / "scripts/bundle-sources.json").read_text()),
                "files":files}
    (PAYLOAD / "bundle-manifest.json").write_text(json.dumps(manifest, indent=2)+"\n", encoding="utf-8")
    print(f"Manifest: {len(files)} files, {sum(item['bytes'] for item in files):,} bytes")


if __name__ == "__main__":
    main()
