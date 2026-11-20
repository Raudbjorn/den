# IRONMAW Oxide tokens (committed copy)

`oxide.tokens.json` is a byte-for-byte copy of IRONMAW's resolved Oxide tokens.

| | |
|---|---|
| Upstream repository | <https://github.com/Raudbjorn/IRONMAW> |
| Upstream path | `src/lib/tokens/resolved/oxide.tokens.json` |
| Upstream commit | `2bf753eda6d48fd4b300dfdc7aece15ba7ed474f` (2026-11-11) |
| Licence | MIT, copyright (c) 2026 Sveinbjörn Geirsson |

Do not hand-edit it. To take a newer IRONMAW, copy the file again from a commit and
update the commit above:

```bash
git -C <IRONMAW> show <commit>:src/lib/tokens/resolved/oxide.tokens.json > assets/tokens/oxide.tokens.json
```

## How it becomes den's palette

`build.rs` reads this file on every build where it changed and writes
`$OUT_DIR/oxide_palette.rs`, which `src/palette.rs` includes. The generator itself is
`build/palette_gen.rs` (pure; `tests/palette_generation.rs` exercises it). Nothing reads
IRONMAW's checkout at build time.

The generator's slot table (`SLOTS` in `build/palette_gen.rs`) is den's design, not
IRONMAW's:

- which token fills each of the engine's 27 palette slots, including the slots IRONMAW
  has no token for (`accent_2` and `info` take structure tones; the `syn_*` slots borrow
  brand roles and only matter if den ever shows code);
- the 16-colour fallback for each slot. The tokens say nothing about 16-colour
  terminals, and nearest-colour matching breaks IRONMAW's rules there (rust and amber
  both land on yellow; the selected fill lands on black, the page colour). So each slot
  names an ANSI colour chosen by role: teal → cyan, rust → red, amber → yellow,
  slate → dark grey, selected → blue.

256-colour terminals use the token colours, quantised to the nearest xterm colour.
