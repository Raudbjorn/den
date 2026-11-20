# Vendored: raudbjorn-tui engine

| | |
|---|---|
| Upstream repository | <https://github.com/Raudbjorn/design-system> |
| Upstream path | `crates/raudbjorn-tui` |
| Upstream commit | `464835c25bad757de794e5889f96e5ccb193f474` (2026-09-08, "docs: fix stale test command and add Rust TUI coverage to CLAUDE.md (#41)") |
| Licence | MIT, copyright (c) 2026 Sveinbjörn Geirsson. Full text in [`LICENSE`](LICENSE), copied from the upstream repository root. |
| Why vendored | [ADR-0001](../../docs/adr/0001-vendor-raudbjorn-tui-reskinned-to-ironmaw.md) |

## What was copied

Only the engine: `ComponentRenderer` and the template store (`src/component.rs`), the
sentinel-colour palette (`src/theme/`), and the colour and glyph profiles
(`src/profile.rs`), plus `src/lib.rs` and `Cargo.toml`.

Not copied: the 28 gallery components (`templates/*.crepus`), the component catalog and
browser (`src/catalog/`, `src/browser.rs`), the gallery example, and the upstream tests,
which exercise the gallery components. den writes its own templates (ADR-0001).

The first commit that adds this directory holds the copied files byte-for-byte as they
are at the upstream commit. Every later change is a den patch, listed below.

## den patches

See `git log -p -- vendor/raudbjorn-tui` for the exact diff against the verbatim copy.

1. **Engine only.** `lib.rs` no longer declares `browser` and `catalog`; `Cargo.toml`
   drops `crossterm` and `tui-overlay` (only those modules used them) and sets
   `publish = false`. `theme/generated.rs` (upstream's DARK, LIGHT and AMBER themes)
   is removed; den generates its own Oxide palette.
2. **Open template namespace** (`component.rs`). `TemplateFile` is now a caller-named
   value (`TemplateFile::new("frame")`) instead of a closed four-variant enum, and
   `TemplateStore::from_sources` registers any set of files, rejecting empty,
   path-like and duplicate names and reporting every problem with its file.
   `ComponentId`, `load_embedded` and the section-keyed `normalize_*` helpers are
   removed: they served the gallery components, and the normalisers would have
   rewritten any den section that happened to be called `Table`, `Tabs` and so on.
   `ComponentRenderer::render` no longer lets `ctx.virtual_files` override a
   registered template (upstream re-parsed it on every render and silently fell back
   to the store on a parse error). That holds for `include` too: registered files take
   precedence over a caller file at the same path.
3. **IRONMAW palette slots** (`theme/mod.rs`, `profile.rs`). `TerminalPalette` gains
   `selected`, `mark_bg`, `border_strong` and `action_hover`, with sentinels
   `#aa0017`..`#aa001a`. They are substituted in the same pass as the other roles,
   before the profile is applied. Named constants for every sentinel live in
   `theme::sentinel`.
4. **Mono cues** (`theme/mod.rs`, `component.rs`). In the mono and no-color
   profiles, a cell painted with one of those four roles gets a text attribute:
   selected = reversed, mark plate = dim, border strong = bold, rust hover = bold
   and underlined.

## Re-syncing

Upstream fixes do not flow in automatically. To compare against a newer upstream:

```bash
git -C <design-system> diff 464835c <new-commit> -- crates/raudbjorn-tui/src/component.rs \
  crates/raudbjorn-tui/src/profile.rs crates/raudbjorn-tui/src/theme
```

then re-apply den's patches by hand and update the commit above.
