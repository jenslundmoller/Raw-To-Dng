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

**Failures** — a toggle appears in the header once something fails and filters the
queue down to just those files. Its label, visibility and whether filtering is
permitted are all **derived from the queue** by `failure_indicator`, re-evaluated
at every point that changes the queue: adding files, clearing, starting a run, and
each finished file. Tracking that state alongside the queue instead produced a bug
where clearing left a stale "2 failed" showing with the filter still active, so
newly added files rendered as an empty list beneath a non-empty queue.

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

## Verification

The intended workflow is to convert a large batch and then delete the NEFs, so
"finished without errors" has to mean "provably correct", not "nothing crashed".

After writing, the `.part` file is decoded again and every raw sample is compared
against the source. Only a bit-for-bit match is renamed to `.dng`; anything else
is discarded and reported as a failure. Verification runs **before** the rename,
so a `.dng` never exists in an unverified state — there is no window in which a
suspect file could be mistaken for a finished one.

Measured on a Nikon Z 6 file: 6064x4040, 24,498,560 samples, identical bit-for-bit
under both `CropMode::Best` and `CropMode::None`. Sequential cost rose from about
422 ms to about 545 ms per file, roughly 29%.

**Why not a file-size heuristic.** Compression ratio depends on image content, so
a flat or dark frame can legitimately compress very small while a corrupt file can
land at a plausible size. A threshold would both false-alarm and miss real damage.
The test `a_silently_corrupted_dng_is_rejected` demonstrates the gap: bits flipped
deep inside a valid DNG leave the file exactly the same size with an intact header,
and are caught only by comparing samples — 56,710 of 24,498,560 differed.

Sample comparison is by exact bits, including for floating-point data. A conversion
that merely rounds to something close has still lost data.

### Metadata

Identical pixels are not sufficient: a file with correct samples but a wrong white
level or colour matrix renders wrongly while passing any sample comparison. So the
same two decodes are also compared on everything a renderer depends on — colour
matrices per illuminant, white and black levels, CFA layout, components per pixel,
orientation, active area and crop area.

All of these round-trip **exactly** on a real Nikon Z 6 file, so equality is
demanded rather than approximated. The one exception is white balance: DNG stores
`AsShotNeutral` as rationals, so a small quantisation is unavoidable and was
measured at about 1.4e-5 relative. `WB_RELATIVE_TOLERANCE` is set to 1e-4 —
comfortably above the observed drift, and still thousands of times tighter than any
visually meaningful white balance difference. Unused channels are `NaN`, and a
channel appearing or disappearing counts as a mismatch rather than being ignored.

This costs nothing measurable, because it reuses the decodes the pixel comparison
already performs: 4.36 s to 4.18 s over eight files, within noise.

Detection is proven by mutation rather than assumed: tests perturb a colour matrix
coefficient and a white level on a genuinely converted file and assert both are
caught.

### Shot data

Timestamps, exposure, ISO, focal length, camera serial, artist, copyright and GPS
are compared through rawler's `RawMetadata`. Two fields are deliberately excluded:

- `modify_date`, which the conversion legitimately sets to now.
- Lens make, model and specification, because **rawler reads these from a NEF but
  not from a DNG**, so comparing through its API would report a loss that has not
  happened.

That second point was nearly a false conclusion. Comparing through rawler alone
suggested the lens was being dropped; inspecting the actual bytes showed the DNG
does carry `LensMake`, `LensModel`, `LensSerialNumber` and `LensSpecification`.
The gap is in rawler's DNG reader, not in the file.

Lens tags are therefore read straight from both files by `exif_tags`, a small
TIFF/EXIF reader limited to ASCII and RATIONAL values in IFD0 and the EXIF
sub-IFD. Two normalisations are required, both measured rather than assumed:

- DNG rewrites lens names in different case (`NIKON` to `Nikon`), so text is
  compared case-insensitively and trimmed.
- DNG re-encodes rationals (`240/10` to `24/1`), so numbers are compared by value.

A tag absent from the source is not required in the output; a tag present in the
source must survive. A file that is not TIFF, or is truncated, yields no tags
rather than an error, because this check must never fail a conversion on its own.

Also free in practice: 4.18 s to 4.20 s over eight files, since the source is
already in the page cache from decoding.

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

## Installation

`./install.sh` builds the release binary and installs it entirely under
`~/.local`, needing no root: the binary to `~/.local/bin`, a scalable icon to
`~/.local/share/icons/hicolor/scalable/apps`, and the desktop entry to
`~/.local/share/applications`. `./install.sh --uninstall` reverses it.

The desktop entry is named `dk.lundmoller.NefToDng.desktop` to match the
GApplication ID exactly, which is how Wayland associates a window with its icon
in the dash and the alt-tab switcher. `Exec` is written as an absolute path at
install time, because `~/.local/bin` is not reliably on `PATH` for applications
launched by the shell.

The entry declares `MimeType` for NEF and NRW so the converter appears under
*Open With*, and `%F` so it accepts files. That obliges the application to
actually handle them: it runs with `HANDLES_OPEN` and routes opened files into
the existing window's queue rather than opening a second window.

**Registering the MIME types makes the converter a candidate default handler.**
Installation therefore does not leave it as the default for NEF; a raw converter
stealing double-click from a raw *editor* would be a regression, so Darktable
stays the default and the converter is reachable through *Open With*.
