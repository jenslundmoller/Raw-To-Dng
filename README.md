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

## Installing

Two ways: download a ready-made build, or compile it yourself. Neither needs root.

### Download (easiest)

Grab the latest archive from
[Releases](https://github.com/jenslundmoller/Raw-To-Dng/releases), then:

```bash
tar xzf rawtodng-*-x86_64-linux.tar.gz
cd rawtodng-*-x86_64-linux
./install.sh
```

No Rust, no compiling. You need a 64-bit x86 Linux with GTK 4.14 or newer and
libadwaita 1.5 or newer — Ubuntu 24.04, Fedora 40, or anything more recent. Those
libraries ship with any current GNOME desktop, so there is usually nothing to
install.

To check the download, compare it against the published `.sha256` file:

```bash
sha256sum -c rawtodng-*-x86_64-linux.tar.gz.sha256
```

If it will not start, see [If it does not work](#if-it-does-not-work) below.

## Building from source

Takes about a minute.

### 1. Install Rust

**Use [rustup](https://rustup.rs), not your distribution's package.** This needs
Rust 1.88 or newer, and distro packages are often far older — Ubuntu 24.04 ships
1.75, which will fail to build with a confusing error.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then either restart your terminal or run `source "$HOME/.cargo/env"`. Check it
worked:

```bash
cargo --version
```

Already have Rust? Update it with `rustup update`.

### 2. Install the build dependencies

Needs GTK 4.14 or newer and libadwaita 1.5 or newer, which means Ubuntu 24.04,
Fedora 40, or anything more recent.

**Debian, Ubuntu, Zorin, Mint:**
```bash
sudo apt install git build-essential libgtk-4-dev libadwaita-1-dev
```

**Fedora:**
```bash
sudo dnf install git gcc gtk4-devel libadwaita-devel
```

**Arch, Manjaro:**
```bash
sudo pacman -S git base-devel gtk4 libadwaita
```

**openSUSE:**
```bash
sudo zypper install git gcc gtk4-devel libadwaita-devel
```

### 3. Download and install

```bash
git clone https://github.com/jenslundmoller/Raw-To-Dng.git
cd Raw-To-Dng
./install.sh
```

`install.sh` compiles the app and installs it — you do not need to run `cargo
build` separately.

Everything goes under `~/.local`: the binary to `~/.local/bin`, plus an icon and
desktop entry so the app appears in your application grid and under *Open With*
for raw files. Nothing is written outside your home directory.

The installer also records your existing default applications for every raw format
it registers and restores anything it would have displaced, so it will not hijack
double-click from your raw editor.

### 4. Run it

Look for **RAW to DNG** in your applications, or run `rawtodng` in a terminal.

### Uninstalling

From whichever folder you installed from — a release archive or a source checkout:

```bash
./install.sh --uninstall
```

### If it does not work

| Symptom | Cause and fix |
| --- | --- |
| `cargo: command not found` | Rust is not installed, or the terminal was not restarted after installing it. Run `source "$HOME/.cargo/env"`. |
| Build fails mentioning a `rustc` version | Your Rust is too old. Use rustup as in step 1, then `rustup update`. |
| Build fails on `gtk4` or `libadwaita` | The development packages from step 2 are missing, or your distribution is older than GTK 4.14. |
| `rawtodng: command not found` | `~/.local/bin` is not on your `PATH`. Use `~/.local/bin/rawtodng`, or add that folder to your `PATH`. |
| Not in the application grid | Log out and back in, or run `update-desktop-database ~/.local/share/applications`. |

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
RAWTODNG_TEST_DIR=/path/to/folder/of/raws \
RAWTODNG_TEST_NEF=/path/to/one.nef \
RAWTODNG_TEST_PORTRAIT_NEF=/path/to/a/portrait.nef \
cargo test
```

The portrait fixture matters: orientation handling differs between landscape and
rotated shots, and a regression there once rejected every portrait photograph.

## Design notes

`docs/session-report-2026-08-10.html` is a written report of how the app was
built: the licensing research, the measurements, every defect found and the
reasoning behind each decision. Open it in a browser — it is self-contained.

`docs/plans/2026-08-10-raw-to-dng-design.md` records the architecture and the
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
