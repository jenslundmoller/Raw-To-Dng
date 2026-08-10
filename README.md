# RAW to DNG

A GTK4 batch converter that turns camera raw files into DNG on Linux, and then
**proves** the result matches the original before accepting it.

Free raw-to-DNG conversion on Linux already works from a terminal via
[dnglab](https://github.com/dnglab/dnglab). This is a native desktop
application over the same engine, built for one workflow: convert a large batch,
confirm nothing went wrong, then delete the originals with confidence.

## Why verification

"The conversion did not report an error" is a weaker claim than it sounds. A file
can be written successfully and still be wrong.

After writing, this app decodes the result and compares it against the source:

- **Every raw sample, bit-for-bit** — not a checksum, not a size heuristic
- **Colour** — colour matrices per illuminant, white balance, black and white levels
- **Geometry** — CFA layout, components per pixel, active area, crop area
- **Shot data** — capture time, exposure, ISO, focal length, camera serial, GPS
- **Lens** — make, model, serial and specification, read from the file's own tags

Only a file that passes all of that is renamed to `.dng`. Verification runs on a
temporary file *before* the rename, so a `.dng` never exists in an unverified
state. Anything that fails is discarded and reported.

This costs roughly 30% more time and rules out truncation, silent corruption,
wrong pixel data and dropped metadata.

A file-size check was considered and rejected: compression ratio depends on image
content, so a flat frame can legitimately compress very small while a corrupt file
can land at a plausible size. The test suite demonstrates the gap by flipping bits
deep inside a valid DNG — same size, intact header, caught only by comparing
samples.

## Safety rules

These are invariants, not options:

1. **Originals are never deleted, moved or modified.** There is no "delete
   originals" setting. Conversion is strictly additive.
2. **Atomic writes.** Output goes to a hidden `.part` file and is renamed only on
   success, so a crash or cancellation cannot leave a truncated `.dng` that later
   passes for a good one.
3. **Never silently overwrite.** An existing target is skipped unless you opt in.

## Camera support

Every format [rawler](https://github.com/dnglab/dnglab) supports — 724 camera
models across 19 manufacturers, including Canon, Sony, Panasonic, Nikon, Fujifilm,
Olympus, Pentax, Leica, Hasselblad, Phase One and Samsung. The accepted extension
list is queried from rawler at runtime rather than hardcoded, so it tracks the
library.

DNG is deliberately **not** accepted as input, since it is this app's output.

### Known limitation

Nikon's **High Efficiency** modes (HE and HE\*) on the Z9, Z8, Zf, Z6III and ZR are
not supported. They use intoPIX TicoRAW, which is patented, so an open-source
decoder cannot legally ship without a licence. Those bodies shooting Lossless
Compressed work normally.

## Building

Requires Rust and the GTK4 development libraries:

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
cargo build --release
```

## Installing

```bash
./install.sh
```

Installs entirely under `~/.local` — no root needed. The binary goes to
`~/.local/bin`, with an icon and desktop entry so the app appears in your
application grid and under *Open With* for raw files.

The installer records your existing default applications for every raw format it
registers and restores anything it would have displaced, so it will not hijack
double-click from your raw editor.

```bash
./install.sh --uninstall
```

## Using it

Drag files or folders onto the window, or use **Add Files** / **Add Folder**.
Folders are walked recursively.

Output goes to a chosen root — `~/Pictures/DNG` by default — with the source
folder structure mirrored beneath it. The output root is excluded from folder
walks, so it can safely live inside your pictures tree.

Failed files are highlighted, and a header toggle filters the queue down to just
those.

## Tests

```bash
cargo test
```

Tests that need real camera files are skipped unless you point them at some:

```bash
NEFTODNG_TEST_DIR=/path/to/folder/of/raws \
NEFTODNG_TEST_NEF=/path/to/one.nef \
NEFTODNG_TEST_PORTRAIT_NEF=/path/to/a/portrait.nef \
cargo test
```

The portrait fixture matters: orientation handling differs between landscape and
rotated shots, and a regression there once rejected every portrait photograph.

## Design notes

`docs/plans/2026-08-10-nef-to-dng-design.md` records the architecture and the
reasoning behind each decision, including the measurements behind every tolerance
and the cases where a library reported something the files did not actually say.

## Licence

LGPL-2.1, see [LICENSE](LICENSE). It links [rawler](https://crates.io/crates/rawler),
which is also LGPL-2.1.

No Nikon SDK, no Adobe SDK and no intoPIX code is used. Raw decoding is
reverse-engineered, as it is in dcraw, LibRaw, darktable and RawTherapee.

## Provenance

This code was written with AI assistance (Claude). Worth knowing if you plan to
contribute, redistribute or package it — notably, [Flathub does not accept
AI-assisted applications](https://docs.flathub.org/docs/for-app-authors/requirements).

It has been verified against real camera files rather than assumed correct, but it
has only been exercised end-to-end on Nikon Z 6 raws. Other manufacturers rest on
rawler's camera database and the extension tests. If you use it on another brand,
the verification step is the safety net: a model-specific problem surfaces as a
verification failure, not a silently wrong DNG.
