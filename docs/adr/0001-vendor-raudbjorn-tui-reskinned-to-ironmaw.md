# Vendor raudbjorn-tui and re-skin it to IRONMAW Oxide

den's terminal components are copied from `raudbjorn-tui` (design-system @ `464835c`) rather than depended on, because adopting IRONMAW changes the templates, not just the palette: IRONMAW assigns colour by job (teal = actionable, rust = the one action / you are here, amber = caution, slate = structure), whereas raudbjorn-tui uses its teal `accent` for focus and selection. The palette is generated from IRONMAW's resolved DTCG tokens, never hand-edited.

## Considered Options

- **Depend on raudbjorn-tui unchanged and swap the palette only**: rejected; yields IRONMAW colours with raudbjorn semantics.
- **Add semantic roles (actionable / current / caution / structure) upstream in design-system first, then depend via git**: the right long-term home, deferred until den has proven the mapping. Revisit when the vendored copy drifts painfully.

## Consequences

Upstream raudbjorn-tui fixes do not flow into den automatically; the recorded source commit is the baseline for any manual re-sync.
