# Parla — local dictation

Parla is a local speech-to-text app with a shared Rust core and native Windows, macOS, and Linux adapters. Windows uses **Ctrl+Space** to start/stop and **Ctrl+Win** for hold-to-talk. The Unix default toggle is **Ctrl+Alt+Space**; macOS hold-to-talk uses **Ctrl+Command**. A local dashboard provides settings, corrections, history, recording controls, and recovery. The floating HUD is Windows-only.

The `0.3.0-portable-20260921` source adds conservative stutter removal, faster audio conversion and reusable short-recording buffers. Stutter cleanup preserves wording and runs without an LLM; it can be disabled in settings. See the [current verification and limitations](docs/current-state.md), [Mac/Linux installation guide](docs/unix-installation.md), [consolidated project handbook](docs/PARLA-PROJECT-HANDBOOK.md), and [public launch plan](docs/PARLA-PUBLIC-LAUNCH-PLAN.md).

Platform code is not a claim that every desktop/editor has been tested. X11 supports guarded automatic paste; Wayland uses explicit recording controls and manual paste. macOS requires Microphone, Accessibility, and Input Monitoring permissions. Unix insertion stays on one line and does not support verified Restore. Published package evidence is attached to the corresponding release.

## Download for Windows

**[Download Parla Setup.exe for Windows x64](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-Setup.exe)**

The installer includes **Parakeet, Whisper, both speech models, and a private Python runtime**. Install, open the desktop shortcut, and dictate; no separate speech downloads, Python, CUDA, Rust, Node.js, or Ollama are required for Faithful mode. The dashboard opens automatically and includes setup checks and optional-tool links. See the [Windows installation guide](docs/windows-download.md). The app and installer are unsigned; Windows may show an unknown-publisher prompt.

[Release notes and checksums](https://github.com/ILoveRestockMonitors/parla/releases/latest) · [Third-party notices](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/THIRD-PARTY-NOTICES.txt)

## Start here

**Development release: `0.3.0-portable-20260921`; source updated September 21, 2026.** Read [Current state](docs/current-state.md) for implementation, test results, package publication status, and remaining gaps. [Changes](CHANGELOG.md) records release behavior. Build this repository directly; old embedded source snapshots are historical.

**[Complete shareable build guide](PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md)** — detailed prerequisites, exact commands, architecture, implementation contracts, troubleshooting, tests, installation, and rollback. The guide also contains a checksummed source snapshot and a Python extractor, so the single Markdown file can be shared independently of this repository.

If you clone this repository, skip the guide's extraction step and build the repository directly. The guide's appendix is a frozen September 8 snapshot; it does not include the later insertion, numeric, list, browser, and feedback updates. Use the version emitted by the current build script in installation commands instead of the guide's older release label.

## Features

On Windows, automatic insertion uses Unicode typing. Before an ordinary insertion attempt, Parla also makes a best-effort clipboard copy; a busy clipboard does not block typing. The Windows HUD shows recording/processing and disappears when idle. A failed Windows accessibility check can use the same native field when the input monitor confirms no intervening typing or clicks; verified restore operations still require exact text checks. macOS and X11 use guarded clipboard paste and manual recovery; Wayland uses manual paste.

- Local recognition through bundled Parakeet, with bundled whisper.cpp available as an alternative in the Windows installer.
- **Faithful** cleanup preserves recognized wording while applying explicit dictionary corrections and the automatic numeric/list behavior described below.
- **Remove stutters** (on by default) removes explicit fragments such as `b-b-book`, repeated pronouns such as `I I need`, and selected adjacent phrase restarts such as `I want I want to leave`. It leaves ambiguous emphasis, grammatical repetition, numbers, negation, quoted text, and protected dictionary terms alone. This is conservative transcript cleanup, not a guarantee that every spoken stutter is recognized or corrected. Raw and dictionary-normalized text remain available for recovery.
- Clear lists automatically become separate bullet lines in writing apps such as Codex, in both cleanup modes. Say "I need eggs, milk, bread, and cheese pizza" or "First, call Alex. Second, review the quote." Introductions stay above the list; multiword items stay together. Natural pauses help recognition add item separators. Unclear boundaries stay as spoken text, and code editors keep ordinary dictation.
- Browser dictation defaults to normal sentences on one line, including address/search bars and page editors. Chrome, Edge, Firefox, Brave, Opera, Vivaldi and other recognized browsers receive text without Enter or Shift+Enter, so dictation does not submit searches or navigate between list items. Recovered or polished multiline text is flattened before typing; numeric entry still works normally. You submit the text yourself.
- Hermes CLI in Windows Terminal and classic console hosts tolerates terminal output and prompt redraws while dictating, provided the same pane and native focus remain, the input monitor reports no intervening typing/clicks, and no output text is selected. Terminal dictation stays on one line and sends no Enter/Shift+Enter. Terminal output is not used as polishing context or as an editable range for Restore.
- Number-only dictation is automatic in both cleanup modes: "one five four" becomes `154`; "sixty seven two four zero nine eight" becomes `6724098`; "sixty-nine thousand four hundred twenty" becomes `69420`. Natural number phrases are combined before joining adjacent chunks. Leading zeros and existing numeric groups are preserved. Ordinary sentences stay on the normal cleanup path. Digit entries have no trailing space; signs, decimals and ambiguous homophones are left alone.
- Optional **Polished** cleanup uses a local Ollama model, with validation and fallback when the proposed edit changes protected content.
- **Learn correction** saves an explicit spelling replacement; it does not retrain the recognition model.
- On Windows, **Restore original** attempts to replace the most recent unchanged insertion with raw recognition text after you return to the original field. The dashboard explains the time limit and recovery conditions. Unix uses Copy raw and manual recovery.
- Optional short-lived retry audio, bounded recording sessions, soft chimes, history controls, and a rounded native HUD that fades above the active monitor's taskbar without taking focus.

## Build overview

For Windows, use the Rust GNU toolchain and a complete WinLibs compiler distribution. For macOS/Linux, follow the [native build and packaging guide](docs/unix-installation.md). This is a Rust executable with an embedded HTML dashboard. The `src-tauri` directory name and React scaffold are historical; no npm build is needed. Python is the local adapter around native sherpa-onnx inference, not the app runtime.

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

The September 21 Windows build passed **140 Rust tests (0 failed, 4 opt-in live tests ignored)** and an optimized GNU build. The new installer passed an isolated installation, hashes for 1,015 payload files, fresh/default settings and source-version checks, recognition by both bundled engines, and uninstall preserving settings. The controlled Python HTTP suite passed five tests. Native Unix build and installed-package evidence is recorded in [Current state](docs/current-state.md) and the release's verification files. These results do not establish compatibility with every PC or editor; a clean Windows VM and live Mac/Linux microphone/editor sessions were not tested.

Local processing still creates local data according to your settings. Review history and retry-audio settings in the guide. Git source excludes model weights, executables, personal settings, databases, recordings, and operational logs. The downloadable installer includes the pinned speech dependencies and models with their license notices.
