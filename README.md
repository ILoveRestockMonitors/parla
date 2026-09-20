# Parla — local dictation for Windows

Parla is a Windows x64 speech-to-text app inspired by Whisperflow. Press **Ctrl+Space** to start recording, then press it again to finish, or hold **Ctrl+Win** for quick dictation. A small native HUD shows recording and processing status, and a local dashboard provides settings, history, corrections, and recovery controls.

## Download for Windows

**[Download Parla for Windows x64 (.exe)](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/parla.exe)** · **[Download with setup helpers (.zip)](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-windows-x64.zip)**

The prebuilt app requires no Rust compiler or Node.js. For a new installation, download the ZIP and follow the [Windows setup instructions](docs/windows-download.md). Speech recognition also requires a separately installed local engine and model; those large dependencies are not included. The executable is unsigned and Windows may show an unknown-publisher prompt.

[Release notes and checksums](https://github.com/ILoveRestockMonitors/parla/releases/latest) · [Third-party notices](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/THIRD-PARTY-NOTICES.txt)

## Start here

**Current release: `0.2.0-terminal-insertion-20260916`; source and documentation checked September 16, 2026.** Read [Current state](docs/current-state.md) for the implemented features, test results, and remaining gaps, and [Changes](CHANGELOG.md) for the shipped updates. This repository is the authoritative maintained source.

**[Complete shareable build guide](PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md)** — detailed prerequisites, exact commands, architecture, implementation contracts, troubleshooting, tests, installation, and rollback. The guide also contains a checksummed source snapshot and a Python extractor, so the single Markdown file can be shared independently of this repository.

If you clone this repository, skip the guide's extraction step and build the repository directly. The guide's appendix is a frozen September 8 snapshot; it does not include the later insertion, numeric, list, browser, and feedback updates. Use the version emitted by the current build script in installation commands instead of the guide's older release label.

## Features

Automatic insertion uses Unicode typing. Before an ordinary insertion attempt, Parla also makes a best-effort clipboard copy; a busy clipboard does not block typing. The HUD shows recording/processing and disappears when idle. A failed accessibility check can use the same native field when the input monitor confirms no intervening typing or clicks; verified restore operations still require exact text checks.

- Local recognition through whisper.cpp, with optional Parakeet through a local Python/sherpa-onnx service.
- **Faithful** cleanup preserves recognized wording while applying explicit dictionary corrections and the automatic numeric/list behavior described below.
- Clear lists automatically become separate bullet lines in writing apps such as Codex, in both cleanup modes. Say "I need eggs, milk, bread, and cheese pizza" or "First, call Alex. Second, review the quote." Introductions stay above the list; multiword items stay together. Natural pauses help recognition add item separators. Unclear boundaries stay as spoken text, and code editors keep ordinary dictation.
- Browser dictation defaults to normal sentences on one line, including address/search bars and page editors. Chrome, Edge, Firefox, Brave, Opera, Vivaldi and other recognized browsers receive text without Enter or Shift+Enter, so dictation does not submit searches or navigate between list items. Recovered or polished multiline text is flattened before typing; numeric entry still works normally. You submit the text yourself.
- Hermes CLI in Windows Terminal and classic console hosts tolerates terminal output and prompt redraws while dictating, provided the same pane and native focus remain, the input monitor reports no intervening typing/clicks, and no output text is selected. Terminal dictation stays on one line and sends no Enter/Shift+Enter. Terminal output is not used as polishing context or as an editable range for Restore.
- Number-only dictation is automatic in both cleanup modes: "one five four" becomes `154`; "sixty seven two four zero nine eight" becomes `6724098`; "sixty-nine thousand four hundred twenty" becomes `69420`. Natural number phrases are combined before joining adjacent chunks. Leading zeros and existing numeric groups are preserved. Ordinary sentences stay on the normal cleanup path. Digit entries have no trailing space; signs, decimals and ambiguous homophones are left alone.
- Optional **Polished** cleanup uses a local Ollama model, with validation and fallback when the proposed edit changes protected content.
- **Learn correction** saves an explicit spelling replacement; it does not retrain the recognition model.
- **Restore original** attempts to replace the most recent unchanged insertion with raw recognition text after you return to the original field. The dashboard explains the time limit and recovery conditions.
- Optional short-lived retry audio, bounded recording sessions, soft chimes, history controls, and a rounded native HUD that fades above the active monitor's taskbar without taking focus.

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
| `docs/current-state.md` | Current release, architecture, verification, and known gaps |
| `docs/architecture-review-2026-09-16.md` | Source-grounded architecture assessment and follow-up recommendations |
| `PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md` | Standalone build specification and embedded source snapshot |

## Verification and limitations

The current source passed **122 Rust tests (0 failed, 4 opt-in live tests ignored)**, an optimized Windows GNU build, and three CLI smoke checks on September 16, 2026. The read-only Windows Terminal provider test was then run explicitly and passed. Five controlled shim HTTP tests passed before this insertion-only update. The guide's 85-test result refers to its older embedded baseline. See [Current state](docs/current-state.md) for verification scope and the distinction between code checks and live application behavior. These results do not guarantee recognition accuracy or compatibility with every editor.

Local processing still creates local data according to your settings. Review history and retry-audio settings in the guide. Model weights, executables, personal settings, databases, recordings, and operational logs are excluded from this repository. Download third-party dependencies and models separately under their respective terms.
