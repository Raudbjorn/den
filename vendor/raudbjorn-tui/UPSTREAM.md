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

## Re-syncing

Upstream fixes do not flow in automatically. To compare against a newer upstream:

```bash
git -C <design-system> diff 464835c <new-commit> -- crates/raudbjorn-tui/src/component.rs \
  crates/raudbjorn-tui/src/profile.rs crates/raudbjorn-tui/src/theme
```

then re-apply den's patches by hand and update the commit above.
