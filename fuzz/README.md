# Rutile fuzz targets

> **Status: Current.** The fuzz workspace uses a pinned nightly toolchain and exercises four production protocol/core surfaces.

| Target | Production surface | Main invariants |
|---|---|---|
| `preview_event` | `rutile-protocol` decoder | Bounded typed decode, loaded-revision equality, scroll bounds, canonical safe links |
| `render_markdown` | Markdown renderer and source-block validator | Output byte caps, balanced allowlisted HTML, fixed internal assets/CSP, valid source mapping |
| `source_blocks` | Source-block builder/validator | Ordered non-overlapping blocks, byte caps, continuation typing |
| `html_to_markdown` | Smart-paste converter plus renderer | Bounded conversion, no executable HTML/schemes, safe re-rendering |

Run deterministic smoke passes from the repository root:

```sh
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz preview_event -- -runs=10000 -seed=1
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz render_markdown -- -runs=10000 -seed=1
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz source_blocks -- -runs=10000 -seed=1
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz html_to_markdown -- -runs=10000 -seed=1
```

Corpus and crash artifacts live under `fuzz/corpus/<target>/` and `fuzz/artifacts/<target>/`. Do not describe a run as evidence-bearing unless its command, seed/run budget, toolchain, source revision, and retained artifacts are recorded with the result.
