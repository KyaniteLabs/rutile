# Vendored pulldown-cmark 0.13.4 + fm1 patch

Byte-for-byte copy of the crates.io `pulldown-cmark-0.13.4` source with one
patch (`RUTILE PATCH (fm1)` in `src/parse.rs`):

- **Bug:** `Parser::next` panics (`Option::unwrap()` on `None`,
  `src/parse.rs:2199` in the unpatched crate) when a tight paragraph has no
  children — reachable with input like `-\t[`]:I\r\t\t` (a list item whose
  paragraph is only a link-reference definition). With Rutile's
  `panic = "abort"` release profile this aborts the whole app, so any hostile
  Markdown document is a denial of service.
- **Found by:** 30-minute `render_markdown` fuzz campaign, 2026-07-10
  (crash artifact `fuzz/artifacts/render_markdown/`; regression test
  `crates/rutile-core/tests/render.rs`
  `fuzz_regression_list_item_link_reference_definition_renders_without_panicking`).
- **Status upstream:** present in 0.13.0 through 0.13.4 and on `master` as of
  2026-07-10. The fix mirrors the crate's own empty-TightParagraph handling in
  the `None` branch of the same function. TODO: submit upstream and drop this
  vendor copy once a fixed release ships.
- **Wiring:** `[patch.crates-io]` in the workspace `Cargo.toml` and
  `fuzz/Cargo.toml`. Version stays `0.13.4` so the patch applies transparently.
