# Vendored dependencies

These crates are **copied into MF66** so the repo is self-contained — the IDE
(`mf66-ui` / `mf66-tcl`, the `ui`/`gui` features) builds without depending on the
external `../locus` working tree. They are the Locus IDE substrate:

| crate | role |
|---|---|
| `macide` | the macOS AppKit + Core-Text shell (`window::run`, the `CgCanvas` renderer, the mailbox). **Carries MF66's native file-dialog change** (`window::open_file_dialog` / `save_file_dialog` — the worker posts a `UiCmd::FileDialog`, the main thread runs `NSOpenPanel`/`NSSavePanel` and replies). |
| `locus-ide-protocol` | the UI-neutral host/worker contract — `DrawCmd`/`DrawBatch`, `UiEvent`. |
| `locus-analysis-model` | a leaf dep of the protocol. |
| `rust-tcl` | the TCL interpreter the agentic control layer (`src/wsdriver.rs`) embeds. |

Origin: `../locus/{macide,locus-ide-protocol,locus-analysis-model,rust-tcl}`.
Inter-crate path deps are sibling-relative (`../crate`), so the layout here
mirrors theirs. To refresh from upstream, re-copy each crate's `Cargo.toml` +
`src/` (and re-apply the macide dialog change if upstream hasn't taken it).
