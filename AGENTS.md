# AGENTS.md — Reversal

Guidance for AI coding agents (and humans) working in this repository.

## Project overview

Reversal converts scanned or camera-captured **film negatives into positive images**.
One shared engine serves every front end:

1. **CLI and local web UI** — the `reversal` binary.
2. **Mac app** — a small window around the web UI, with the same binary as its engine.

Key design decisions: input is camera raw or
DNG via LibRaw; output is a **positive, scene-linear LinearRaw DNG** (16-bit, linear
Rec.2020, own camera model "Reversal Positive", embedded profiles); inversion uses an
explicit **density model** (film base, per-frame gray curves, density crosstalk, exposure).

Describe conversion methods on their own terms. Don't name or compare other
converters or editors when discussing them, in docs, code comments or commit messages.
Naming an app is fine only for compatibility testing (does it open our DNG correctly).

## Architecture

```
crates/reversal-core/   Conversion engine — decode, invert, DNG writer; no UI, no host APIs
crates/reversal-cli/    `reversal` binary: `convert` command + `ui` (local web UI server)
crates/reversal-cli/ui/ Web UI assets (plain HTML/JS, embedded in the binary)
packaging/macos/        Mac app: a small Swift window around the UI, with the binary as its engine
Samples/              Local sample scans (git-ignored)
```

Rules:
- **All conversion logic lives in `reversal-core`.** The CLI, UI and Mac app are
  thin adapters; never duplicate processing code between them.
- `reversal-core` must not depend on UI frameworks, web servers or host SDKs, so
  it can be tested headlessly.
- Keep the core API deterministic: same input + same parameters → bit-identical output.

## Domain notes (read before touching image code)

- **Negative types differ.** Color negative (C-41), black & white, and slide/E-6
  (already positive) need different pipelines. Never assume color negative.
- **Orange mask.** Color negative film has an orange base. Remove it by sampling
  the film base (unexposed rebate/border) and neutralizing per channel —
  do not just invert and white-balance afterwards.
- **Work in linear light.** Do inversion and per-channel scaling on linear data
  (decoded RAW or linearized TIFF), not gamma-encoded 8-bit values. Apply the
  output transfer curve / color space conversion last.
- **Precision.** Process internally in 32-bit float (or at least 16-bit). Only
  quantize on export. Avoid clipping highlights/shadows mid-pipeline.
- **Gray curves.** The dye layers have different contrast, and it varies with density:
  map red and blue onto green with a per-frame gray curve (not a black/white point per
  channel), so neutrals stay neutral at every density. Scene analysis must exclude the
  holder around the film and light through holes: it measures a centered share of the scan
  unless the user draws an area.
- **Inputs** include camera RAW (DNG, CR2/CR3, NEF, ARW, RAF…) and scanner TIFFs.
  Preserve and write metadata (EXIF, ICC profile) where possible.
- **Non-destructive.** Never overwrite source files. Output to new files.

## Coding conventions

- Prefer small, pure functions in `reversal-core` that operate on image buffers + a
  parameters struct; keep I/O at the edges.
- Name parameters with units/meaning (`base_rgb_linear`, `gamma_out`), not `a`, `b`.
- Document any color-science formula with a short comment.
- Match the style of surrounding code; don't reformat unrelated files.
- Use American spelling everywhere: labels, docs, comments and names (`color`, `center`, `gray`).

## Testing

- Every `reversal-core` change needs tests. Use small synthetic images for unit tests.
  Golden reference images (`tests/fixtures/`) are planned but don't exist yet.
- Compare images with a tolerance (e.g. max ΔE or per-channel epsilon), not exact
  byte equality, unless testing determinism.
- Large sample scans do not go in git (see `.gitignore`); use Git LFS if fixtures grow.

## Stack

- **Rust** (stable, Cargo workspace) for the engine, CLI and UI server.
- **LibRaw** (system library via FFI; `brew install libraw pkgconf` on macOS) for raw decoding.
- Own minimal **DNG writer** in `reversal-core` (no Adobe DNG SDK in the product).
- **Web UI:** plain HTML/CSS/JS served by the `reversal ui` command; no frontend build step.
- **Mac app:** `packaging/macos/build.sh`. The window adds only what a page can't do (start the engine, choose a folder), so features belong in the engine and the page, not in Swift.

## Commands

```bash
cargo build --release                       # build everything
cargo test                                  # unit tests
cargo fmt && cargo clippy --all-targets -- -D warnings    # format and lint
cargo run --release -- convert Samples/_DSC5384.NEF -o out.dng   # convert one file
cargo run --release -- ui Samples           # web UI at http://127.0.0.1:8787
packaging/macos/build.sh                    # Mac app disk image in dist/
```

## CI

- `.github/workflows/ci.yml` runs on every pull request and push to `main`, except docs-only
  changes. Linux checks format, lint and tests, with and without LibRaw. macOS then runs the
  tests, only once Linux passes, against the LibRaw the Mac app ships with
  (`packaging/macos/libraw.sh`).
- `.github/workflows/mac-app.yml` builds and checks the disk image on every push to `main`
  except docs-only changes, on pull requests that change packaging or dependencies, and on
  request. It keeps the image as a run artifact for 7 days.
- CI decodes small raw files that the tests write themselves, not real camera files, and it
  can't check how a DNG opens in a raw editor. Check conversions locally when changing
  `decode.rs`, and check that a DNG still opens correctly when changing what's written.

## Experiments and docs

- The repo keeps product code, `README.md` for users of the tool, this file for how to work
  on it, and docs or skills that are useful to anyone.
- Experiments, test scripts and their output, agent config and memory, draft docs, ad hoc
  reports, research notes and plans stay out of this repo.
- Write tracked files, commits and PRs as if the repo were public. Don't link to private
  docs or internal decision numbers.

## Git workflow

- Default branch: `main`. Work on feature branches; keep commits focused.
- Imperative, concise commit messages (`Add film-base sampling to core`).
- Don't commit build output, secrets, or large sample images.
- Commit with your GitHub noreply email address.
