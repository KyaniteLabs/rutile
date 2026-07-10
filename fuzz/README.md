# FeatherMark fuzz targets

`preview_event` exercises the real bounded protocol decoder and is the evidence-bearing Task 1A
target. Its exact input grammar is an eight-byte little-endian loaded revision followed by one
newline-terminated preview-event NDJSON frame. Inputs shorter than eight bytes exercise the
explicit harness error path without calling the decoder. Successful decodes assert revision,
scroll-bound, and canonical-link invariants.

The `corpus/render_markdown/` and `corpus/source_blocks/` directories remain unchanged as
Task-1A-owned reserved, non-evidence seed data. Their former no-op harnesses and bins are removed.
Task 1C alone recreates real harnesses against its typed-render/source-block owner and may then
claim these corpora as evidence.

Pinned evidence command:

```sh
cargo +nightly-2026-07-01 fuzz run --fuzz-dir fuzz preview_event -- -runs=10000 -seed=1
```
