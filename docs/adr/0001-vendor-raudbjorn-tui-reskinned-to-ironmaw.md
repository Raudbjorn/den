# Vendor raudbjorn-tui's engine and write den's own IRONMAW Oxide templates

den vendors `raudbjorn-tui` (design-system @ `464835c`) rather than depending on it, because adopting IRONMAW changes the templates, not just the palette. IRONMAW assigns colour by job: teal = actionable, rust = the one action / you are here, amber = caution, slate = structure. raudbjorn-tui uses its teal `accent` for focus and selection. The palette is generated from IRONMAW's resolved DTCG tokens, never hand-edited.

The prototype (branch `prototype/ironmaw-tui`) showed that what is worth vendoring is the **engine**: `ComponentRenderer`, the sentinel-colour palette mechanism, and the colour/glyph profiles. raudbjorn-tui's 28 components are gallery stubs (its Table renders a single string), so den writes its own `.crepus` templates and reuses none of them.

## Considered Options

- **Depend on raudbjorn-tui unchanged and swap the palette only**: rejected; yields IRONMAW colours with raudbjorn semantics, and the components are not production components anyway.
- **Add semantic roles (actionable / current / caution / structure) upstream in design-system first, then depend via git**: the right long-term home, deferred until den's templates have settled. Revisit when the vendored copy drifts painfully.

## Consequences

- Upstream raudbjorn-tui fixes do not flow into den automatically; the recorded source commit is the baseline for any manual re-sync.
- Two small changes to the vendored engine are expected: an open template namespace (`TemplateFile` is a closed enum, so the prototype rode the `Views` slot via a virtual-file override), and palette slots for IRONMAW's `selected`, `mark-bg`, `border-strong` and `action-hover`. The prototype's second sentinel pass runs after profile quantisation, which is wrong for non-truecolor profiles.
