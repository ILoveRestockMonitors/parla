# Parla — local dictation for Windows

Parla is a Windows x64 speech-to-text app inspired by Whisperflow. Press **Ctrl+Space** to start recording, then press it again to finish, or hold **Ctrl+Win** for quick dictation. A small native HUD shows recording and processing status, and a local dashboard provides settings, history, corrections, and recovery controls.

## Download for Windows

**[Download Parla Setup.exe for Windows x64](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-Setup.exe)**

The installer includes **Parakeet, Whisper, both speech models, and a private Python runtime**. Install, open the desktop shortcut, and dictate; no separate speech downloads, Python, CUDA, Rust, Node.js, or Ollama are required for Faithful mode. The dashboard opens automatically and includes setup checks and optional-tool links. See the [Windows installation guide](docs/windows-download.md). The app and installer are unsigned; Windows may show an unknown-publisher prompt.

[Release notes and checksums](https://github.com/ILoveRestockMonitors/parla/releases/latest) · [Third-party notices](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/THIRD-PARTY-NOTICES.txt)

## Start here

**Current release: `0.2.0-bundled-20260919`; source and documentation checked September 19, 2026.** Read [Current state](docs/current-state.md) for the implemented features, test results, and remaining gaps, and [Changes](CHANGELOG.md) for the shipped updates. This repository is the authoritative maintained source.

**[Complete shareable build guide](PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md)** — detailed prerequisites, exact commands, architecture, implementation contracts, troubleshooting, tests, installation, and rollback. The guide also contains a checksummed source snapshot and a Python extractor, so the single Markdown file can be shared independently of this repository.

If you clone this repository, skip the guide's extraction step and build the repository directly. The guide's appendix is a frozen September 8 snapshot; it does not include the later insertion, numeric, list, browser, and feedback updates. Use the version emitted by the current build script in installation commands instead of the guide's older release label.

## Features

Automatic insertion uses Unicode typing. Before an ordinary insertion attempt, Parla also makes a best-effort clipboard copy; a busy clipboard does not block typing. The HUD shows recording/processing and disappears when idle. A failed accessibility check can use the same native field when the input monitor confirms no intervening typing or clicks; verified restore operations still require exact text checks.

- Local recognition through bundled Parakeet, with bundled whisper.cpp available as an alternative.
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

To create the complete offline installer, follow [Building the Windows installer](docs/windows-installer-build.md). The commands below are for an advanced manual installation of the standalone source build.

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

The current source passed **129 Rust tests (0 failed, 4 opt-in live tests ignored)**, an optimized Windows GNU build, and three CLI smoke checks on September 19, 2026. Both packaged speech engines transcribed a public test recording with system Python and developer tools excluded from their search path. The installer was checked in an isolated directory on the development PC, including installed-file hashes and preservation of personal settings during uninstall. See [Current state](docs/current-state.md) for scope and earlier live application checks. These results do not establish compatibility with every PC or editor; a clean Windows VM was not tested.

Local processing still creates local data according to your settings. Review history and retry-audio settings in the guide. Git source excludes model weights, executables, personal settings, databases, recordings, and operational logs. The downloadable installer includes the pinned speech dependencies and models with their license notices.
