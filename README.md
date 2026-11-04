# Reversal

Minimal converter of film negative scans into DNG.

Reversal takes a camera scan of a negative and writes a positive, losslessly compressed DNG. The result opens in a raw editor and edits like a normal photo: sliders work the right way round, and no plugin or profile needs to be installed to see it correctly.

## How it works

1. **Reads the raw scan** through LibRaw, as linear camera data with no colour rendering applied.
2. **Inverts in density.** It reads the film base from the unexposed edge of the film and finds how grey runs through the dye layers in each frame, so neutrals stay neutral from shadows to highlights. Exposure comes from the frame's highlights and average density, ignoring the holder around the film.
3. **Writes a scene-linear positive** as a 16-bit LinearRaw DNG in linear Rec.2020, with spare highlight range that a raw editor can recover.

Your source files are never changed. Each DNG stores its conversion settings in its metadata, and the web UI also saves them in a settings file next to the DNG, so a frame reopens as you left it.

## Install

Reversal is built from source. These steps are for macOS with Homebrew.

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

Open the web UI on a folder of scans. It runs locally at http://127.0.0.1:8787 and saves DNGs to a `positives` folder inside that folder.

```bash
./target/release/reversal ui path/to/scans
```

Or convert from the command line:

```bash
./target/release/reversal convert scan.NEF --flip --rotate 3 --crop-frame
```

| Option | What it does |
| --- | --- |
| `--exposure`, `--contrast` | Tone, −100 to +100 |
| `--temperature`, `--tint`, `--saturation` | Colour, −100 to +100 |
| `--flip` | Mirror left-right, for scans made emulsion side up |
| `--rotate`, `--straighten` | Quarter turns (0–3), and fine rotation in degrees |
| `--crop-frame` | Crop to the detected image frame; the default keeps the whole scan |
| `--compression` | `ljpeg` (default) or `none` |
| `-o` | Output file or folder; the default is `<name>-positive.dng` next to the input |

## Status

Early and in active development.

- Only tested with Nikon D5100 NEF and DNG scans of colour negative film. Other cameras that LibRaw reads should open, but the colour correction was fitted to this camera and one light source.
- Colour negative only. Black and white and slide film aren't handled yet.
- Frame detection for cropping is unreliable on film strips and full-frame scans, so draw the crop there. Film-stock presets and roll-wide settings aren't there yet.
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
