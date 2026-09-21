#!/usr/bin/env python3
"""Attach verified native CI artifacts to an existing draft; never publish it.

Uses the runner's gh CLI and GH_TOKEN. There are no release edits, tag edits,
asset deletions, clobber uploads, dependency installs, or executed artifact code.
Run --self-test for hermetic rejection/idempotency checks.
"""
from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tempfile
import time
import unittest

REPOSITORY = "ILoveRestockMonitors/parla"
WORKFLOW_PATH = ".github/workflows/portable.yml"
WORKFLOW_NAME = "Portable offline packages"
PLATFORMS = ("linux-x64", "macos-arm64", "macos-x64")
HEX = re.compile(r"[0-9a-f]{64}\Z")
REVISION = re.compile(r"[0-9a-f]{40}\Z")
TAG = re.compile(r"v[0-9][A-Za-z0-9._-]{0,79}\Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def gh(*args: str) -> str:
    result = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
    if result.returncode:
        raise RuntimeError(f"gh {args[0]} failed: {result.stderr.strip()}")
    return result.stdout


def api(endpoint: str):
    return json.loads(gh("api", endpoint))


def pages(endpoint: str, field: str | None = None) -> list:
    data = json.loads(gh("api", "--paginate", "--slurp", endpoint))
    return [item for page in data for item in (page[field] if field else page)]


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def json_file(path: Path):
    require(path.stat().st_size <= 16 * 1024 * 1024, f"Oversized metadata: {path.name}")
    return json.loads(path.read_text(encoding="utf-8"))


def validate_run(run: dict, run_id: int, revision: str) -> None:
    require(run.get("id") == run_id, "Source run ID mismatch")
    require(run.get("status") == "completed" and run.get("conclusion") == "success", "Source run must have completed successfully")
    require(run.get("head_sha") == revision, "Source run revision mismatch")
    require(run.get("name") == WORKFLOW_NAME and run.get("path") == WORKFLOW_PATH, "Source run is not the native package workflow")
    require(run.get("repository", {}).get("full_name") == REPOSITORY, "Source repository mismatch")
    require(run.get("head_repository", {}).get("full_name") == REPOSITORY, "Fork artifacts are not accepted")
    require(run.get("event") in ("push", "workflow_dispatch"), "Pull request and other event artifacts are not accepted")


def validate_draft(release: dict, tag: str, revision: str, release_id: int | None = None) -> None:
    require(release.get("draft") is True, "Target release is no longer a draft")
    require(release.get("tag_name") == tag, "Target release tag mismatch")
    require(release.get("target_commitish") == revision, "Draft must target the exact verified source revision")
    require(isinstance(release.get("id"), int), "Target release ID missing")
    if release_id is not None:
        require(release["id"] == release_id, "Target release was replaced")


def validate_artifacts(artifacts: list, run_id: int, revision: str) -> list:
    expected = {f"parla-{platform}" for platform in PLATFORMS}
    require(len(artifacts) == len(expected) and {a.get("name") for a in artifacts} == expected, "Source run must contain exactly the three expected platform artifacts")
    for artifact in artifacts:
        require(artifact.get("expired") is False and artifact.get("size_in_bytes", 0) > 0, "An artifact is expired or empty")
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest", "")) is not None, "Artifact server digest is missing")
        origin = artifact.get("workflow_run", {})
        require(origin.get("id") == run_id and origin.get("head_sha") == revision, "Artifact source provenance mismatch")
    return sorted(artifacts, key=lambda a: a["name"])


def expected_files(platform: str, version: str) -> set[str]:
    files = {f"manifest-{platform}.json", f"verification-{platform}.json", f"THIRD-PARTY-NOTICES-{platform}.txt"}
    extensions = ("deb", "tar.gz") if platform == "linux-x64" else ("pkg",)
    return files | {f"Parla-{version}-{platform}.{extension}" for extension in extensions}


def validate_checksums(directory: Path, expected: set[str]) -> None:
    entries = list(directory.iterdir())
    require(all(p.is_file() and not p.is_symlink() for p in entries), "Artifact must contain ordinary files only")
    require({p.name for p in entries} == expected | {"SHA256SUMS.txt"}, "Unexpected or missing artifact files")
    checksums: dict[str, str] = {}
    for line in (directory / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})  ([^/\\\r\n]+)", line)
        require(match is not None, "Malformed checksum line")
        digest, name = match.groups()
        require(name in expected and name not in checksums, "Unexpected or duplicate checksum entry")
        checksums[name] = digest.lower()
    require(set(checksums) == expected, "Checksum list is incomplete")
    for name, digest in checksums.items():
        path = directory / name
        require(path.stat().st_size > 0 and sha(path) == digest, f"Checksum mismatch: {name}")


def validate_runtime(record: dict, manifest: dict, version: str, revision: str, label: str) -> str:
    require(isinstance(record, dict), f"Missing {label} runtime evidence")
    require(type(record.get("verified_files")) is int and record["verified_files"] == len(manifest["files"]), f"{label}: installed file verification incomplete")
    require(record.get("binary_version") == f"Parla {version} ({revision})", f"{label}: binary provenance mismatch")
    for key in ("stutter_correction_default", "settings_preserved"):
        require(record.get(key) is True, f"{label}: {key} was not verified")
    for key in ("microphone_used", "input_injection_used", "live_hotkey_permissions_tested"):
        require(record.get(key) is False, f"{label}: unexpected or missing {key} declaration")
    setup = record.get("setup", {})
    require(setup.get("engine") == "parakeet" and setup.get("cleanup_mode") == "faithful", f"{label}: incorrect fresh-install defaults")
    states = {item.get("id"): item.get("state") for item in setup.get("items", [])}
    require(all(states.get(k) == v for k, v in {"python": "found", "sherpa": "ready", "parakeet-model": "found"}.items()), f"{label}: installed dependencies not ready")
    versions = record.get("runtime_versions", {})
    require(all(isinstance(versions.get(k), str) and versions[k] for k in ("python", "numpy", "sherpa")), f"{label}: runtime version evidence missing")
    replay = record.get("replay", "").lower().replace("’", "'")
    require("don't wish to see" in replay and "old portrait" in replay, f"{label}: public audio replay did not pass")
    fixture = record.get("public_fixture_sha256", "")
    require(HEX.fullmatch(fixture) is not None, f"{label}: fixture digest missing")
    return fixture


def validate_platform(directory: Path, platform: str, version: str, revision: str) -> list[Path]:
    expected = expected_files(platform, version)
    validate_checksums(directory, expected)
    manifest = json_file(directory / f"manifest-{platform}.json")
    require(manifest.get("platform") == platform and manifest.get("version") == version and manifest.get("source_commit") == revision, "Manifest platform/version/source mismatch")
    require(manifest.get("default_engine") == "parakeet" and manifest.get("default_cleanup") == "faithful", "Manifest defaults mismatch")
    require(manifest.get("whisper_included") is False and manifest.get("ollama_included") is False, "Unexpected native package contents")
    files = manifest.get("files")
    require(isinstance(files, list) and bool(files), "Manifest file inventory is missing")
    paths = set()
    for entry in files:
        path = entry.get("path", "")
        require(isinstance(path, str) and bool(path) and "\\" not in path and not PurePosixPath(path).is_absolute() and ".." not in PurePosixPath(path).parts and path not in paths, "Manifest contains an unsafe or duplicate path")
        paths.add(path)
        if "symlink" not in entry:
            require(HEX.fullmatch(entry.get("sha256", "")) is not None and type(entry.get("bytes")) is int and entry["bytes"] >= 0, "Manifest file digest/size missing")
    require(("parla" if platform == "linux-x64" else "Contents/MacOS/parla") in paths, "Manifest does not identify the native binary")
    evidence = json_file(directory / f"verification-{platform}.json")
    records = ["staged", "installed"] + (["tar_relocated"] if platform == "linux-x64" else [])
    fixtures = {validate_runtime(evidence.get(key), manifest, version, revision, f"{platform}/{key}") for key in records}
    require(len(fixtures) == 1, "Staged/installed fixture digests disagree")
    if platform.startswith("macos"):
        receipt = evidence.get("macos_receipt", {})
        require(receipt.get("volume") == "/" and receipt.get("install-location", "").strip("/") == "Applications", "macOS installation destination was not verified")
    return [directory / name for name in sorted(expected)]


def asset_record(path: Path, platform: str | None, run_id: int, revision: str) -> dict:
    return {"name": path.name, "bytes": path.stat().st_size, "sha256": sha(path), "platform": platform, "source_run_id": run_id, "source_commit": revision}


def match_server_asset(asset: dict, expected: dict) -> None:
    require(asset.get("name") == expected["name"] and asset.get("state") == "uploaded", f"Release asset is not complete: {expected['name']}")
    require(asset.get("size") == expected["bytes"] and asset.get("digest") == f"sha256:{expected['sha256']}", f"Existing/uploaded asset differs; refusing overwrite: {expected['name']}")


def server_assets(release_id: int) -> dict:
    values = pages(f"repos/{REPOSITORY}/releases/{release_id}/assets?per_page=100")
    require(len({v["name"] for v in values}) == len(values), "Release contains duplicate asset names")
    return {value["name"]: value for value in values}


def attach(args) -> None:
    require(args.source_run > 0 and REVISION.fullmatch(args.expected_revision) is not None and TAG.fullmatch(args.tag) is not None, "Invalid run, full revision, or release tag")
    require(os.environ.get("GITHUB_REPOSITORY", REPOSITORY) == REPOSITORY, "Transfer must run in the Parla repository")
    revision, run_id, tag = args.expected_revision, args.source_run, args.tag
    run = api(f"repos/{REPOSITORY}/actions/runs/{run_id}")
    validate_run(run, run_id, revision)
    workflow = api(f"repos/{REPOSITORY}/actions/workflows/{run['workflow_id']}")
    require(workflow.get("path") == WORKFLOW_PATH and workflow.get("name") == WORKFLOW_NAME, "Workflow ID/path mismatch")
    version_file = api(f"repos/{REPOSITORY}/contents/VERSION?ref={revision}")
    require(version_file.get("encoding") == "base64", "VERSION API response missing content")
    version = base64.b64decode(version_file["content"]).decode("utf-8").strip()
    require(tag == f"v{version}", "Release tag does not match source VERSION")
    # GitHub's by-tag endpoint returns 404 for an unpublished draft whose Git
    # tag does not exist yet. Enumerate authenticated releases without creating
    # that tag or changing the draft's publication state.
    matching = [r for r in pages(f"repos/{REPOSITORY}/releases?per_page=100") if r.get("tag_name") == tag]
    require(len(matching) == 1, "Expected exactly one existing release with this tag")
    release = matching[0]
    validate_draft(release, tag, revision)
    release_id = release["id"]
    artifacts = validate_artifacts(pages(f"repos/{REPOSITORY}/actions/runs/{run_id}/artifacts?per_page=100", "artifacts"), run_id, revision)
    output = args.output.resolve()
    require(not output.exists(), "Output already exists; use a new directory")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="parla-native-transfer-") as temporary:
        root = Path(temporary)
        # Download all artifacts; validate_artifacts proved exactly three names.
        gh("run", "download", str(run_id), "--repo", REPOSITORY, "--dir", str(root))
        require({p.name for p in root.iterdir()} == {a["name"] for a in artifacts}, "Downloaded artifact directories mismatch")
        paths, records = [], []
        for platform in PLATFORMS:
            directory = root / f"parla-{platform}"
            require(directory.is_dir() and not directory.is_symlink(), "Unsafe artifact directory")
            for path in validate_platform(directory, platform, version, revision):
                paths.append(path)
                records.append(asset_record(path, platform, run_id, revision))
        require(len({r['name'] for r in records}) == len(records), "Native asset names collide")
        native_records = list(records)
        sums = output / "SHA256SUMS-native.txt"
        sums.write_text("".join(f"{r['sha256']}  {r['name']}\n" for r in sorted(records, key=lambda r: r['name'])), encoding="utf-8")
        paths.append(sums)
        records.append(asset_record(sums, None, run_id, revision))
        index = {"schema_version": 1, "repository": REPOSITORY, "source_run": run_id, "source_run_url": f"https://github.com/{REPOSITORY}/actions/runs/{run_id}", "workflow_path": WORKFLOW_PATH, "source_commit": revision, "version": version, "release_id": release_id, "release_tag": tag,
                 "artifacts": [{k: a[k] for k in ("id", "name", "size_in_bytes", "digest")} for a in artifacts], "assets": sorted(native_records, key=lambda r: r['name'])}
        index_path = output / "native-assets.json"
        index_path.write_text(json.dumps(index, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        # The index excludes itself and the aggregate checksum. The parent
        # compares both small files against their independent server digests.
        paths.append(index_path)
        records.append(asset_record(index_path, None, run_id, revision))
        existing = server_assets(release_id)
        for record in records:
            if record["name"] in existing:
                match_server_asset(existing[record["name"]], record)
        for path, record in zip(paths, records):
            validate_draft(api(f"repos/{REPOSITORY}/releases/{release_id}"), tag, revision, release_id)
            if record["name"] not in existing:
                # No --clobber. A race/collision fails, preserving existing bytes.
                gh("release", "upload", tag, str(path), "--repo", REPOSITORY)
            for attempt in range(4):
                actual = server_assets(release_id).get(record["name"])
                if actual is not None and actual.get("digest") is not None:
                    match_server_asset(actual, record)
                    break
                if attempt == 3:
                    raise RuntimeError(f"Server digest unavailable: {record['name']}")
                time.sleep(2 ** attempt)
            print(f"Verified draft asset: {record['name']} ({record['bytes']} bytes)", flush=True)
        validate_draft(api(f"repos/{REPOSITORY}/releases/{release_id}"), tag, revision, release_id)
        final = server_assets(release_id)
        for record in records:
            require(record["name"] in final, "An attached asset disappeared")
            match_server_asset(final[record["name"]], record)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as stream:
            stream.write(f"Verified and attached {len(records)} native assets to draft `{tag}`. The release remains unpublished.\n\nSource: `{revision}`, run {run_id}.\n\n`native-assets.json` lists file sizes and SHA-256 digests for independent API comparison.\n")


class TransferTests(unittest.TestCase):
    revision = "a" * 40
    version = "0.3.0-portable-20260921"

    def test_run_must_be_successful_same_repository_exact_workflow_and_revision(self):
        run = {"id": 1, "status": "completed", "conclusion": "success", "head_sha": self.revision, "name": WORKFLOW_NAME, "path": WORKFLOW_PATH, "repository": {"full_name": REPOSITORY}, "head_repository": {"full_name": REPOSITORY}, "event": "push"}
        validate_run(run, 1, self.revision)
        for key, value in [("conclusion", "failure"), ("status", "in_progress"), ("head_sha", "b" * 40), ("path", "other.yml"), ("event", "pull_request"), ("head_repository", {"full_name": "fork/parla"})]:
            changed = dict(run, **{key: value})
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_run(changed, 1, self.revision)

    def test_draft_target_and_existing_asset_must_match_without_clobber(self):
        release = {"id": 3, "draft": True, "tag_name": f"v{self.version}", "target_commitish": self.revision}
        validate_draft(release, release["tag_name"], self.revision, 3)
        for key, value in [("draft", False), ("target_commitish", "main"), ("id", 4)]:
            with self.assertRaises(RuntimeError):
                validate_draft(dict(release, **{key: value}), release["tag_name"], self.revision, 3)
        expected = {"name": "package.pkg", "bytes": 7, "sha256": "a" * 64}
        actual = {"name": "package.pkg", "size": 7, "digest": "sha256:" + "a" * 64, "state": "uploaded"}
        match_server_asset(actual, expected)
        for key, value in [("digest", None), ("digest", "sha256:" + "b" * 64), ("size", 8), ("state", "starter")]:
            with self.assertRaises(RuntimeError):
                match_server_asset(dict(actual, **{key: value}), expected)

    def test_checksums_cover_every_file_and_reject_tampering_and_extra_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "package.pkg").write_bytes(b"package")
            sums = root / "SHA256SUMS.txt"
            sums.write_text(f"{sha(root / 'package.pkg')}  package.pkg\n")
            validate_checksums(root, {"package.pkg"})
            (root / "package.pkg").write_bytes(b"changed")
            with self.assertRaises(RuntimeError): validate_checksums(root, {"package.pkg"})
            (root / "package.pkg").write_bytes(b"package")
            sums.write_text(sums.read_text() * 2)
            with self.assertRaises(RuntimeError): validate_checksums(root, {"package.pkg"})
            sums.write_text(f"{'a' * 64}  ../package.pkg\n")
            with self.assertRaises(RuntimeError): validate_checksums(root, {"package.pkg"})

    def test_installed_evidence_requires_provenance_defaults_replay_and_no_input(self):
        manifest = {"files": [{"path": "parla"}]}
        record = {"verified_files": 1, "binary_version": f"Parla {self.version} ({self.revision})", "stutter_correction_default": True, "settings_preserved": True, "microphone_used": False, "input_injection_used": False, "live_hotkey_permissions_tested": False, "setup": {"engine": "parakeet", "cleanup_mode": "faithful", "items": [{"id": k, "state": v} for k, v in {"python": "found", "sherpa": "ready", "parakeet-model": "found"}.items()]}, "runtime_versions": {"python": "3.12.14", "numpy": "2.4.6", "sherpa": "1.13.7"}, "replay": "I don't wish to see the old portrait.", "public_fixture_sha256": "b" * 64}
        validate_runtime(record, manifest, self.version, self.revision, "test")
        for key, value in [("verified_files", 0), ("binary_version", "wrong"), ("stutter_correction_default", False), ("microphone_used", True), ("input_injection_used", True), ("settings_preserved", False), ("replay", "failed")]:
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_runtime(dict(record, **{key: value}), manifest, self.version, self.revision, "test")

    def test_exact_three_artifacts_and_no_expired_or_unproven_sources(self):
        artifacts = [{"name": f"parla-{platform}", "expired": False, "size_in_bytes": 100, "digest": "sha256:" + "a" * 64, "workflow_run": {"id": 1, "head_sha": self.revision}} for platform in PLATFORMS]
        validate_artifacts(artifacts, 1, self.revision)
        for changed in [artifacts[:-1], artifacts + [artifacts[0]], copy.deepcopy(artifacts)]:
            if len(changed) == 3: changed[0]["expired"] = True
            with self.assertRaises(RuntimeError): validate_artifacts(changed, 1, self.revision)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-run", type=int)
    parser.add_argument("--expected-revision")
    parser.add_argument("--tag")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(TransferTests)
        raise SystemExit(0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1)
    if not all((args.source_run, args.expected_revision, args.tag, args.output)):
        parser.error("--source-run, --expected-revision, --tag and --output are required")
    attach(args)


if __name__ == "__main__":
    main()
