# FeatherMark Architect Review — Round 4

**Verdict:** CONCERNS
**Reviewed SHA-256:** `6a3e336a1a71cfc400cb69ae8e4b3bd74c8cd1c7a56fe496f3fbb33e2a3cfb83`

## Required changes

1. Assign `feathermark-types` and `feathermark-protocol` creation to exactly one Task-1 subtask.
2. Separate executable-size gates from post-build package-size gates; package limits run only after `.dmg`/`.deb`/`.rpm` creation and before smoke.
3. Add exact artifact fan-in after all five runner jobs, then one global matrix assertion; do the same for package-smoke artifacts.
4. Build pinned release `xtask` before first use, pin/install `tokei`, and replace open runner placeholders with a closed matrix command.
5. Give no-scroll precedence: if either pane has no scroll range, both directions target zero before bottom/EOF rules; apply to oracles and tests.

## Resolved

IME single mutation path, dependency graph, preview host, one macOS winner, strip/sign/hash identity, installed-package IME removal, type-aware continuations, independent oracles, Ferrite pin, scaffold isolation, closed runner identities, native Wayland, Wry GTK API, typed security, bounded rendering/history, app Interactive, Fedora 43, publication, and non-IDE scope.

## Assessment

The architecture is stable. Remaining concerns are localized execution-order and proof-pipeline contradictions. A Ferrite fork remains the strongest antithesis; the comparator is credible once its tool and artifact pipeline is executable.
