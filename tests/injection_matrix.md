# Manual application compatibility gate

Perform only in operator-controlled disposable fields. Record actual app/version,
build ID, expected text, observed text and verified/unverified insertion receipt.
No historical pass result is evidence for a new computer or build.

| App class | Actual app/version | Hold | Ctrl+Space | Unicode | Selection | Result |
|---|---|---|---|---|---|---|
| Native | Notepad | pending | pending | pending | pending | pending |
| Browser | Disposable textarea | pending | pending | pending | pending | pending |
| Electron | Disposable editor | pending | pending | pending | pending | pending |
| Terminal/editor | Non-executing test surface | pending | pending | pending | pending | pending |

Also test focus/caret changes while processing, manual edits before Restore,
unchanged verified Restore within ten seconds, modifiers held at completion,
rapid second recording, queue saturation, and known secure/elevated rejection.
Never use an addressed message, actual password field or executing shell command
as an unattended insertion test. Record unsupported cases instead of weakening
target validation. Use the guide's quality corpus to evaluate ASR separately.
