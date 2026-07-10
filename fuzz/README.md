# FeatherMark fuzz targets

`preview_event` exercises the real bounded protocol decoder and is an evidence-bearing Task 1A
target.

`render_markdown` and `source_blocks` are build-only placeholders. The approved plan asks Task 1A
to fuzz rendering and source-block invariants, but the implementations that own those invariants
are deliberately sequenced in later tasks. Task 1A previously used local toy implementations;
those could pass without testing FeatherMark and have been removed.

An Architect must resolve the task-ownership contradiction, followed by Critic review, before
either placeholder may be described as invariant or fuzz evidence. A successful `cargo fuzz
build` for those two targets proves only that their harness entry points compile.
