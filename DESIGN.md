# DESIGN.md: den

The look of den on one page. It gathers decisions recorded elsewhere and points to the file each one comes from. When this page and a source disagree, the **source wins**. Fix this page.

| Source | Owns |
|---|---|
| `assets/tokens/oxide.tokens.json` (IRONMAW Oxide, copied byte for byte; see `assets/tokens/SOURCE.md`) | every colour value |
| `build/palette_gen.rs` (`SLOTS`) | which token fills each palette slot, the 16- and 8-colour colours by role, and the 256-colour pins |
| ADR-0001 | the colour roles |
| ADR-0006, spec 0003 "Look: the den kit" | widgets, panels, the kit |
| `src/layout.rs` | sizes and breakpoints |
| `src/app/view_model.rs` | canonical semantic constants (`SEPARATOR`, `ELLIPSIS`, `ENTER_KEY`); some callers still copy their literal values (section 3) |
| `src/ui/glyphs.rs` | shared glyphs per profile, including how each profile draws those three |
| `src/ui/sidebar.rs` (`repo_glyphs`) | Repo rail marks per profile (section 4) |
| `CONTEXT.md` | the words on screen |

> **Current state.** `main` still draws milestone 1's *Tonal plates* with `.crepus` templates. The design below is the decided target: ratatui widgets through `src/ui/kit/` (ADR-0006, milestone 3), which supersedes Tonal plates. Until the kit lands, the templates are the running code, and this page describes where they are headed. (One visible difference: milestone 1's templates also draw a fork's Upstream target in rust. Spec 0003 drops that.)

## 1. Atmosphere

A dark, quiet workbench for one Repo at a time. It is closer to an instrument panel than a dashboard: charcoal ground, slate structure, and colour spent only where it means something. Teal marks what you can act on, and rust marks the one action and where you are (and, on a launch sheet, an Upstream target). Reference renders: `docs/specs/0003-assets/workbench-160x45.png`, `workbench-200x56.png` and `repos-160x45.png`.

- **One action per screen.** At most one rust Button. The limit is on action Buttons: the cursor tick and an Upstream target are rust too, but they aren't actions.
- **Every meaning is carried by a glyph or words.** Colour adds to them and never replaces them. This must hold in every colour profile and in ASCII (spec 0003, story 37).
- **Honest about what den didn't read.** Partial data is always marked (`⚠ partial`, coverage lines) and never drawn as if complete.

## 2. Colour

### Roles (ADR-0001)

Colour is assigned by **job**, not by component:

| Role | Token | Hex | Means | Never |
|---|---|---|---|---|
| **Teal**: actionable | `accent` | `#3CA0A0` | Waiting On you, focus, a live Job, the active tab, gauges | decoration |
| **Rust**: the one action / you are here | `action` | `#D0602B` | the rust `▌` cursor tick, the single action Button, and an Upstream target's value on a launch sheet beside its "upstream →" label (spec 0001, story 89); nothing else (spec 0003, "The workbench") | a second action Button on a screen; warnings; errors |
| **Amber**: caution | `warning` | `#E2A33B` | partial reads, refusals, a moved head, rate limits | the same ANSI colour as rust |
| **Slate**: structure | `structure` / `border-strong` | `#5F7E8F` | panel borders at rest, rules, structure text (`structure-text` `#8EA6B3`) | content |

### Ground and text

| Token | Hex | Use |
|---|---|---|
| `bg` | `#0F1416` | the page (brand charcoal) |
| `surface-1` / `-2` / `-3` | `#141B1E` / `#192327` / `#1F2B30` | panels, chips, kbd keys, raised fields |
| `selected` | `#17262A` | the selected row's tone |
| `mark-bg` | `#101715` | the mark plate |
| `border` | `#2A3940` | quiet rules |
| `text` | `#D9E1E4` | body |
| `text-strong` | `#F3F6F7` | titles, the Repo's name, the selected item |
| `text-muted` | `#9EB1BA` | secondary facts, reasons |
| `text-faint` | `#86999F` | metadata, hints, "not swept yet" |
| `success` | `#5CB38B` | checked and fine (`✓ complete`) |
| `error` | `#E86052` | failures |

Values are as of the token file's recorded IRONMAW commit (`2bf753e`). Never edit a colour in code: copy a newer token file (`SOURCE.md`) and rebuild, then update this page's tables from it. Nothing checks these hex values against the token file.

### Colour profiles (`src/palette.rs`)

- **Truecolor:** token colours as given.
- **256-colour:** token colours quantised to the nearest xterm colour (`nearest_xterm_index`), except the roles that would merge, which `SLOTS` pins (#128): page 233, mark plate 232, surfaces 234 / 235 / 237, the selected row 236, `border` 238, `error` 167 and `action-hover` 173. Every fill stays its own tone, and an error never takes rust hover's colour.
- **16-colour:** a role-based ANSI palette, *not* nearest-colour, because nearest-colour would merge rust with amber and the selected fill with the page. The mapping is teal → cyan, rust → red, amber → yellow, slate → dark grey, selected → blue.
- **8-colour:** its own role-based palette (#128), because folding the 16-colour one into eight would draw errors in rust and slate in body-text grey. `error` is magenta, never rust's red. Slate lines (`border`, `border-strong`) are blue, apart from text. `info` stays grey, because it is drawn on the selected row, which is blue. `text-faint` and `text-strong` are grey like body text, so words and weight carry those.
- **Mono / no colour:** meaning survives through glyphs, words and position, plus the engine's attribute cues (`mono_cue` in the vendored engine): the selected row in reverse video, the mark plate dim, `border-strong` bold, and rust hover bold and underlined (`tests/palette_profiles.rs`). The cues hang on the engine's colour sentinels, so the kit has to keep them or replace them.

## 3. Type and text

- **The terminal's monospace font.** den sets no font. The reference renders use Iosevka, which is IRONMAW's `font-mono`.
- **Emphasis by weight and tone, not size.** Panel titles and section labels are uppercase (`NEXT STEPS`, `WAITING ON`). Titles and the Repo name are `text-strong`; reasons are muted.
- **The lockup:** `D  E  N`, letter-spaced, with its tagline, on the Repo screen (spec 0003). The reference render's tagline is "Next Steps for GitHub work, one Repo at a time".
- **Words** come from `CONTEXT.md` (Next Step, Waiting On, Live Job, Landing Group, …). Its `_Avoid_` lists apply on screen too.
- **Semantic text has two layers.** The ViewModel should use `SEPARATOR` between sentence parts, `ELLIPSIS` for work under way and `ENTER_KEY` for key hints, from `src/app/view_model.rs`. It never picks a glyph profile or marks width-dependent cuts. Their profile forms are listed once in section 4.
- **Convert before measuring.** The renderer must apply `Glyphs::text` before measuring, wrapping or cutting semantic text, using terminal-cell widths (`src/ui/text.rs`), not character counts. `Glyphs::fit` combines conversion with a cut between graphemes. When inserting a separator, ellipsis or Enter hint, use the appropriate constant or glyph field rather than spelling its form outside a per-profile table (`src/ui/glyphs.rs`, or `repo_glyphs` in `src/ui/sidebar.rs`). This rule is about those semantic uses, not hyphens in words, command flags, borders or gauge fills.
- **Truncation belongs to the renderer.** `text::fit` / `text::end_with` append `glyphs.ellipsis` when cutting. If the slot is narrower than the mark, only its prefix fits: ASCII widths 1 and 2 produce `.` and `..`; width 0 produces nothing. Such a fragment is not an unambiguous standalone status glyph. An ellipsis already in work-under-way or Button copy does not mean the text was cut.

**Known gaps in the current templates.** These are requirements, not a claim that every path already complies. ViewModel callers in `src/app/view/launch.rs`, `src/app/view/footer.rs` and `src/app/view/scope.rs` still copy semantic literals; changing a constant requires auditing those copies too. `src/ui/launch.rs` draws raw text and inserts a literal separator for inline field notes (#151). The sidebar label in `src/ui/sidebar.rs`, list labels / Next Step names / Waiting On text in `src/ui/list.rs`, and header segment labels in `src/ui/header.rs` also bypass conversion. The header budgets its scope and cuts segment labels by character count. Unconverted semantic characters reach the engine's `ascii_fallback` as `?`, which already means "unattributed". This documentation change neither fixes those paths nor installs a central conversion guard.

## 4. Glyphs

Every colour-coded meaning is also carried by a glyph or words (story 37). The first table lists shared glyphs from `src/ui/glyphs.rs`; Repo rail marks follow it. Focus (a teal panel border and title) and the active tab have no glyph yet: in mono and no-colour the kit must carry them another way, and how is undecided.

| Meaning | Unicode | ASCII |
|---|---|---|
| current row (rust tick) | `▌` | `>` |
| text-input cursor | `▏` | `\|` |
| incomplete / partial | `⚠` | `!` |
| unattributed | `?` | `?` |
| complete | `✓` | `+` |
| Live Job | `●` | `*` |
| Job running / idle | `▸` / `◐` | `@` / `.` |
| worktree gone / unknown | `○` / `◌` | `o` / `~` |
| Linear | `◇` | `[L]` |
| separator, ellipsis, Enter | `·` `…` `⏎` | `-` `...` `RET` |

**Repo rail** (`src/ui/sidebar.rs::repo_glyphs`). These marks are contextual: the Repo's current scope, a Job's state and an empty count occupy different places. They are not globally unique identifiers. In particular, the rail's "none" mark comes from `repo_glyphs`, not the semantic separator constant.

| Meaning in the rail | Unicode | ASCII |
|---|---|---|
| fork | `⑂` | `Y` |
| Upstream target | `upstream →` | `upstream >` |
| archived | `▣` | `#` |
| current scope | `●` | `*` |
| no count | `·` | `.` |

**Undecided.** The target screens use glyphs that `glyphs.rs` doesn't define yet, and their ASCII forms aren't chosen. The engine's `ascii_fallback` would draw each as `?`, which already means "unattributed".

| Meaning | Unicode | ASCII |
|---|---|---|
| a Main Clone is found (the Repo screen's CLONE column; the rail's identity block) | `◆` | undecided |
| search (the filter Field in the reference renders) | `⌕` | undecided |
| Job state unknown | spec 0003's JOB column writes `? unknown`; `glyphs.rs` has `◌` | `~` in `glyphs.rs`; the spec's `?` would collide with "unattributed" |

In the kit's ASCII profile, borders are `+-|` and chips are `[text]` (spec 0003). Row selection uses `Glyphs::tick`; a Field's text-input cursor uses `Glyphs::cursor`. Take their forms, and the separator, ellipsis and Enter forms, from the tables above. A form can occupy several cells; measure after conversion.

## 5. Components: the den kit (ADR-0006; spec 0003)

ratatui widgets wrapped in `src/ui/kit/`. Each takes its style from the theme (`kit/theme.rs`), which resolves IRONMAW roles for the active profile.

| Component | Built from | Look |
|---|---|---|
| **Panel** | `Block`, rounded, `merge_borders(Fuzzy)` | uppercase title top-left, status title top-right (count and explicit `complete`, with marks from section 4); slate border at rest, teal when focused, as is the focused panel's title in the reference renders |
| **DataTable** | `Table` + `Scrollbar` | header row, group rows (`WAITING ON YOU`, `RECENT`), rust `▌` cursor on the focused table, `selected` tone on the row, empty and coverage states |
| **Chip** | styled span | the Next Step label on a `surface` fill: teal when Waiting On you, structure text otherwise |
| **TabBar** | `Tabs` | active tab teal on charcoal |
| **Field** | `app/editor.rs` | the filter fields |
| **Gauge** | `LineGauge` | teal fill, with its words beside it (`119 of 163 Repos swept`) |
| **KeyBar** | kbd chips | key hints at the bottom left, the glyph legend at the bottom right |
| **Button** | rust span | **the one action** (`c  Reopen the Live Job`, followed by the profile's ellipsis from section 4) |
| **Sheet** | `Clear` + Panel | overlays: launch sheet, detail sheet |
| **Bear** | `ui::bear` | the lettered SVEINBJORN bear (`assets/brand/bear-mark.svg`), drawn with kitty graphics where the terminal answers to it, half-blocks in truecolor otherwise, and left out in every other colour profile (`Graphics::choose`; `DEN_GRAPHICS` overrides) |

**Panel status:** the table describes content, not a verified ASCII border layout. Spacing must distinguish the status from corners and rules; that still needs a mono ASCII rendering fixture when the kit lands.

**Bear rule:** the full bear where there is room, otherwise no bear. Never an emblem or a shrunken bear (spec 0003, story 36). On the Repo screen it needs at least `REPO_LIST_MIN_ROWS` (10) table rows beside it. In the rail it must fit together with the identity block and every section without scrolling.

## 6. Layout (`src/layout.rs`)

| Constant | Value |
|---|---|
| Designed for | 160×45 (`DESIGN_COLS` × `DESIGN_ROWS`); to be verified at 200×56 too (spec 0003) |
| Minimum | 80×24 (`MIN_COLS` × `MIN_ROWS`); smaller still draws, and says so |
| Detail pane | at ≥ 110 columns (`WIDE_LAYOUT_MIN_COLS`), 25% clamped to 32–48; below that, Enter (profile form in section 4) opens the detail as a Sheet |
| Rail | 22% clamped to 30–40 columns |

- **Repo screen:** the top bar (`den ┊ Open a Repo`, the viewer and `gh` sign-in), the bear and lockup, one filter Field, the Linear pointer line, one REPOS DataTable (owner tabs; RECENT, then BY LAST PUSH), the Sweep Gauge, and the KeyBar. Content is centred and capped at `REPO_SCREEN_MAX_COLS` (140; a spec 0003 constant, not yet in `src/layout.rs`).
- **Workbench:** the top bar (`den ┊ owner/name`, and on the right the *elsewhere* line: `N requested of you`, `◇ N Linear Issues`, `g Repos`), the rail (bear, identity block, WORK and NOT TRACKED HERE sections), the NEXT STEPS DataTable grouped by Waiting On, and the DETAIL Panel (NEXT STEP, WAITING ON, WHY, ALSO, JOB, …; `PgUp`/`PgDn` pages it) with the one rust action at its foot, and the KeyBar.
- Panel borders cost two rows and two columns each, so the 80×24 minimum must be re-verified once the kit lands (ADR-0006).

## 7. Do and don't

- **Do** give every new meaning a role, and a glyph or words, before it gets a colour.
- **Do** take colours from the theme's roles. Never write hex or ANSI colours in UI code.
- **Do** mark partial data in amber with `⚠`, and say what wasn't read.
- **Don't** use rust for anything but the one action Button, the cursor tick and an Upstream target.
- **Don't** let amber and rust share an ANSI colour, or the selected fill match the page.
- **Don't** shrink or crop the bear. Omit it.
- **Don't** bring back Tonal plates (regions told apart by tone alone, slate sutures, the `◢` chamfer). ADR-0006 retired them; regions are bordered Panels now.
- **Don't** hand-edit `oxide.tokens.json` or the generated palette.

## 8. Testing the look

- `TestBackend` buffer snapshots at fixed sizes and profiles are milestone 3's rendering seam (ADR-0006; spec 0003 "Testing Decisions"). Until the kit lands, `tests/frame.rs` smoke-renders the current frame.
- `tests/palette_generation.rs` checks the compiled palette against the token file, and that every 8-colour choice is one of the eight and every 256 pin an xterm colour. `tests/palette_profiles.rs` checks: rust apart from amber and teal in 16 colours; the four extra roles distinct, and the selected fill apart from the page, in 256 and 16 colours; in 8 colours, error apart from rust and slate lines apart from text; in 256 colours, every fill and error apart; in every colour profile, nothing drawn on the selected row disappearing into it; and the mono cues.
