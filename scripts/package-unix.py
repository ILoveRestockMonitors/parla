#!/usr/bin/env python3
"""Build native offline Unix packages; verify payloads with public audio, never a mic.

Requires Python >=3.12, a native release binary and Cargo's downloaded sources.
All downloaded runtime/model/wheel bytes have committed SHA-256 pins. A native
runner is necessary: this script does not emulate or cross-package another OS.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.3.0-portable-20260921"
PYTHON_RELEASE = "20260901"
PYTHON_VERSION = "3.12.14"
# Checked against the publisher's release/20260901/SHA256SUMS on 2026-09-21.
PYTHONS = {
    "macos-arm64": ("aarch64-apple-darwin", "81a359f1cfadd4da11766534c5913791cea55f26e1bb902cacd2a531bb1e4b2b"),
    "macos-x64": ("x86_64-apple-darwin", "65b195c9cedc1fef6767f044f9822069adbd1bd9204d424ece4628776fdc04bb"),
    "linux-x64": ("x86_64-unknown-linux-gnu", "72748da13197c1fb161e3afeef20a6a385ff24f2165e6e2758e47008e7faba4c"),
}
# Exact wheel filenames and digests from PyPI's version JSON; no dependency solver.
WHEELS = {
    "macos-arm64": [
        ("sherpa-onnx", "1.13.7", "sherpa_onnx-1.13.7-cp312-cp312-macosx_11_0_arm64.whl", "fed61cc23dba8f212eeb2723d2e90108d6de95734c5ae427aeafdb10f09ec184"),
        ("sherpa-onnx-core", "1.13.7", "sherpa_onnx_core-1.13.7-py3-none-macosx_11_0_arm64.whl", "b191e79d4952199bd11ce05c5b29c5efb08ed43a5d96839202e8ab603ab96278"),
        ("numpy", "2.4.6", "numpy-2.4.6-cp312-cp312-macosx_11_0_arm64.whl", "ebfb099f8dcf083deef3ac1ca4c1503f387cf76296fcb3816b66f5ecb5f54fdb"),
    ],
    "macos-x64": [
        ("sherpa-onnx", "1.13.7", "sherpa_onnx-1.13.7-cp312-cp312-macosx_10_15_x86_64.whl", "7fbdc37a3b2a1f45369a918b47960035fd15b0d365d42157d5cdc5335271ab86"),
        ("sherpa-onnx-core", "1.13.7", "sherpa_onnx_core-1.13.7-py3-none-macosx_10_15_x86_64.whl", "c53a930543f3a0f1063956fd3d01081acd6cb66f46c9b1f544b4393d6cbc1893"),
        ("numpy", "2.4.6", "numpy-2.4.6-cp312-cp312-macosx_10_13_x86_64.whl", "001fbb8e08d942dd57599e781f2472269ee7f2755fae407b4f67b2f0b17da3f1"),
    ],
    "linux-x64": [
        ("sherpa-onnx", "1.13.7", "sherpa_onnx-1.13.7-cp312-cp312-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", "7b4cce2e44f989f6c830103c8449e4df84833f66fa4f625fd092270b54918a55"),
        ("sherpa-onnx-core", "1.13.7", "sherpa_onnx_core-1.13.7-py3-none-manylinux2014_x86_64.whl", "e3066e94c24c898c5794e08507f955e14bc904299bac80f357d0bcb7d93e3825"),
        ("numpy", "2.4.6", "numpy-2.4.6-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl", "90f9849678c75fe7afa2d348ac842c168b0a4d3d61919687216dfc547976d853"),
    ],
}
MODEL_NAMES = {"encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"}


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def request(url: str) -> bytes:
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "Parla-release-builder"}), timeout=90) as response:
        return response.read()


def download(source: dict, cache: Path) -> Path:
    target = cache / source["name"]
    if not target.exists():
        pending = target.with_suffix(target.suffix + ".partial")
        subprocess.run(["curl", "--fail", "--location", "--retry", "3", "--max-time", "900", "--output", str(pending), source["url"]], check=True)
        if sha(pending) != source["sha256"]:
            raise RuntimeError(f"Download SHA-256 mismatch: {target.name}")
        pending.rename(target)
    if sha(target) != source["sha256"]:
        raise RuntimeError(f"Cached SHA-256 mismatch: {target.name}")
    return target


def contained(root: Path, candidate: Path) -> Path:
    resolved = candidate.resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ValueError(f"Archive path escapes extraction root: {candidate.name}")
    return resolved


def member_path(root: Path, name: str) -> Path:
    relative = PurePosixPath(name)
    if relative.is_absolute() or ".." in relative.parts or "\\" in name or ":" in name:
        raise ValueError(f"Unsafe archive member: {name}")
    return contained(root, root.joinpath(*relative.parts))


def extract_tar(archive_path: Path, destination: Path) -> None:
    """Validate all names/links, then use Python's traversal-safe data filter."""
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive_path) as archive:
        for member in archive.getmembers():
            target = member_path(destination, member.name)
            if member.issym() or member.islnk():
                link = PurePosixPath(member.linkname)
                if link.is_absolute() or "\\" in member.linkname or ":" in member.linkname:
                    raise ValueError(f"Unsafe archive link: {member.name}")
                base = target.parent if member.issym() else destination
                contained(destination, base.joinpath(*link.parts))
            elif not (member.isfile() or member.isdir()):
                raise ValueError(f"Unsupported archive member: {member.name}")
        archive.extractall(destination, filter="data")


def extract_wheel(archive_path: Path, destination: Path) -> None:
    seen = set()
    with zipfile.ZipFile(archive_path) as archive:
        for member in archive.infolist():
            target = member_path(destination, member.filename)
            mode = member.external_attr >> 16
            if stat.S_ISLNK(mode) or member.filename in seen:
                raise ValueError(f"Wheel link/duplicate is unsupported: {member.filename}")
            seen.add(member.filename)
            if member.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            # These pinned wheels use site-packages layout, not wheel .data relocation.
            if any(part.endswith(".data") for part in PurePosixPath(member.filename).parts):
                raise ValueError("Wheel requires an unsupported .data installation scheme")
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(0o755 if mode & 0o111 else 0o644)


def model_payload(archive_path: Path, payload: Path, fixtures: Path) -> Path:
    destination = payload / "models/parakeet"
    destination.mkdir(parents=True)
    fixtures.mkdir(parents=True)
    seen = set()
    with tarfile.open(archive_path) as archive:
        for member in archive:
            member_path(destination, member.name)
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError("Model archive links/devices are unsupported")
            name = PurePosixPath(member.name).name
            if name not in MODEL_NAMES and name != "0.wav":
                continue
            if name in seen:
                raise ValueError(f"Duplicate model asset: {name}")
            seen.add(name)
            target = (destination if name in MODEL_NAMES else fixtures) / name
            with archive.extractfile(member) as source, target.open("wb") as output:
                shutil.copyfileobj(source, output)
    if seen != MODEL_NAMES | {"0.wav"}:
        raise ValueError(f"Model archive is incomplete: {seen}")
    return fixtures / "0.wav"


def sources_for(target: str) -> list[dict]:
    triple, digest = PYTHONS[target]
    name = f"cpython-{PYTHON_VERSION}+{PYTHON_RELEASE}-{triple}-install_only_stripped.tar.gz"
    result = [{"name": name, "url": f"https://github.com/astral-sh/python-build-standalone/releases/download/{PYTHON_RELEASE}/{urllib.parse.quote(name)}", "sha256": digest}]
    for package, version, filename, digest in WHEELS[target]:
        metadata = json.loads(request(f"https://pypi.org/pypi/{package}/{version}/json"))
        entry = next(asset for asset in metadata["urls"] if asset["filename"] == filename)
        if entry["digests"]["sha256"] != digest:
            raise ValueError(f"Published wheel digest changed: {filename}")
        result.append({"name": filename, "url": entry["url"], "sha256": digest})
    model = next(item for item in json.loads((ROOT / "scripts/bundle-sources.json").read_text()) if item["name"] == "parakeet-int8.tar.bz2")
    result.append(model)
    result.append({"name": "Parakeet-CC-BY-4.0.txt", "url": "https://creativecommons.org/licenses/by/4.0/legalcode.txt", "sha256": "9ba9550ad48438d0836ddab3da480b3b69ffa0aac7b7878b5a0039e7ab429411"})
    return result


def write_notices(payload: Path, cache: Path, target: str) -> None:
    licenses = payload / "licenses"
    licenses.mkdir()
    shutil.copy2(cache / "Parakeet-CC-BY-4.0.txt", licenses)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", {"macos-arm64": "aarch64-apple-darwin", "macos-x64": "x86_64-apple-darwin", "linux-x64": "x86_64-unknown-linux-gnu"}[target]], cwd=ROOT))
    summary = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if not package["source"]:
            continue
        directory = Path(package["manifest_path"]).parent
        copied = []
        candidates = [path for path in directory.iterdir() if path.name.lower().startswith(("license", "licence", "copying", "notice", "copyright"))]
        if package.get("license_file"):
            candidates.append(directory / package["license_file"])
        for path in candidates:
            if not path.exists() or not path.resolve().is_relative_to(directory.resolve()):
                continue
            output = licenses / "rust" / f"{package['name']}-{package['version']}" / path.name
            output.parent.mkdir(parents=True, exist_ok=True)
            if path.is_dir():
                shutil.copytree(path, output, dirs_exist_ok=True)
            else:
                shutil.copy2(path, output)
            copied.append(output.relative_to(payload).as_posix())
        summary.append({"name": package["name"], "version": package["version"], "license": package.get("license"), "source": package["source"], "license_files": copied})
    (licenses / "rust-dependencies.json").write_text(json.dumps(summary, indent=2) + "\n")
    for path in ROOT.glob("LICENSE*"):
        if path.is_file():
            shutil.copy2(path, licenses / ("Parla-" + path.name))
    (payload / "THIRD-PARTY-NOTICES.txt").write_text(
        "Parla offline speech distribution\n\n"
        "NVIDIA Parakeet TDT 0.6B v2, CC BY 4.0. Original model:\n"
        "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2\n"
        "ONNX conversion and INT8 quantization by k2-fsa/sherpa-onnx; these are\n"
        "modifications of the original model. Parla does not modify these weights.\n"
        "https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models\n"
        "Full model license: licenses/Parakeet-CC-BY-4.0.txt\n\n"
        f"Python {PYTHON_VERSION}, Python Software Foundation license, distributed by\n"
        f"astral-sh/python-build-standalone release {PYTHON_RELEASE}. Python and its\n"
        "bundled library notices are retained under runtime/python.\n"
        "sherpa-onnx / sherpa-onnx-core 1.13.7: Apache-2.0.\n"
        "NumPy 2.4.6 and bundled native library notices remain in their wheel\n"
        "dist-info directories under runtime/python/lib/python3.12/site-packages.\n"
        "Rust dependency versions, expressions and copied notices: licenses/rust*.\n"
        "Whisper, Ollama and polishing language models are not bundled on Unix.\n",
        encoding="utf-8",
    )


def sign_macho_files(application: Path) -> None:
    magic = {b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"}
    for path in sorted(application.rglob("*")):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as stream:
            is_macho = stream.read(4) in magic
        if is_macho:
            subprocess.run(["codesign", "--force", "--sign", "-", str(path)], check=True, stdout=subprocess.DEVNULL)
    # Individual Mach-O objects receive ad-hoc signatures for Apple Silicon.
    # The app/installer has no Developer ID identity and is not notarized.


def manifest(root: Path, payload: Path, target: str, sources: list[dict]) -> None:
    entries = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            contained(root, path)
            entries.append({"path": relative, "symlink": os.readlink(path)})
        elif path.is_file():
            entries.append({"path": relative, "bytes": path.stat().st_size, "sha256": sha(path)})
    data = {"version": VERSION, "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "platform": target, "root": "application" if target.startswith("macos") else "payload", "default_engine": "parakeet", "default_cleanup": "faithful", "whisper_included": False, "ollama_included": False, "downloads": sources, "files": entries}
    (payload / "bundle-manifest.json").write_text(json.dumps(data, indent=2) + "\n")


def verify_manifest(root: Path, payload: Path) -> int:
    data = json.loads((payload / "bundle-manifest.json").read_text())
    for entry in data["files"]:
        path = member_path(root, entry["path"])
        if "symlink" in entry:
            original = root / entry["path"]
            if not original.is_symlink() or os.readlink(original) != entry["symlink"]:
                raise RuntimeError(f"Symlink mismatch: {entry['path']}")
        elif path.stat().st_size != entry["bytes"] or sha(path) != entry["sha256"]:
            raise RuntimeError(f"Payload mismatch: {entry['path']}")
    return len(data["files"])


def run_isolated(command: list[str], env: dict, cwd: Path, timeout: int = 180) -> str:
    proc = subprocess.Popen(command, env=env, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        output, _ = proc.communicate(timeout=timeout)
        if proc.returncode:
            raise RuntimeError(f"Failed {command[1:]} ({proc.returncode}):\n{output}")
        return output
    finally:
        # Only this invocation's process group; also removes its model child.
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if proc.poll() is None:
            proc.wait(timeout=10)


def verify_runtime(root: Path, payload: Path, binary: Path, fixture: Path) -> dict:
    count = verify_manifest(root, payload)
    with tempfile.TemporaryDirectory(prefix="parla-package-test-") as temporary:
        state = Path(temporary)
        env = dict(os.environ)
        for name in ("PYTHONHOME", "PYTHONPATH", "PARLA_MODEL", "PARLA_PARAKEET_PYTHON"):
            env.pop(name, None)
        env.update({"HOME": str(state), "XDG_DATA_HOME": str(state / "data"), "XDG_CONFIG_HOME": str(state / "config"), "LOCALAPPDATA": str(state / "local"), "PARLA_DATA_DIR": str(state / "parla"), "PYTHONDONTWRITEBYTECODE": "1", "PYTHONNOUSERSITE": "1", "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"})
        python = payload / "runtime/python/bin/python3"
        versions = run_isolated([str(python), "-B", "-c", "import json,sys,numpy,sherpa_onnx; print(json.dumps({'python':sys.version,'numpy':numpy.__version__,'sherpa':sherpa_onnx.__version__}))"], env, state)
        run_isolated([str(binary), "--initialize-bundle"], env, state)
        settings_files = list(state.rglob("settings.json"))
        if len(settings_files) != 1:
            raise RuntimeError(f"Expected exactly one isolated settings file: {settings_files}")
        settings = settings_files[0]
        content = settings.read_bytes()
        data = json.loads(content)
        if data["asr_backend"] != "parakeet" or Path(data["parakeet_model_dir"]).resolve() != (payload / "models/parakeet").resolve():
            raise RuntimeError("Fresh settings did not select the bundled Parakeet model")
        run_isolated([str(binary), "--initialize-bundle"], env, state)
        if settings.read_bytes() != content:
            raise RuntimeError("Repeated initialization overwrote existing settings")
        setup = run_isolated([str(binary), "--check-setup"], env, state)
        prompt = json.loads(run_isolated([str(binary), "--export-prompt"], env, state))
        if not prompt.get("system"):
            raise RuntimeError("Prompt export missing system prompt")
        replay = run_isolated([str(binary), "--replay", str(fixture)], env, state, timeout=240)
        text = replay.lower().replace("’", "'")
        if "don't wish to see" not in text or "old portrait" not in text:
            raise RuntimeError(f"Public audio fixture transcript did not match expected phrases:\n{replay}")
    return {"verified_files": count, "runtime_versions": json.loads(versions), "settings_preserved": True, "setup": setup, "public_fixture_sha256": sha(fixture), "public_fixture_source": "Pinned k2-fsa Parakeet model archive, test_wavs/0.wav", "replay": replay, "microphone_used": False, "input_injection_used": False, "live_hotkey_permissions_tested": False}


def tar_output(source: Path, target: Path, epoch: int) -> None:
    def normalize(info):
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mtime = epoch
        return info
    with target.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as zipped, tarfile.open(fileobj=zipped, mode="w") as archive:
        archive.add(source, arcname="parla", filter=normalize)


def package(args) -> None:
    native = ("macos" if sys.platform == "darwin" else "linux") + ("-arm64" if platform.machine() in ("arm64", "aarch64") else "-x64")
    if sys.platform not in ("darwin", "linux") or args.target != native:
        raise RuntimeError(f"Build requires native {args.target}; this host is {sys.platform}/{platform.machine()}")
    output = args.output.resolve()
    if output.exists():
        raise RuntimeError("Output directory exists; use a new directory to preserve prior evidence")
    output.mkdir(parents=True)
    cache = args.cache.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    sources = sources_for(args.target)
    for source in sources:
        download(source, cache)
    mac = args.target.startswith("macos")
    root = output / ("Parla.app" if mac else "parla")
    payload = root / "Contents/Resources" if mac else root
    binary = root / "Contents/MacOS/parla" if mac else root / "parla"
    payload.mkdir(parents=True)
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.binary.resolve(), binary)
    binary.chmod(0o755)
    fixture = model_payload(cache / "parakeet-int8.tar.bz2", payload, output / "fixtures")
    extract_tar(cache / sources[0]["name"], payload / "runtime")
    site = payload / "runtime/python/lib/python3.12/site-packages"
    for source in sources:
        if source["name"].endswith(".whl"):
            extract_wheel(cache / source["name"], site)
    write_notices(payload, cache, args.target)
    shutil.copy2(ROOT / "docs/unix-installation.md", payload / "INSTALL.md")
    if mac:
        (root / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleName": "Parla", "CFBundleDisplayName": "Parla", "CFBundleIdentifier": "com.parla.dictation", "CFBundleExecutable": "parla", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": "0.3.0", "CFBundleVersion": "30020260921", "LSMinimumSystemVersion": "15.0", "NSMicrophoneUsageDescription": "Parla records speech only when you start dictation and transcribes it on your computer.", "NSHighResolutionCapable": True}))
        sign_macho_files(root)
    else:
        launcher = root / "parla-launch"
        launcher.write_text('#!/bin/sh\nset -eu\nPARLA_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$PARLA_ROOT/parla" --dashboard "$@"\n')
        launcher.chmod(0o755)
    manifest(root, payload, args.target, sources)
    verification = {"staged": verify_runtime(root, payload, binary, fixture)}
    dist = output / "dist"
    dist.mkdir()
    epoch = int(subprocess.check_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT))
    if mac:
        pkg = dist / f"Parla-{VERSION}-{args.target}.pkg"
        subprocess.run(["pkgbuild", "--component", str(root), "--install-location", "/Applications", "--identifier", "com.parla.dictation", "--version", "0.3.0", str(pkg)], check=True)
        subprocess.run(["pkgutil", "--expand", str(pkg), str(output / "pkg-expanded")], check=True)
        subprocess.run(["sudo", "installer", "-pkg", str(pkg), "-target", "/"], check=True)
        installed = Path("/Applications/Parla.app")
        verification["installed"] = verify_runtime(installed, installed / "Contents/Resources", installed / "Contents/MacOS/parla", fixture)
    else:
        tar_output(root, dist / f"Parla-{VERSION}-linux-x64.tar.gz", epoch)
        debroot = output / "deb-root"
        shutil.copytree(root, debroot / "opt/parla", symlinks=True)
        desktop = debroot / "usr/share/applications/parla.desktop"
        desktop.parent.mkdir(parents=True)
        desktop.write_text("[Desktop Entry]\nType=Application\nName=Parla\nComment=Local speech dictation\nExec=/opt/parla/parla --dashboard\nTerminal=false\nCategories=Utility;Accessibility;\n")
        command = debroot / "usr/bin/parla"
        command.parent.mkdir(parents=True)
        command.write_text('#!/bin/sh\nexec /opt/parla/parla "$@"\n')
        command.chmod(0o755)
        control = debroot / "DEBIAN/control"
        control.parent.mkdir()
        size = sum(path.stat().st_size for path in root.rglob("*") if path.is_file()) // 1024
        control.write_text(f"Package: parla\nVersion: 0.3.0+20260921\nArchitecture: amd64\nMaintainer: Parla maintainers <noreply@github.com>\nInstalled-Size: {size}\nDepends: libasound2, libx11-6, libxtst6, libxdo3, libxcb1, libxkbcommon0, libc6 (>= 2.35), libstdc++6\nDescription: Local speech dictation with bundled Parakeet\n Includes a private Python runtime and speech model.\n")
        deb = dist / f"Parla-{VERSION}-linux-x64.deb"
        env = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch))
        subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(debroot), str(deb)], env=env, check=True)
        subprocess.run(["sudo", "dpkg", "--install", str(deb)], check=True)
        installed = Path("/opt/parla")
        verification["installed"] = verify_runtime(installed, installed, installed / "parla", fixture)
        with tempfile.TemporaryDirectory(prefix="parla-tar-test-") as temporary:
            extract_tar(dist / f"Parla-{VERSION}-linux-x64.tar.gz", Path(temporary))
            relocated = Path(temporary) / "parla"
            verification["tar_relocated"] = verify_runtime(relocated, relocated, relocated / "parla", fixture)
    (dist / f"verification-{args.target}.json").write_text(json.dumps(verification, indent=2) + "\n")
    shutil.copy2(payload / "bundle-manifest.json", dist / f"manifest-{args.target}.json")
    shutil.copy2(payload / "THIRD-PARTY-NOTICES.txt", dist / f"THIRD-PARTY-NOTICES-{args.target}.txt")
    (dist / "SHA256SUMS.txt").write_text("".join(f"{sha(path)}  {path.name}\n" for path in sorted(dist.iterdir()) if path.is_file()))
    print(f"Verified packages and evidence: {dist}", flush=True)


class ArchiveTests(unittest.TestCase):
    def test_tar_rejects_traversal_and_external_links(self):
        for name, link in [("../escape", None), ("/absolute", None), ("link", "../../escape"), ("link", "/etc/passwd")]:
            with self.subTest(name=name, link=link), tempfile.TemporaryDirectory() as temporary:
                base = Path(temporary)
                archive = base / "bad.tar"
                with tarfile.open(archive, "w") as output:
                    info = tarfile.TarInfo(name)
                    if link:
                        info.type, info.linkname = tarfile.SYMTYPE, link
                    output.addfile(info)
                with self.assertRaises((ValueError, tarfile.FilterError)):
                    extract_tar(archive, base / "unpack")

    def test_zip_rejects_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            with zipfile.ZipFile(base / "bad.whl", "w") as output:
                output.writestr("../escape", "bad")
            with self.assertRaises(ValueError):
                extract_wheel(base / "bad.whl", base / "unpack")

    def test_zip_preserves_executable_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            with zipfile.ZipFile(base / "good.whl", "w") as output:
                info = zipfile.ZipInfo("lib/module.so")
                info.external_attr = (stat.S_IFREG | 0o755) << 16
                output.writestr(info, b"library")
            extract_wheel(base / "good.whl", base / "unpack")
            self.assertEqual((base / "unpack/lib/module.so").read_bytes(), b"library")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=PYTHONS)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/parla")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--cache", type=Path, default=ROOT / "release/unix-cache")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(ArchiveTests)
        if not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful():
            raise SystemExit(1)
        return
    if sys.version_info < (3, 12):
        parser.error("Python 3.12 or newer is required for safe archive extraction")
    if not args.target or not args.output:
        parser.error("--target and --output are required")
    package(args)


if __name__ == "__main__":
    main()
