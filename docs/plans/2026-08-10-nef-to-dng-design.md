# NefToDng — design

**Date:** 2026-08-10
**Status:** validated, ready to implement

A GTK4/libadwaita batch converter that turns Nikon NEF files into DNG on Linux.

## Why this exists

Free NEF→DNG conversion on Linux already works, but only from a terminal. `dnglab`
is CLI-only; Adobe's converter needs Wine. The gap is a native GUI with a queue,
progress and drag-and-drop — not the conversion itself.

## Licensing position

NEF is proprietary and has no published specification, but reading it requires no
licence from Nikon. Every open-source decoder (dcraw, LibRaw, darktable,
RawTherapee, dnglab) is reverse-engineered, and no raw decoder has ever been sued.
DNG is the opposite: Adobe publishes the spec under a royalty-free patent grant.

One real exception: the **HE / HE\*** compression modes on the Z9, Z8, Zf, Z6III and
ZR use intoPIX **TicoRAW, which is patented**. Reverse engineering does not cure a
patent, so those modes need an intoPIX licence and stay out of scope.

**This does not affect us.** The target camera is a **Nikon Z 6**, which writes
lossless-compressed NEF — fully supported, no encumbrance.

The app links `rawler` (LGPL-2.1), so it ships under **LGPL-2.1 or GPL-3.0**.
No Nikon SDK, no Adobe SDK, no intoPIX code.

## Feasibility — already proven

A spike linking `rawler` 0.7.2 converted all 8 sample NEFs from
`~/Pictures/2026/07/21/`:

| Metric | Result |
| --- | --- |
| Success rate | 8/8, zero failures |
| Speed | ~0.43 s/file (47 MB NEF, single thread) |
| Size | 47 MB → 28 MB (~40% smaller) |
| Output | Valid DNG 1.6.0.0 |
| Camera detected | Nikon Z 6 |
| Colour | ColorMatrix1 + ColorMatrix2, illuminants Std A + D65 |
| White balance | AsShotNeutral preserved |
| Image data | 6064×4040 lossless JPEG + 1024×681 preview + 180×120 thumb |

The per-model calibration database — the genuinely hard part of raw conversion, and
the reason writing a converter from scratch is a multi-year commitment — is already
solved inside `rawler` for this camera. **The remaining work is entirely GUI.**

## Architecture

Single Rust binary. GTK4 + libadwaita for native GNOME integration on Wayland.
`rawler` linked directly rather than shelling out to the `dnglab` binary: one
self-contained executable, no runtime dependency, real cancellation, and thumbnails
straight from the embedded previews.

Three layers:

**UI thread** — `AdwApplicationWindow` in an `AdwToolbarView`. The queue is a
`GtkListView` over a `GioListStore` of `FileRow` GObjects (filename, camera, status,
progress). Because rows are GObjects with properties, mutating a row redraws it
automatically.

**Coordinator** — a `glib::MainContext` channel carrying
`Msg::{Started, Progress, Done, Failed}`. The single point where worker state meets
UI state, which keeps GTK's main-thread rule trivially satisfied.

**Worker pool** — `rayon` across `num_cpus` threads (user-adjustable). Each task calls
`rawler::dng::convert::convert_raw_file` into a `BufWriter`. An `AtomicBool` cancel
flag is checked between files.

At ~0.4 s/file, per-file progress granularity is sufficient; no progress callbacks
need to be threaded into rawler's internals.

## Interaction

**Adding files** — drag-and-drop (`GtkDropTarget` + `GdkFileList`), "Add Files…" via
the XDG portal chooser, and "Add Folder…" which walks recursively with a depth cap,
filters `.nef`/`.nrw` case-insensitively, and de-duplicates against the existing queue.

**Queue** — one row per file. On add, a metadata-only pass reads camera model and
dimensions without a full decode, so rows appear instantly for large batches.
Thumbnails come from the embedded preview, loaded lazily off-thread. Status runs
queued → converting → done ✓ / failed ✗. Failed rows show their error inline and
remain in the list.

**Reporting** — a finished row shows what it cost: `Done · 42,4 MB → 24,6 MB`.
When the batch ends, an `AdwAlertDialog` states how many files converted, how many
were skipped or failed, and the total space saved. Sizes go through
`glib::format_size`, so separators and units follow the user's locale rather than
being hardcoded. A batch whose DNGs came out *larger* says so plainly instead of
reporting a negative saving.

**Options** — an `AdwPreferencesDialog` mapping onto `ConvertParams`: compression
(Lossless / Uncompressed), embed original NEF, crop mode, preserve mtime, optional
Artist string. Defaults are good enough to ignore entirely.

## File-safety rules

These are invariants, not preferences.

1. **Sources are never deleted, moved or modified.** There is no "delete originals"
   option at any point. Conversion is strictly additive.
2. **Atomic writes.** Convert to `.NAME.dng.part` in the destination, sync, rename.
   A crash, cancel or full disk never leaves a half-written DNG that could later
   pass for a good one.
3. **Never silently overwrite.** An existing target means the row is skipped and
   marked "already exists". Overwriting requires an explicit toggle.
4. **Free space is checked up front**, estimating output at 0.75 × input bytes.

**Output layout — mirrored tree.** One output root (e.g. `~/Pictures/DNG`); the
source structure is recreated beneath it. Dropping a folder sets the base to that
folder's *parent*, so `~/Pictures/2026` produces `<out>/2026/07/21/AMB_2657.dng`.
Individually-selected files land flat at the output root. The output root is excluded
from folder walks, so it may safely live inside the source tree. Destination
directories are created lazily, only when a file in them succeeds.

## Panic isolation

dnglab's README states the project deliberately prefers panics over defensive error
handling on malformed input. In-process linking means one corrupt NEF could otherwise
kill the whole app mid-batch. Every conversion therefore runs inside
`std::panic::catch_unwind`, turning a panic into a failed row.

This is the specific cost of choosing in-process over subprocess isolation, and it is
about five lines to neutralise.

## Testing

- **Integration:** convert a fixture NEF and assert DNGVersion, both ColorMatrices,
  AsShotNeutral, and the SubIFD layout — the tag-validation script from the spike,
  promoted to a test.
- **Unit:** path mirroring, base-path derivation, queue de-duplication, conflict
  rules. This is the logic that could quietly corrupt an archive, so it gets covered
  without touching the GUI.
- **Manual:** cancel mid-batch and confirm no `.part` files and no partial DNGs.

## Deviations from this design, as built

Two things were cut from v1 and are recorded here rather than silently dropped:

- **Thumbnails.** The queue shows filename and status, not an embedded preview.
  Lazy preview extraction is the single largest complexity add (async decode,
  `GdkTexture` upload, scroll-aware cancellation) and the app is useful without
  it. The row layout leaves space for it.
- **Per-row progress bars.** At ~0.4 s/file a per-file percentage would be
  theatre, so an active row shows a spinner and the batch shows one overall
  progress bar. This is more honest about what is actually known.

One design claim was **not** confirmed in practice: a truncated real NEF produced
a clean decode error rather than a panic, so `catch_unwind` was never observed
firing. It stays in as defence-in-depth, because dnglab's README documents
panicking as intended behaviour on malformed input, but it is untested against a
real panic.

## Explicitly out of scope

HE/HE\* (patent-encumbered), raw editing or development, non-Nikon formats in the UI
(rawler supports them; the app stays focused), and any form of destructive operation
on sources.

## Build prerequisites

Present: Rust 1.94, cargo, gcc, pkg-config.
Needed: `sudo apt install libgtk-4-dev libadwaita-1-dev`
Optional, for debugging: `sudo apt install libimage-exiftool-perl`
