# Parla current state

**Source updated: 2026-09-21 · New build: `0.3.0-portable-20260921`.**

## September 21 changes and verification

The shared application remains Rust. OpenWhispr's architecture informed the separation of OS integration from recording, model services, and cleanup; its Electron/TypeScript shell was not transplanted. No OpenWhispr source was copied. Reference: [OpenWhispr at a77fdce](https://github.com/OpenWhispr/openwhispr/tree/a77fdce34dbc0932cc3eee90646017df9be1c876), particularly `src/helpers/whisperServer.js`, the native platform helpers, and `src/locales/en/prompts.json`. OpenWhispr's general LLM cleanup permits grammar edits; Parla's new deterministic stutter stage follows the owner's instruction to preserve wording.

- `Remove stutters` defaults on and can be disabled. It removes explicit repeated fragments (`b-b-book`), pronoun repetitions (`I I need`), and selected adjacent phrase restarts (`I want I want to leave`). It preserves tested grammatical repetition (`had had`, `that that`, `I think I think too much`), emphasis, numbers, negation, quotes/code, and dictionary-protected terms. Ambiguous cases are intentionally retained. This is transcript cleanup, not a guarantee of recognition/correction of all stuttered speech. Raw ASR and dictionary-normalized versions remain available.
- The full Windows path remains native. macOS uses accessibility-based target checks and a guarded paste; X11 uses focused-window identity and an input-change monitor. Unix pastes are single-line and never send Return. Unix has manual recovery rather than verified Restore; its status display is the dashboard rather than the Windows floating HUD.
- Wayland uses dashboard Start/stop or a desktop-bound `parla toggle` command, followed by Copy final/manual paste. Native Wayland global shortcuts and automatic injection are not implemented. Explicit dashboard/CLI recordings do not automatically paste into the dashboard or invoking terminal.
- Speech processes stay warm. A running process owned by Parla and matching the configured model avoids repeated file deployment, weight scans, and health requests per utterance; child exits and model changes are still checked. External services still require readiness checks.
- The resampler uses independent accumulation lanes and reuses phase normalization. One synthetic ten-second benchmark measured 1.78–1.80x faster conversion (about 10.3–10.5 ms down to 5.7–5.9 ms), with identical PCM on those inputs. This is a component result, **not** an overall dictation speed multiplier. Reproduce with `scripts/benchmark-resample.py`.
- Short-recording buffers reuse their allocation and clear used samples; large recordings retain ownership transfer to avoid a new copy penalty. The microphone still starts/stops as before; it is not kept recording while idle.
- `scripts/benchmark-asr.py` measured a 7.435-second public fixture on the development CPU: the existing four-thread Parakeet setting had a warm median of about 456 ms. Eight threads measured about 435 ms, too small and machine-specific a difference to justify changing everyone's default. No GPU or language rewrite speed claim is made.
- Data paths are `%LOCALAPPDATA%/Parla`, `~/Library/Application Support/Parla`, and `${XDG_DATA_HOME:-~/.local/share}/parla`; `PARLA_DATA_DIR` provides an explicit override for isolated tests. Bundled model/runtime files are separate from writable state. Builds include source-revision metadata.
- [Project handbook](PARLA-PROJECT-HANDBOOK.md) consolidates 32 inventoried Markdown files and their history. [Public launch plan](PARLA-PUBLIC-LAUNCH-PLAN.md) contains source-linked marketing research and a staged 90-day plan. Original documents remain intact.

Windows validation completed: **140 Rust tests passed, 0 failed, 4 opt-in live checks ignored**; optimized GNU build; five controlled Python HTTP tests; isolated installer/uninstaller; hashes for **1,015 installed files**; bundled Parakeet and Whisper public-fixture recognition with a system-only PATH; default-on stutter setting and embedded source version; repeated initialization and uninstall preserving user settings. The tested executable embeds `ff19184a197e302ab37584131ebad5713049e68b`. The maintainer's running September 16 application was not replaced.

Native package validation passed at `853c5ee7cda79e411f1b9240c74f0068323e1a47` in [GitHub Actions run 35613094718](https://github.com/ILoveRestockMonitors/parla/actions/runs/35613094718): **114 Rust tests on each Mac architecture and 115 on Linux**, no failures or ignored tests, optimized native builds, actual PKG/DEB installations, payload hashes, version/source identity, dependency readiness, default-on stutter settings, preservation of existing settings, and installed public-fixture replay. Linux additionally passed replay from a relocated tarball. Both Mac packages verify their receipt and actual `/Applications/Parla.app` installation. Five packaging regression tests passed. The Windows and Unix builds have identical application/Cargo/VERSION sources; their commit difference consists of packaging, verification, and documentation changes.

Published downloads: [release v0.3.0-portable-20260921](https://github.com/ILoveRestockMonitors/parla/releases/tag/v0.3.0-portable-20260921), containing the Windows installer and standalone executable, Apple Silicon/Intel Mac PKGs, Linux x64 DEB/archive, manifests, verification records, notices and SHA-256 checksums. The Windows installer is **962,942,588 bytes**, SHA-256 `6c6451aafde0f705ef629211ddbae691e9973b7a892c423ae1b34284b1533193`. The release's `release-record.json` identifies the actual Windows/Unix build commits and test scope. All 27 uploaded asset digests matched the tested originals/verified native CI index. [Public download verification](release-download-verification-20260921.json) checked anonymous access; large binaries used header reads plus matching full-file GitHub server digests, while small evidence files were downloaded and hashed in full. Fresh download queries resolved transient cached gateway errors.

No macOS/Linux live microphone, permission-grant, or editor interaction was performed. Mac/Linux packages are initial previews pending interactive verification; Windows was checked on the development PC rather than a clean VM. Windows is unsigned; Mac is not Developer ID signed or notarized. See [Unix installation and build instructions](unix-installation.md).

## Previous released baseline — September 19

The following record describes `0.2.0-bundled-20260919`, before the changes above. Its test counts and download sizes are historical, not evidence for the new packages.

**[Download the Windows x64 installer](https://github.com/ILoveRestockMonitors/parla/releases/latest/download/Parla-Setup.exe)** — about 963 MB, containing Parakeet, Whisper, both speech models, and private Python dependencies. It installs about 1.3 GB without additional speech downloads. See [installation instructions](windows-download.md). The installer and app are unsigned.

The files in this repository are the authoritative maintained source. The embedded source in the [complete build guide](../PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md) is a frozen `0.2.0-reliability-20260908` baseline. Build this repository directly for the current app; do not extract the older appendix over it.

This release adds offline installer support, fresh-install settings, dashboard launch shortcuts, and dependency diagnostics to the September 16 terminal-insertion runtime. It does not change dictation formatting or insertion rules. Fresh installs use bundled Parakeet and Faithful cleanup; existing settings are preserved. Ollama remains optional.

## September 19 baseline behavior

| Area | Implemented behavior |
| --- | --- |
| Recording | Ctrl+Space starts/stops recording; Ctrl then Win is hold-to-talk. Sessions and outstanding work are bounded. |
| Recognition | Bundled local Parakeet Python/sherpa-onnx service by default; bundled Whisper server is available as an alternative. |
| Dictionary | Explicit spelling replacements and preferred casing, applied after recognition. Whisper also receives vocabulary hints; the Parakeet client currently ignores hotword hints. |
| Faithful | Preserves recognized wording with dictionary corrections and the requested automatic numeric/list formatting. |
| Polished | Optional local Ollama cleanup, collected and checked before insertion. Rejected edits fall back to normalized wording. |
| Numbers | Number-only speech becomes digits automatically in both modes, including natural cardinal phrases and leading zeros. |
| Lists | Clear list cues produce bullet lines in writing apps such as Codex. Code editors skip automatic lists. |
| Browsers | Ordinary single-line prose; multiline results are flattened, and no Enter/Shift+Enter is generated for recognized browser processes. Numeric entry remains available. |
| Terminals | Windows Terminal and classic console screen controls tolerate output/prompt redraws with unchanged native focus, pane identity, input-monitor epoch and empty selections. Terminal host dictation stays on one line with no Enter/Shift+Enter; scrollback is excluded from polishing context and document-style replacement. |
| Insertion | Automatic Unicode typing with native target, input-change, and modifier checks; partial sends are not retried in full. |
| Accessibility | Explicit inspection-failure handling, bounded observation retries, editor-boundary clamping, and an input-monitor-backed native fallback. |
| Recovery | Best-effort clipboard backup, dashboard raw/normalized/final text, optional short-lived in-memory retry audio, and exact-range checks for restoring or replacing an insertion. |
| Feedback | Soft chimes and a native, click-through recording/processing HUD. It hides when idle. |
| Controls | Embedded local HTML dashboard on port 9393, settings, history options, learned corrections, and restore/retry controls. |
| Packaging | Per-user offline Setup.exe, desktop/Start shortcuts, automatic fresh settings, pinned dependencies and model hashes, license notices, and uninstall preserving user data. Source-build staging and rollback helpers remain available. |
| Setup checks | Read-only dependency checks on dashboard load or recheck, official missing-tool links, optional Ollama detection and downloaded-model checks. File presence and speech-server readiness are reported separately. |

See [CHANGELOG.md](../CHANGELOG.md) for the individual September 8 and September 15 changes, including the return to automatic typing, field-boundary fixes, numeric phrases, refined HUD/chimes, lists, multiline handling, and browser behavior.

## September 19 baseline architecture

Windows hooks feed a recording controller. Microphone callbacks collect audio; one bounded worker performs resampling, recognition, dictionary normalization, and optional cleanup. The controller commits completed results after destination checks. Separate workers support accessibility inspection, the local dashboard, and the native HUD. Dictionary and optional history use SQLite.

This is a Rust Windows application with local HTTP model services. The `src-tauri` directory and React scaffold are historical; Tauri IPC is not wired. LLM output is collected before insertion; the older descriptions of live token-by-token injection no longer apply.

## Earlier release verification

On September 19, the bundled release passed:

- **129 Rust tests, 0 failed, 4 opt-in live tests ignored**, an optimized GNU build, and **3 isolated CLI smoke checks**.
- Parakeet and Whisper transcription of the public speech fixture shipped by sherpa-onnx. Tests used the packaged Python and speech executables, a restricted system-only PATH, ephemeral local ports, and isolated user data. No microphone recording or automatic typing was performed.
- Silent installation into an isolated path containing spaces, first-run bundled settings, hash checks of all **1,014 manifest files**, model transcription from the installed directory, and uninstall retaining the personal settings file. This was a test installation on the development PC; it did not replace the maintainer's running app.
- Browser inspection of the actual dashboard with fixture responses, including dependency readiness, an optional missing Ollama link, and the Recheck tools control.

These tests do not establish clean-machine, every-CPU, microphone, or editor compatibility. A clean Windows VM and signed-distribution test were not performed. Full release download hashes and the installed-file manifest are published with the installer; the Rust executable still embeds a release label rather than a Git commit.

### Earlier live workflow validation

On September 16, the terminal-insertion release passed:

- Rust suite: **122 passed, 0 failed, 4 ignored**. Ignored tests require explicitly invoked live accessibility/HUD checks.
- Optimized Windows GNU release build and **3 CLI checks passed**.
- The new read-only terminal provider check was explicitly run against the existing Windows Terminal window and passed: terminal classification, text-pattern availability and pane identity were confirmed. No focus changes or keyboard input were generated by this check.
- Controlled Python shim HTTP suite: **5 passed** before the insertion-only update, using a fake recognizer on an ephemeral local port.

The maintainer's idle app was upgraded to `0.2.0-terminal-insertion-20260916` with rollback copies retained. Its SHA-256 matched the tested release:

```text
1E533933A359E07B2EE63ADB9C95F48C8B9A5454744402DC48ADDE9F774BF443
```

This identifies that installed artifact; other builds are not required to produce an identical hash. The binary currently embeds a release label, not a Git revision, so source attribution also relies on the release records.

Before the fix, the running app reported a recovered insertion with `caret, selection, or surrounding text changed` during the user's failed Hermes attempts. Regression tests reproduce this comparison failure and verify that only recognized terminal screen controls can tolerate redraws. Automated checks did not type into Hermes or record audio. After activation, the user confirmed **five successful automatic insertions out of five Hermes CLI dictations**. This validates the reported workflow, without establishing compatibility with every terminal, elevated process or remote session.

The guide's **85-test** count applies to its historical baseline. The separately maintained older project copy produced **109 passed, 1 ignored** during comparison. Neither count describes the current repository.

## Follow-up work recorded at the earlier baseline

These are review findings and proposed improvements, **not implemented fixes**:

- Controller-side field inspection, finalization, and insertion can delay new recording events. Cancellation discards stale output but does not interrupt active model computation.
- Native fallback cannot prove field identity or password status when accessibility inspection fails. Ordinary document insertion confirmation checks a 120-character suffix; replacement additionally requires matching the entire selected payload. Terminal receipts are always unverified for replacement because console selection is not editable input.
- CLI replay and dashboard Retry omit the original app and field context. Retry is a preview, not automatic reinsertion.
- Saved settings, startup configuration, and the runtime's reported settings can diverge. Some changes need reopening a device or restarting the app.
- One last-result slot and one retry clip are retained. A later result can displace failed-result recovery; the idle HUD does not display errors.
- Deterministic formatting and optional polishing need clearly documented, distinct validation policies. Preserve the requested numbers/lists/browser behavior while improving those policies.
- Embedding source commit hashes in builds and removal of unused scaffolding remain future work. The bundled installer now supplies a model-content and installed-file manifest.

See the [September 16 architecture assessment](architecture-review-2026-09-16.md) for the evidence and qualifications.
