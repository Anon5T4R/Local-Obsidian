# App icon

The Local Notebook icon in the suite's "night" style: a dark tile with a violet rim and a folder holding Markdown
pages, with `.md` drawn as paths. The Android app uses the same drawing (`android/art/icon.svg`).

## Files

| File | What it is |
|---|---|
| `localobsidian.svg` | the master: the full-bleed 512 px tile (corner radius 100) Windows and Linux use as is |
| `localobsidian-small.svg` | the small-size variant; used for 16, 20, 24 and 32 px only |
| `localobsidian-1024.png` | the master at 1024 px, for stores, README and the issue tracker |
| `hicolor/<size>x<size>/apps/com.localobsidian.app.png` | the freedesktop icon theme tree (16 to 512 px) and the runtime window icon (256 px) |
| `hicolor/scalable/apps/com.localobsidian.app.svg` | the scalable theme entry (a copy of the master) |
| `localobsidian.ico` | the Windows icon: 16, 20, 24, 32, 40, 48, 64, 128 and 256 px, packed by `cargo xtask ico` |

Every one of them is generated from the two SVGs by `packaging/icons.sh`; the outputs are
committed, so building and packaging never need `resvg`.

## The design

The family's frame, unchanged: a full-bleed tile with corner radius 100, a diagonal gradient, a
white glyph at ~70% of the tile and the "Taylor" signature at the bottom. What is this app's own:

- **Hue: violet-indigo** (`#746af0` → `#3730a3`), the one strong hue the suite did not use yet
  (paint amber, slides orange, PDF red, sheets emerald, vector teal, photo fuchsia, office blue).
  The light stop leans 5–8° towards violet on purpose: Local Word is blue, and this is what keeps
  the two apart in a taskbar.
- **Glyph: a note page with a folded corner and the Markdown `#`** knocked out of it through a mask,
  so the tile's gradient shows inside the page. `#` is the app's own metadata language (tags,
  headings), not a generic document mark; the `#` box is centred in the page (y 186–326 of 96–416).
- **A second master for the small sizes.** At 16 px the master's strokes fall under one pixel and
  the page reads as "a page with lines of text" — which is Local Word's mark. `localobsidian-small.svg`
  draws the `#` ~40% heavier and wider, so 16–32 px keep a glyph that cannot be mistaken for the
  Word icon. 40 px and up use the master.

## Regenerating

```
packaging/icons.sh              # Linux/macOS (needs resvg; uses segoesc.ttf when present)
```

On Windows the same steps are `resvg -w N -h N --use-font-file C:\Windows\Fonts\segoesc.ttf
<src> <out.png>` per size (16/20/24/32 from `localobsidian-small.svg`, the rest from
`localobsidian.svg`) and then `cargo xtask ico assets/app-icon/localobsidian.ico <the 9 PNGs>`.

The signature is Segoe Script: without `--use-font-file` (or with fontdb unable to find the family)
resvg falls back to another face and the icon stops being deterministic.

QA of the set: the renders at 16/24/32/48/64/128/256/512, an 8× nearest-neighbour zoom of the 16 px
slice, a contact sheet next to the sibling apps' icons, and the pixel check (corner alpha 0, tile
corners rounded, glyph coverage 15–29% across the sizes) — the renders live in `target/icones-qa/`.
