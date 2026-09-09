# Parla — local dictation for Windows

Parla is a Windows x64 speech-to-text app inspired by Whisperflow. Press **Ctrl+Space** to start recording, then press it again to finish, or hold **Ctrl+Win** for quick dictation. A small native HUD shows recording and processing status, and a local dashboard provides settings, history, corrections, and recovery controls.

## Start here

See [Changes](CHANGELOG.md) for the current insertion reliability patch and the distinction between this source and the guide's original embedded snapshot.

**[Complete shareable build guide](PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md)** — detailed prerequisites, exact commands, architecture, implementation contracts, troubleshooting, tests, installation, and rollback. The guide also contains a checksummed source snapshot and a Python extractor, so the single Markdown file can be shared independently of this repository.

If you clone this repository, the source is already extracted. Skip the guide's extraction step and use the repository root for its build commands. The guide's appendix is a frozen snapshot; this README adds the repository introduction.

## Features

Automatic insertion uses Unicode typing. Each completed dictation also copies its text to the clipboard as a backup; a busy clipboard does not block typing. The HUD shows recording/processing and disappears when idle. A failed accessibility check can use the same native field when the input monitor confirms no intervening typing or clicks; verified restore operations still require exact text checks.

- Local recognition through whisper.cpp, with optional Parakeet through a local Python/sherpa-onnx service.
- **Faithful** cleanup preserves recognized wording while applying explicit dictionary corrections.
- Optional **Polished** cleanup uses a local Ollama model, with validation and fallback when the proposed edit changes protected content.
- **Learn correction** saves an explicit spelling replacement; it does not retrain the recognition model.
- **Restore original** attempts to replace the most recent unchanged insertion with raw recognition text after you return to the original field. The dashboard explains the time limit and recovery conditions.
- Optional short-lived retry audio, bounded recording sessions, chimes, history controls, and a native HUD.

## Build overview

Use Windows x64, the Rust GNU toolchain, and a complete WinLibs compiler distribution. Follow the guide for installation and verification of these prerequisites. This is a Rust executable with an embedded HTML dashboard. The `src-tauri` directory name and React scaffold are historical; no npm build is needed.

From the repository root, after installing the prerequisites:

```powershell
cargo +stable-x86_64-pc-windows-gnu fetch --locked --manifest-path .\Cargo.toml
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-release.ps1 -CompilerBin 'C:\path\to\mingw64\bin'
```

Replace the compiler path with the actual folder containing `gcc.exe` and `ar.exe`. The build script runs the Rust tests, builds an optimized executable, and creates a versioned folder under `release` containing the executable and its SHA-256 checksum. It uses locked, offline dependencies after the fetch step.

Download whisper.cpp's server and a compatible model as described in the guide. Initialize settings using actual paths:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\initialize-settings.ps1 -WhisperServerExe 'C:\path\to\whisper-server.exe' -WhisperModelPath 'C:\path\to\ggml-small.bin'
```

This first-run script refuses to overwrite existing settings. Follow the guide's staging, activation, and first-run checks before using dictation in other applications. Parakeet and Ollama are optional; Faithful mode does not require Ollama.

## Source layout

| Location | Purpose |
| --- | --- |
| `src-tauri/src` | Rust runtime, recording, hotkeys, recognition, cleanup, insertion, and persistence |
| `src-tauri/assets/dashboard.html` | Local settings and recovery dashboard |
| `src-tauri/assets/parakeet-shim.py` | Optional local Parakeet service |
| `scripts` | Build, first-run settings, installation, and rollback helpers |
| `tests` | Python integration checks and manual application compatibility matrix |
| `PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md` | Standalone build specification and embedded source snapshot |

## Verification and limitations

The source snapshot was locally verified with 85 passing Rust tests, five passing shim HTTP tests, three passing CLI smoke tests, an optimized Windows GNU build, and installer/rollback checks. The guide records the scope and limitations of that verification. These results do not guarantee recognition accuracy or compatibility with every editor; real microphone, model, and target-application checks remain necessary.

Local processing still creates local data according to your settings. Review history and retry-audio settings in the guide. Model weights, executables, personal settings, databases, recordings, and operational logs are excluded from this repository. Download third-party dependencies and models separately under their respective terms.
