# Changes

## 0.2.0-insertion-20260908

- Retry a failed accessibility observation once while the original native target remains focused. Retry transient pre-insertion identity/caret availability mismatches without authorizing a different target.
- Distinguish failed inspection from a successful observation of a control without text-pattern support. Failed observations cannot authorize insertion or restoration.
- Report window/control changes, field identity changes, caret/text changes, inspection failures, and held modifiers separately.
- Show a recovery message in the HUD when insertion is blocked; the completed text remains available through Copy final.
- Add deterministic regression tests for transient failure recovery and refusal of changed or unverifiable targets, plus an explicitly invoked read-only Windows accessibility diagnostic.

These changes address a concrete transient-inspection failure path. The intermittent issue reported across applications has not been reproduced end to end; further reports now provide a specific failure reason. Recognition and formatting behavior are unchanged.

The complete shareable guide embeds the earlier `0.2.0-reliability-20260908` snapshot. To build these changes, use this repository's source and scripts, not source extracted from that frozen appendix. The current build and installer scripts default to the version above; substitute it in the guide's explicit installation commands.
