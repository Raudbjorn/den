# Draw with ratatui widgets through den's own kit

den draws its screens with ratatui's own widgets (`Block` with merged borders, `Table`, `Tabs`, `LineGauge`, `Scrollbar`, `Clear`). A small kit in `src/ui/kit/` wraps them, taking every style from IRONMAW roles resolved for the active colour profile. This supersedes ADR-0001's decision to write `.crepus` templates for the vendored `ComponentRenderer`, and milestone 1's "Tonal plates" look (no boxes, slate sutures, one chamfer). ADR-0001's colour roles stand: teal = actionable, rust = the one action / you are here, amber = caution, slate = structure. The vendored palette generation and colour/glyph profiles also stand.

The tonal-plate screen read as text in columns. Regions weren't visibly separated, there were no table columns or fields, and the user asked for an interface with semantic segmentation.

## Considered Options

- **Keep the crepus templates and add borders**: the templates lay out spans, not widgets. Tables, scrolling, tabs and fields would each have to be rebuilt in template form.
- **Use raudbjorn-tui's components from `design-system`**: they are gallery stubs. At design-system `464835c` (still its head on 2026-11-21), `table/rich-cells` renders a row as one unaligned string and the Homelab View truncates its buttons to `[Restar`.
- **ratatui widgets through a den kit**: chosen. ratatui 0.30.2 is already pinned. Its `Block::merge_borders` gives panels real junctions, and `Table` gives headers, columns, highlight and scrolling. A throwaway render of the Repo screen and workbench (`docs/specs/0003-assets/`) confirmed the look before this decision.
- **Build the kit in `design-system` first and depend on it**: the right long-term home, deferred until den's kit has settled, for the same reason ADR-0001 deferred semantic roles upstream.

## Consequences

- `templates/*.crepus`, `ui/templates.rs` and den's use of `ComponentRenderer` go. The crepuscularity dependencies go if nothing else needs them. The vendored crate shrinks to the palette and profile code.
- Rendering becomes testable: `TestBackend` buffer snapshots at fixed sizes and profiles, which milestone 1 left to manual checks.
- Panels have borders, so every screen spends two rows and two columns per panel on them. The 80×24 minimum must be re-verified.
- The kit is den's own. Fixes don't flow to `design-system` until it is upstreamed.
