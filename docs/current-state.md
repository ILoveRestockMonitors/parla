# Parla current state

**Checked: 2026-09-16 · Current application release: `0.2.0-browser-prose-20260915`.**

The files in this repository are the authoritative maintained source. The embedded source in the [complete build guide](../PARLA-COMPLETE-SHAREABLE-BUILD-GUIDE.md) is a frozen `0.2.0-reliability-20260908` baseline. Build this repository directly for the current app; do not extract the older appendix over it.

The current application source was published in commit [`15706f071bd88f3ae3606ef508540c2e41b17a15`](https://github.com/ILoveRestockMonitors/parla/commit/15706f071bd88f3ae3606ef508540c2e41b17a15). The September 16 reconciliation changes documentation and workspace organization, not runtime behavior.

## Current behavior

| Area | Implemented behavior |
| --- | --- |
| Recording | Ctrl+Space starts/stops recording; Ctrl then Win is hold-to-talk. Sessions and outstanding work are bounded. |
| Recognition | Local Whisper server or optional local Parakeet Python/sherpa-onnx service. |
| Dictionary | Explicit spelling replacements and preferred casing, applied after recognition. Whisper also receives vocabulary hints; the Parakeet client currently ignores hotword hints. |
| Faithful | Preserves recognized wording with dictionary corrections and the requested automatic numeric/list formatting. |
| Polished | Optional local Ollama cleanup, collected and checked before insertion. Rejected edits fall back to normalized wording. |
| Numbers | Number-only speech becomes digits automatically in both modes, including natural cardinal phrases and leading zeros. |
| Lists | Clear list cues produce bullet lines in writing apps such as Codex. Code editors skip automatic lists. |
| Browsers | Ordinary single-line prose; multiline results are flattened, and no Enter/Shift+Enter is generated for recognized browser processes. Numeric entry remains available. |
| Insertion | Automatic Unicode typing with native target, input-change, and modifier checks; partial sends are not retried in full. |
| Accessibility | Explicit inspection-failure handling, bounded observation retries, editor-boundary clamping, and an input-monitor-backed native fallback. |
| Recovery | Best-effort clipboard backup, dashboard raw/normalized/final text, optional short-lived in-memory retry audio, and exact-range checks for restoring or replacing an insertion. |
| Feedback | Soft chimes and a native, click-through recording/processing HUD. It hides when idle. |
| Controls | Embedded local HTML dashboard on port 9393, settings, history options, learned corrections, and restore/retry controls. |
| Packaging | Versioned executables, checksums, staged installation, and rollback helpers. |

See [CHANGELOG.md](../CHANGELOG.md) for the individual September 8 and September 15 changes, including the return to automatic typing, field-boundary fixes, numeric phrases, refined HUD/chimes, lists, multiline handling, and browser behavior.

## Architecture

Windows hooks feed a recording controller. Microphone callbacks collect audio; one bounded worker performs resampling, recognition, dictionary normalization, and optional cleanup. The controller commits completed results after destination checks. Separate workers support accessibility inspection, the local dashboard, and the native HUD. Dictionary and optional history use SQLite.

This is a Rust Windows application with local HTTP model services. The `src-tauri` directory and React scaffold are historical; Tauri IPC is not wired. LLM output is collected before insertion; the older descriptions of live token-by-token injection no longer apply.

## Verification

On September 16, the current repository passed:

- Rust suite: **116 passed, 0 failed, 3 ignored**. Ignored tests require explicitly invoked live accessibility/HUD checks.
- Controlled Python shim HTTP suite: **5 passed** using a fake recognizer on an ephemeral local port.

The maintainer's running executable was inspected without restarting it. Its SHA-256 matched the recorded `0.2.0-browser-prose-20260915` release:

```text
D4FAF11A35C5759613B2C7211C55B999B3D2BDB9481B02E5A29FF6DF393E900E
```

This identifies that installed artifact; other builds are not required to produce an identical hash. The binary currently embeds a release label, not a Git revision, so source attribution also relies on the release records.

The September 15 release records additionally document an optimized build and three CLI checks. The September 16 architecture review did not rerun live recognition/formatting, record microphone audio, or type into applications. Automated test results do not establish accuracy or compatibility with every editor.

The guide's **85-test** count applies to its historical baseline. The separately maintained older project copy produced **109 passed, 1 ignored** during comparison. Neither count describes the current repository.

## Known gaps and follow-up work

These are review findings and proposed improvements, **not implemented fixes**:

- Controller-side field inspection, finalization, and insertion can delay new recording events. Cancellation discards stale output but does not interrupt active model computation.
- Native fallback cannot prove field identity or password status when accessibility inspection fails. Ordinary insertion confirmation checks a 120-character suffix; replacement additionally requires matching the entire selected payload.
- CLI replay and dashboard Retry omit the original app and field context. Retry is a preview, not automatic reinsertion.
- Saved settings, startup configuration, and the runtime's reported settings can diverge. Some changes need reopening a device or restarting the app.
- One last-result slot and one retry clip are retained. A later result can displace failed-result recovery; the idle HUD does not display errors.
- Deterministic formatting and optional polishing need clearly documented, distinct validation policies. Preserve the requested numbers/lists/browser behavior while improving those policies.
- Source hashes in builds, model-content manifests, and removal of unused scaffolding remain future work.

See the [September 16 architecture assessment](architecture-review-2026-09-16.md) for the evidence and qualifications.
