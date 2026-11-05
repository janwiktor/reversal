# Reversal

Minimal converter of film negative scans into DNG.

Reversal takes a camera scan of a negative and writes a positive, losslessly compressed DNG. The result opens in a raw editor and edits like a normal photo: sliders work the right way round, and no plugin or profile needs to be installed to see it correctly.

## How it works

1. **Reads the raw scan** through LibRaw, as linear camera data with no color rendering applied.
2. **Inverts in density.** It reads the film base from the unexposed edge of the film and finds how gray runs through the dye layers in each frame, so neutrals stay neutral from shadows to highlights. Color and exposure are measured on the center of the scan (85% of each side by default), which stays clear of the holder and rebate; you can change the size or draw the area.
3. **Writes a scene-linear positive** as a 16-bit LinearRaw DNG in linear Rec.2020, with spare highlight range that a raw editor can recover.

Your source files are never changed. Each DNG stores its conversion settings in its metadata, and the web UI also saves them in a settings file next to the DNG, so a frame reopens as you left it.

## Install

The Mac app needs Apple Silicon and macOS 13 or later. It isn't notarized yet, so the disk image includes the steps to open it the first time.

To build from source on macOS with Homebrew:

```bash
brew install rustup libraw pkgconf
```

```bash
rustup default stable
```

```bash
cargo build --release
```

If `cargo` isn't found, add `/opt/homebrew/opt/rustup/bin` to your `PATH`.

## Use

In the app, choose a folder of scans. From the command line, open the web UI on a folder; it runs locally at http://127.0.0.1:8787. Both save DNGs to a `positives` folder inside that folder.

```bash
./target/release/reversal ui path/to/scans
```

Each frame remembers its settings as you change them. Tick several frames to save them together, each with its own settings; frames you haven't touched get the automatic conversion.

Frames of one roll can share a film base: choose Roll under Setup, or pass `--roll` on the command line. This helps most when some scans don't show the unexposed edge of the film, as long as most of them do. Color curves and exposure stay each frame's own.

Or convert from the command line:

```bash
./target/release/reversal convert scan.NEF --flip --rotate 3 --crop-area
```

| Option | What it does |
| --- | --- |
| `--exposure`, `--contrast` | Tone, −100 to +100 |
| `--highlights`, `--shadows`, `--whites`, `--blacks` | Tone curve, −100 to +100. Saved in the profile, so the stored image keeps everything |
| `--profile` | `neutral` (default), `flat` (low contrast, no clipping) or `vivid`. The DNG opens with this one and carries the others |
| `--temperature`, `--tint`, `--saturation` | Color, −100 to +100 |
| `--flip` | Mirror left-right, for scans made emulsion side up |
| `--rotate`, `--straighten` | Quarter turns (0–3), and fine rotation in degrees |
| `--area-percent` | Size of the centered area that color and exposure are measured on, default 85 |
| `--crop-area` | Crop to that area; the default keeps the whole scan |
| `--roll` | Measure one film base across the inputs and use it on each |
| `--compression` | `ljpeg` (default) or `none` |
| `-o` | Output file or folder; the default is `<name>-positive.dng` next to the input |

To build the Mac app, run `packaging/macos/build.sh`, which writes a disk image to `dist/`. It builds LibRaw from source and needs Rust, pkgconf and Xcode.

## Status

Early and in active development.

- Only tested with Nikon D5100 NEF and DNG scans of color negative film. Other cameras that LibRaw reads should open, but the color correction was fitted to this camera and one light source.
- Color negative only. Black and white and slide film aren't handled yet.
- The centered image area doesn't suit full film strips, so draw the area and crop there. Film-stock presets aren't there yet.
- Camera EXIF isn't copied to the DNG.

## Development

```bash
cargo test
```

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
```

`crates/reversal-core` is the conversion engine, with no UI. `crates/reversal-cli` holds the command line and the web UI. See [AGENTS.md](AGENTS.md) for project conventions.

## Notices

This product includes DNG technology under license by Adobe.

Raw decoding uses [LibRaw](https://www.libraw.org/), linked as a system library.
