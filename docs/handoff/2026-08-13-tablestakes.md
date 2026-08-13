# Handoff: Table-stakes follow-up — 2026-08-13

> **Main HEAD**: `f569859` · product tip `ceed4cd` · **Branch**: `main` ·
> **Remote**: Forgejo `git.kyanitelabs.tech:simon/feathermark.git`
> **Status**: Audit leftovers that were table-stakes are shipped or honestly
> blocked. Docs #119–#120 reconciled living status. Not a public release.

## Sequence

Ralplan consensus (rev 2) then IMPLEMENT NOW, one PR per change:

| ID | PR | Result |
|---|---|---|
| A | #110 | `export_html` splices `print_style_block()` then `ExportPage::from_html` |
| B | #111 | ⇧⌘P presents a nonactivating `NSPanel` |
| C1 | #112 | `DocumentSessionCore` parks/swaps ropes; D7 tab close |
| C2 | #113 | Iced strip over `project_tabs` |
| D | #114 | No published iced+wry objc2 pair; skip list kept |
| E | #115 | `xtask quality-probes emit`, 14 ids, `attested: false` |
| — | #108 #109 | AdapterCommitId newtype; Linux M8 leftovers |
| — | #116 | Autosave store inherited onto every tab |
| — | #117 | File Open uses D4 (`adopt_opened_document`) |
| — | #118 | Active tab uses `INK` |
| docs | #119 | Living docs + S+ README + AGENTS crate names |
| docs | #120 | Linux daily-driver plan + host rule |

## Frozen files

Unchanged through this work:

```
crates/rutile-core/src/render.rs
crates/rutile-core/src/security.rs
crates/rutile-types/src/safe_link.rs
```

## Not shipped (do not claim)

- Linux visual tab strip / Window tab menu
- D8 restore of every open path
- VoiceOver spoken output / idle-soak RSS as `passed: true`
- Removing `deny.toml` `bans.skip`
- Publication or readiness `ready: true`

## Next work (if asked)

1. Linux daily-driver parity on a Linux host — `docs/plan/linux-parity.md`
   (P0 File/CLI, then P1 GTK strip, then P2 view/palette/recents).
2. Physical GUI quality-probe night (`QUALITY_PROBE_IDS`).
3. Re-check iced crates.io when ≥0.15 publishes.
4. D8 `open_tabs` session restore.

Live snapshot: `docs/handoff/current-state.md`.
