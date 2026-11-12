# AGENTS.md — Reversal

Guidance for AI coding agents (and humans) working in this repository.

## Project overview

Reversal converts scanned or camera-captured **film negatives into positive images**.
One shared engine serves every front end:

1. **CLI and local web UI** — the `reversal` binary.
2. **Mac app** — a small window around the web UI, with the same binary as its engine.
3. **iOS app (prototype)** — for iPhone and iPad: the same page around the same engine, linked into the app.

Key design decisions: input is camera raw or
DNG via LibRaw; output is a **positive, scene-linear LinearRaw DNG** (16-bit, linear
Rec.2020, own camera model "Reversal Positive", embedded profiles); inversion uses an
explicit **density model** (film base, per-frame gray curves, density crosstalk, exposure).

Describe conversion methods on their own terms. Don't name or compare other converters in docs, code comments or commit messages.
Naming an app that consumes Reversal's DNGs is fine when describing testing: whether it opens a
file correctly, or how a rendering compares or benchmarks against it. Never claim that a feature
works like, is taken from, or matches, another product. Clean implementation based on research, no copying.

## Architecture

```
crates/reversal-core/   Conversion engine — decode, invert, DNG writer; no UI, no host APIs
crates/reversal-cli/    `reversal` binary: `convert` command + `ui` (local web UI server)
crates/reversal-cli/ui/ Web UI assets (plain HTML/JS, embedded in the binary)
crates/reversal-ios/    The engine as a static library with a C interface, for the iOS app
packaging/macos/        Mac app: a small Swift window around the UI, with the binary as its engine
packaging/ios/          iOS app prototype: Xcode project, Swift shell, engine build script
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
  holder around the film and light through holes: it measures a centered share of the scan, leaving
  out dense areas connected to the scan's edge, unless the user draws an area, which it measures as drawn.
- **Inputs** are camera RAW (DNG, CR2/CR3, NEF, ARW, RAF…) today. Scanner TIFFs may follow.
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
- **iOS app:** `packaging/ios/Reversal.xcodeproj`. iOS can't start another process, so the app links the engine, and its web view's requests reach it as calls, through `Engine` in `ui.rs`, with no server or port. The Swift shell adds only access to files outside the app and handing DNGs to Files, Photos and other apps. Its build phase runs `packaging/ios/engine.sh`, which builds LibRaw and `reversal-ios` for the SDK.

## Commands

```bash
cargo build --release                       # build everything
cargo test                                  # unit tests
cargo fmt && cargo clippy --all-targets -- -D warnings    # format and lint
cargo run --release -- convert Samples/_DSC5384.NEF -o out.dng   # convert one file
cargo run --release -- ui Samples           # web UI at http://127.0.0.1:8787
packaging/macos/build.sh                    # Mac app disk image in dist/
rustup target add aarch64-apple-ios aarch64-apple-ios-sim   # once, for the iOS app
open packaging/ios/Reversal.xcodeproj       # iOS app; for a device, set DEVELOPMENT_TEAM in packaging/ios/Local.xcconfig
```

## CI

- `.github/workflows/ci.yml` runs on every pull request and push to `main`. Docs-only changes
  skip its jobs, and its Linux and macOS checks are required to merge. Linux checks format, lint,
  licenses and advisories, and tests, with and without LibRaw. macOS then runs the tests, only once Linux passes, against the LibRaw the Mac app ships with
  (`packaging/macos/libraw.sh`).
- `.github/workflows/mac-app.yml` builds and checks the disk image on every push to `main`
  except docs-only changes, on pull requests that change packaging or dependencies, and on
  request. It keeps the image as a run artifact for 7 days.
- `.github/workflows/ios-app.yml` builds the iOS app for the simulator, unsigned, on every push to
  `main` except docs-only changes, on pull requests that change its packaging, `reversal-ios` or
  dependencies, and on request. It checks the version, the licenses and what the app links, but
  doesn't run the app: check it in the simulator when changing the shell or the page.
- CI decodes small raw files that the tests write themselves, not real camera files, and it
  can't check how a DNG opens in a raw editor. Check conversions locally when changing
  `decode.rs`, and check that a DNG still opens correctly when changing what's written.

- Dependabot opens a monthly pull request for the GitHub Actions and one for the crates.
- `.github/workflows/release.yml` runs on a tag like `v0.1.0`: it checks the tag, builds the
  disk image with `mac-app.yml` and creates a draft release. It can't check that the app
  opens on another Mac, so install from the draft before publishing.

## Versioning and releases

- **One version, x.y.z,** in `[workspace.package]` in `Cargo.toml`. The engine, web UI, Mac app,
  disk image name and git tag all read it. Never write the number anywhere else, except where
  `docs/conversion-method.md` and its `figures.py` name the version they describe.
- **Which tier to bump:** z for fixes that don't change a conversion; y for new features and for
  anything that changes a DNG's pixels or tags; x for 1.0 and later breaking changes. Bump in a PR
  of its own, or as the last commit of a PR that needs it, and run `cargo check` so `Cargo.lock`
  follows. Not every change needs a bump: only what ships in a release.
- **Build identifiers come from git, so never set them by hand.** Every binary carries its short
  commit hash, shown as `0.1.0 · abc1234` in the web UI and `0.1.0 (abc1234)` by `--version`. The
  Mac app's `CFBundleVersion` is the commit count, so it always rises on `main`; the About panel
  shows `Version 0.1.0 (50)`. It needs a full clone, and only builds from `main` count.
- **Keep the commit out of DNGs.** The Software tag holds x.y.z only, so builds of one version
  convert to identical bytes.
- `MODEL_VERSION` in `invert.rs` is separate: it tracks the conversion model that settings files
  were measured with. Raise it when the model changes, whatever the release number.
- **To release:** merge the version bump to `main`, then tag that commit and push the tag
  (`git tag v0.1.0 && git push origin v0.1.0`). Install the disk image from the draft on a Mac that
  hasn't run this version, then publish. Publishing is the maintainer's decision. Releases are
  full releases, not pre-releases, so the newest one shows on the repository page.
- The Mac app stays ad hoc signed through pre-alpha and alpha, with no Apple certificate; users
  approve it with Open Anyway, as `Read me first.txt` explains.
- **iOS test builds go to TestFlight by hand for now:** in Xcode, choose Any iOS Device, then
  Product > Archive, and Distribute App to App Store Connect from the Organizer. Archive from a
  clean `main`, or a release tag for external testers. The build number is the commit count, and App Store Connect accepts each number
  only once per version. Untick Manage Version and Build Number when distributing: left on, Xcode
  renumbers a build whose number is taken, and the number no longer matches the commit count.
  When the app or engine starts calling another API that Apple requires a reason for, add it to
  `packaging/ios/Reversal/PrivacyInfo.xcprivacy`.
- **Builds for external testers, and later the App Store, come from a release tag.** The app's
  LibRaw notice says that each release carries the source of LibRaw and the crates, so check out
  the tag, archive it, and add the build to an external group only once that release is published.
  Upload it before any later build of the same version from `main`, which has a higher number.
  Internal testers can have builds from `main`. App Store Connect links to `PRIVACY.md` on `main`
  as the privacy policy, as About does, and its App Privacy answer is Data Not Collected: keep
  both true, and in step with the privacy manifest.

## License

- Reversal is under `MPL-2.0`. New dependencies must be compatible with it and with distribution
  through app stores, so no GPL or LGPL dependency gets linked in. CI runs `cargo deny check`
  against the licenses allowed in `deny.toml` and the advisory database; add a license there only
  after checking both.
- Every first-party source file starts with the MPL's Exhibit A notice, after any shebang or
  doctype, and new files need it too. Don't add Exhibit B: the code stays compatible with Secondary
  Licenses.
- Every build uses LibRaw under its CDDL 1.0 option, whether it's linked statically in the Mac and
  iOS apps or as a system library. Never its LGPL 2.1 option. Each release carries the source of LibRaw and
  the crates: the CDDL (3.1) and the MPL (3.2) ask that the source be available and that recipients
  be told how to get it.
- Keep the notices in the web UI's About dialog, both `Info.plist` files, `--version`, `Read me first.txt` and
  the README in step. The Adobe DNG notice and LibRaw's copyright stay in all of them.

## Experiments and docs

- The repo keeps product code, `README.md` for users of the tool, this file for how to work
  on it, and docs or skills that are useful to anyone.
- `docs/spec.md` says what the product does and why. `docs/research-notes.md` is the working
  record behind the conversion and technical choices: findings, options weighed, what each test
  changed and decided, and sources. UI choices and their reasons belong in the spec. Keep each
  fact in one of them, and update them when a change makes them untrue.
- `docs/conversion-method.md` presents the current conversion end to end: each step, its formulas
  and constants, and a summary of the evidence. It summarizes the notes, which stay the source
  for test detail. It names the engine and model version it describes: when a change to the
  conversion or the DNG makes it untrue, update it and that version, and run
  `docs/images/conversion-method/figures.py` again when the constants it copies change.
- The wiki is generated from `docs/` by `.github/workflows/wiki.yml`, so never edit it.
- Experiments, test scripts and their output, agent config and memory, draft docs, ad hoc
  reports and plans stay out of this repo. Results that settle a conversion or technical choice go
  in `docs/research-notes.md`, summarized as what was tested, the effect and the decision.
- Write tracked files, commits and PRs as if the repo were public. Don't link to private
  docs or internal decision numbers.

## Git workflow

- Default branch: `main`. Work on feature branches; keep commits focused.
- Imperative, concise commit messages (`Add film-base sampling to core`).
- Don't commit build output, secrets, or large sample images.
- Commit with your GitHub noreply email address.
