# The bear (committed copy)

`bear-mark.svg` is a byte-for-byte copy of IRONMAW's main bear, the same artwork as
`sveinbjorn-bear-transparent.svg` (spec, "Look and layout": the main bear everywhere;
the emblem is not used).

| | |
|---|---|
| Upstream repository | <https://github.com/Raudbjorn/IRONMAW> |
| Upstream path | `assets/brand/bear-mark.svg` |
| Upstream commit | `2bf753eda6d48fd4b300dfdc7aece15ba7ed474f` (2026-11-11) |
| SHA-256 | `0e91b5a7c9dd2848ce629406bd50d59d09dd2dcac7023751160aec126637acbc` |
| Licence | MIT, copyright (c) 2026 Sveinbjörn Geirsson |

Do not hand-edit it. To take a newer bear, copy the file again from a commit and update
the commit and hash above:

```bash
git -C <IRONMAW> show <commit>:assets/brand/bear-mark.svg > assets/brand/bear-mark.svg
```

## How den draws it

`src/ui/bear.rs` embeds the file (`include_bytes!`), rasterises it once with resvg, and
hands each placement (header, splash, empty state) its own ratatui-image protocol state.
In kitty it is drawn with the kitty graphics protocol; elsewhere with half-blocks.
Nothing reads IRONMAW's checkout at build or run time.
