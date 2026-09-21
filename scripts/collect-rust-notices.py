"""Collect actual locked Rust package notices for release redistribution."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True, encoding="utf-8"
    ))
    parts = ["Parla: third-party Rust dependency notices\n"
             "Includes locked dependencies for all supported platforms; not every package is linked into every binary.\n"]
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if not package.get("source"):
            continue
        directory = Path(package["manifest_path"]).parent
        parts.append(f"\n{'='*72}\n{package['name']} {package['version']}\n"
                     f"Declared license: {package.get('license') or 'See package files'}\n"
                     f"Source: {package.get('repository') or package['source']}\n")
        notices = []
        explicit = package.get("license_file")
        if explicit and (directory / explicit).is_file():
            notices.append(directory / explicit)
        for folder in [directory, directory / "licenses"]:
            if folder.is_dir():
                notices.extend(path for path in folder.iterdir() if path.is_file()
                               and path.name.lower().startswith(("license", "licence", "copying", "notice", "copyright")))
        for path in sorted(set(notices)):
            parts.append(f"\n--- {path.relative_to(directory)} ---\n"
                         + path.read_text(encoding="utf-8", errors="replace") + "\n")
        if not notices:
            parts.append("No separate license text is present in this package archive; see its declared license and source above.\n")
    # Statically linked MinGW runtime uses this exception; the original GNU
    # runtime notices remain in the Windows payload license directory as well.
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("".join(parts), encoding="utf-8")
    print(f"Wrote locked Rust notices: {args.output}")


if __name__ == "__main__":
    main()
