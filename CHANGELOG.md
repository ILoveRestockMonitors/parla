# Changes

## 0.2.0-paste-20260908

- Insert dictation with one Ctrl+V gesture instead of a burst of per-character Unicode key events. Never automatically resend an unconfirmed paste. Partial shortcut delivery releases synthetic keys without repeating the paste.
- Completed dictation deliberately replaces and remains on the Windows clipboard, including when target checks block insertion. This enables immediate manual Ctrl+V recovery. Clipboard write failures remain visible, and clipboard sequence checks reject a changed payload before input.
- Permit a changed native child focus handle only when the same nonempty accessibility runtime ID identifies the same editor, in the same foreground window, with unchanged field text. Rebind the final native focus check to that verified current snapshot. Real field/window/text changes are still rejected.
- HUD distinguishes verified insertion, unconfirmed paste, and blocked insertion with clipboard recovery. Unconfirmed insertion is not reported as verified success. Allow bounded additional read-back time for asynchronous editors.
- Automated tests cover native child-handle changes versus real editor changes and feedback priority. This release changes the delivery mechanism; automated checks do not establish universal application compatibility.

Use repository source for this version; the standalone Markdown appendix remains a frozen earlier snapshot.

## 0.2.0-field-boundary-20260908

- Clamp accessibility context to the focused editor's DocumentRange. Chromium character movement can escape a contenteditable: a live Discord check reproduced 512 characters before and 160 after a 52-character draft. Unrelated page updates could therefore block insertion and original-text restoration.
- Reject selections outside the editor. Restoration range selection is also confined to the editor, with existing exact-text and field-identity checks preserved.
- A read-only Rust integration test reproduced the escaping ranges in Discord, then verified the production clamp retained the complete draft and unchanged caret endpoints. No draft was modified or sent.
- Reproduce manually with a verified window containing one short editable draft: set `PARLA_TEST_HWND` to its native handle and `PARLA_EXPECT_ESCAPE=1`, then run `cargo test live_editor_ranges_stay_inside_field -- --ignored --nocapture`. The test prints no draft text. The optional `tests/field-boundary.html` fixture changes text outside a stable editor; browser fixture execution was not available during this validation.

Build this version from repository source. The standalone guide still embeds the original reliability snapshot; substitute this version in explicit installation paths. The previous insertion patch addressed incomplete inspection, not this confirmed field-boundary bug.

## 0.2.0-insertion-20260908

- Retry a failed accessibility observation once while the original native target remains focused. Retry transient pre-insertion identity/caret availability mismatches without authorizing a different target.
- Distinguish failed inspection from a successful observation of a control without text-pattern support. Failed observations cannot authorize insertion or restoration.
- Report window/control changes, field identity changes, caret/text changes, inspection failures, and held modifiers separately.
- Show a recovery message in the HUD when insertion is blocked; the completed text remains available through Copy final.
- Add deterministic regression tests for transient failure recovery and refusal of changed or unverifiable targets, plus an explicitly invoked read-only Windows accessibility diagnostic.

These changes address a concrete transient-inspection failure path. The intermittent issue reported across applications has not been reproduced end to end; further reports now provide a specific failure reason. Recognition and formatting behavior are unchanged.

The complete shareable guide embeds the earlier `0.2.0-reliability-20260908` snapshot. To build these changes, use this repository's source and scripts, not source extracted from that frozen appendix. The current build and installer scripts default to the version above; substitute it in the guide's explicit installation commands.
