# Changes

## 0.2.0-lists-20260915

- Automatically format clear spoken lists as separate plain-text bullet lines in Faithful and Polished modes, without calling a language model. For example, "I'm going grocery shopping. I need these in order. Eggs, milk, bread, cheese pizza" keeps the introduction and produces four bullets, preserving "cheese pizza" as one item.
- Recognize common shopping/list introductions, concise comma/semicolon enumerations, and consecutive first/second/third markers. Preserve item order, quantities, names, repeated items, quoted/parenthesized phrases and a following prose sentence. Code-editor dictation and number-only entry retain their existing paths.
- Keep ordinary prose and existing multiline text unchanged. This is conservative boundary detection: a phrase with no punctuation or list markers remains intact rather than guessing where multiword items begin/end. Recognition quality still determines the available words and punctuation.

## 0.2.0-feedback-20260915

- Replace the bright synth beeps with quiet, overlapping sine chimes. Rounded attacks and complete fade-outs remove the old hard note boundaries; start, completion and error remain distinct.
- Replace the rectangular status window with a native dark capsule, anti-aliased corners, transparent shadow, mint status symbol, clear timing and shortcut hints. Fade in/out gently, respect Windows animation preferences, and scale text for the active monitor above its taskbar.
- Keep the overlay click-through and non-activating. Recording and processing are the only visible states; idle and disabled HUDs stay hidden. Dictation, shortcuts, recognition, numeric conversion and text insertion are unchanged.
- `parla.exe --preview-feedback <output-directory>` exports the exact chimes and native HUD renders at 100%, 150% and 200% scale without playback, microphone capture, keyboard hooks or model services.

## 0.2.0-number-phrases-20260915

- Extend automatic numeric dictation to teens, tens, hyphenated compounds, hundreds and descending thousand/million/billion/trillion phrases. Mixed speech such as "seven eight nine six nine four twenty" becomes `78969420`; "sixty seven two four zero nine eight" becomes `6724098`.
- Parse conventional phrases as numeric chunks before joining them: "sixty-nine thousand four hundred twenty" becomes `69420`. Accept "and" within hundred/scale phrases. Preserve existing numeric groups and leading zeros; reject invalid grammar and arithmetic overflow without partial conversion.
- Keep the whole-utterance check: "Paul had two apples" and other prose continue through normal cleanup. No mode switch or formatter call is required for number-only speech.

## 0.2.0-numbers-20260915

- Automatically join utterances consisting entirely of zero-through-nine words and recognized digit groups. "One five four eight seven nine one three two" becomes `154879132`; leading zeros are preserved. "Oh" means zero only alongside an unambiguous digit.
- Apply after dictionary corrections, before optional polishing, in both cleanup modes and all app categories. Preserve raw recognition and dictionary-normalized text for recovery. No setting, model call, or extra mode is needed.
- Omit the usual trailing space for pure digit output, including clipboard backup. Leave prose, ambiguous homophones, signs, decimals and cardinal phrases on the existing cleanup path.
- Regression coverage includes the requested sequence, ASR punctuation/case/whitespace, mixed digits, long codes, leading zeros, ordinary sentences and exact insertion spacing.

## 0.2.0-automatic-20260908

- Reverted the clipboard-paste release at the user's request. Automatic Unicode typing is restored. Best-effort clipboard copying remains as an independent backup and cannot block automatic insertion.
- HUD displays only recording/processing and hides when idle, including when an error remains in the dashboard.
- Failed/incomplete UI Automation inspection can fall back to a nonzero, unchanged native foreground window and focused control, with no intervening edit/navigation keys or mouse-button presses. A counter tracks input changes without storing keys, text, or pointer positions. Dictation shortcuts, modifier releases and Parla's own injected text are excluded. External automation that types or clicks invalidates the fallback too. Pointer motion and scrolling do not invalidate it.
- Fallback requires both input hooks to be available, matching counters, and no known conflicting accessibility identity/password flag. Actual field/text changes remain blocked. Unicode batches check for input changes before each send; partial sends are never retried in full.
- The fallback authorizes insertion only. Restore original and spoken destructive commands still require their existing exact-text verification.
- Automated coverage includes failed start/end inspections, changed input/focus, unknown monitor state, password refusal, shortcut handling and idle HUD behavior. Native focus plus input continuity is a compatibility fallback, not a proof against an application changing its DOM programmatically while accessibility is unavailable.

Build this version from repository source; the single-file guide's embedded source remains the original reliability snapshot.

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
