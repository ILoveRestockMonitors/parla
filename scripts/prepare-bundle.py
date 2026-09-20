"""Download pinned official runtime/model assets and assemble an offline Windows payload."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / "release/bundle-cache"
PAYLOAD = ROOT / "release/bundle-payload"
SOURCES = json.loads((ROOT / "scripts/bundle-sources.json").read_text())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def download(source):
    target = CACHE / source["name"]
    if not target.exists():
        pending = target.with_name(target.name + ".partial")
        subprocess.run(["curl.exe", "--location", "--fail", "--silent", "--show-error",
                        "--retry", "3", "--max-time", "600", "--output", str(pending),
                        source["url"] + ("?download=true" if "huggingface.co" in source["url"] else "")], check=True)
        if digest(pending) != source["sha256"]:
            raise RuntimeError(f"Download hash mismatch: {source['name']}")
        pending.rename(target)
    if digest(target) != source["sha256"]:
        raise RuntimeError(f"Cached asset hash mismatch: {source['name']}")
    print(f"Verified {source['name']} ({target.stat().st_size:,} bytes)", flush=True)


def main():
    CACHE.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(download, SOURCES))
    if PAYLOAD.exists():
        raise RuntimeError("Payload already exists; preserve it or choose a new output before assembling")
    PAYLOAD.mkdir(parents=True)
    whisper = PAYLOAD / "engines/whisper"
    whisper.mkdir(parents=True)
    shutil.copy2(ROOT / "release/whisper-build/bin/whisper-server.exe", whisper / "whisper-server.exe")
    shutil.copy2(ROOT / "release/whisper-source/LICENSE", whisper / "LICENSE.txt")
    if not (whisper / "whisper-server.exe").is_file():
        raise RuntimeError("Whisper distribution lacks the required server")
    models = PAYLOAD / "models"
    (models / "whisper").mkdir(parents=True)
    shutil.copy2(CACHE / "ggml-small.bin", models / "whisper/ggml-small.bin")
    (models / "parakeet").mkdir()
    fixtures = ROOT / "release/bundle-fixtures"
    fixtures.mkdir(exist_ok=True)
    with tarfile.open(CACHE / "parakeet-int8.tar.bz2", "r:bz2") as archive:
        for member in archive:
            if not member.isfile():
                continue
            name = Path(member.name).name
            destination = models / "parakeet" if name.endswith(".onnx") or name == "tokens.txt" else fixtures
            with archive.extractfile(member) as source, (destination / name).open("wb") as output:
                shutil.copyfileobj(source, output)
    python = PAYLOAD / "runtime/python"
    python.mkdir(parents=True)
    with zipfile.ZipFile(CACHE / "python-3.12.10-embed-amd64.zip") as archive:
        archive.extractall(python)
    packages = python / "Lib/site-packages"
    packages.mkdir(parents=True)
    for source in SOURCES:
        if source["name"].endswith(".whl"):
            with zipfile.ZipFile(CACHE / source["name"]) as archive:
                archive.extractall(packages)
    (python / "python312._pth").write_text("python312.zip\n.\nLib/site-packages\nimport site\n", encoding="ascii")
    print(f"Assembled payload: {PAYLOAD}", flush=True)


if __name__ == "__main__":
    main()
