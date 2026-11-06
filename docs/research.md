# Reversal: Supporting Research

This doc holds the evidence behind the [Product Spec](spec.md): the options we weighed, what we measured, and where the facts came from. What we chose lives in the spec. All measurements are from Nov 2026.

## Options considered

### How a negative usually gets converted

The usual approaches either keep the file a negative or write a large rendered file. Reversal was built to fill the gap between them.

| Approach | How it converts | What you get |
| --- | --- | --- |
| Profile and curves on the raw | Camera profile, tone curves and settings over the negative | Still a negative; correct only where the profile and settings travel |
| Rendered export | Inverts, then renders to TIFF or JPEG | A positive, but large (TIFF) or lossy (JPEG), with no raw latitude |
| Inversion inside one raw editor | A module in that editor's own pipeline | A positive only inside that editor |

The first approach causes the most trouble in everyday editing. Sliders act on the negative, later curves shift the color balance, previews break where the profile doesn't travel, and leaving the editor means a TIFF or JPEG.

### Input decoder

| Option | Pros | Cons | Outcome |
| --- | --- | --- | --- |
| [LibRaw](https://www.libraw.org/), [LGPL 2.1 or CDDL 1.0](https://third-party-mirror.googlesource.com/libraw/+/refs/heads/master/LICENSE) | Reads 1,000+ cameras and DNG; gives levels and matrices | New cameras need LibRaw updates | Chosen, under the LGPL: the CDDL isn't compatible with Reversal's GPL license |
| [Adobe DNG SDK](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html), 1.7.1 build 2611 | Every DNG feature, including JPEG XL | Reads DNG only; terms [not stated on the download page](https://community.adobe.com/questions-563/adobe-dng-sdk-terms-of-use-licensing-182173) | Rejected |
| [Adobe DNG Converter](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html) | Adobe's camera coverage, with a command line | Separate install, macOS and Windows only | Fallback for users with unsupported cameras |
| Convert in a raw editor first | No work for us | Extra manual step | Rejected |
| rawler from [DNGLab](https://github.com/dnglab/dnglab) | Pure Rust, LGPL-2.1 | Self-described alpha, unstable API, may panic on bad input | Rejected |

We also decided against our own raw-to-DNG step, since it adds nothing to the inversion.

### Output encoding

| Question | Options | Outcome and why |
| --- | --- | --- |
| Sample format | 16-bit integer, float16, float32 | 16-bit integer: float16 with Deflate works in Adobe apps but fails in Apple's engine |
| Color space | Camera native, linear Rec.2020, linear ProPhoto | Linear Rec.2020: ProPhoto rendered identically in Lightroom Classic, Lightroom mobile and Apple Photos, so we kept the real-color standard |
| Reference | Scene-linear, print-referred | Scene-linear: raw tools behave as on a digital photo, with the most headroom. Print-referred gets double contrast from the editor's base curve |
| Where the look lives | Baked in, profile tone curve, left to the user | Profile tone curve: Apple Photos applies only the main profile, so it must be the chosen look |
| Compression | Lossless JPEG, [JPEG XL](https://forum.dxo.com/t/compressed-dng-brought-to-pl9-but-with-a-bug/54494/38) | Lossless JPEG, read everywhere; JPEG XL is well supported mainly for LinearRaw and isn't built |
| Writer | Adobe DNG SDK, rawler, our own | Our own minimal writer: it needs only LinearRaw and a few tags |
| Original in the file | Embedded, left out | Left out: [embedding it is why Denoise DNGs reach about 4× a raw](https://community.adobe.com/t5/lightroom-classic-discussions/why-are-dng-files-produced-bt-denoise-ai-6-times-larger-than-the-orignal/m-p/13748097), against [2–3× for linear DNGs generally](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/) |

ProRAW is [a linear DNG, demosaiced before Lightroom sees it](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/), which is the precedent for this file type.

### Inversion model

Naive 1 − x inversion is a known pitfall, and density-based inversion is the industry basis, as in [Cineon printing density](https://en.wikipedia.org/wiki/Cineon). We compared curves or LUTs, per-channel power and a full density model, and chose density. How it is fitted changed once: per-channel black and white points gave way to per-frame gray curves, based on the experiment below. An early plan to mimic enlarger heads and lab scanners with controls was dropped after the first test, because it added nothing over familiar digital controls.

### Default measured area

All options were scored against reference scans; numbers are under Experiments.

| Option | Outcome |
| --- | --- |
| Frame detection | Removed |
| Whole scan | Failed on a black holder edge, which landed on the white point |
| Holder detection | Replaced |
| Centered cut, size adjustable, drawing as the fallback | Chosen at 85% |

### Language

| Option | Pros | Cons | Outcome |
| --- | --- | --- | --- |
| Rust | Memory-safe parsing of untrusted files; easy distribution | LibRaw is C++, so it needs FFI | Chosen |
| C++ | LibRaw and the DNG SDK link directly | Parser memory-safety burden | Rejected |

The pipeline is per-pixel and runs on all cores in plain Rust. GPU work remains an untried option.

### App shell

A Swift window around the existing web UI won, because a native UI gains nothing perceptible and adds a second UI to keep in step. The connection between page and engine adds only 0.3 to 0.7 ms per preview.

| Option | Added size | Crates | Outcome |
| --- | --- | --- | --- |
| Swift WKWebView window | Disk image about 1 MB at the time, 3.2 MB with the icon | None | Chosen |
| wry + tao | 1.2 MB | 79 | Rejected |
| Tauri | 7.0 MB | 411 | Rejected |
| egui | 4.4 to 8.8 MB | 128+ | Rejected; color management unconfirmed |

Leaving OpenMP, lcms and libjpeg out of LibRaw kept DNGs byte-identical on three test files. The cost is that DNGs with lossy JPEG compression can't be read.

### Where the setup controls sit

Film base and measured area can break a conversion, so they moved away from the development sliders, to a block pinned under the frame list. Setup/Develop tabs were rejected because they hide a suspect base while you judge color. An on-image toolbar was rejected because it covers the picture. A film base color chip was tried and dropped, since camera RGB looks green and would mislead. Mirror was first placed in Setup, then moved to Frame after review, because it only orients the image.

In the desktop UI pass on November 5, Setup moved from under the frame list to the right column, pinned above Develop, which gave the frame list its full height. Setup/Develop tabs were tried again and rejected for the same reason as before. A base color chip was tried again in the roll review, to compare readings with each other, and removed for the same reason: camera RGB looks green.

## Experiments and results

The experiment code and reference scans aren't published, so these numbers can't be reproduced from the repo. Targets are finished conversions from another tool, so they show one look rather than the truth. Lower scores are better.

### DNG spike

Hand-made positive LinearRaw DNGs, from D5100 samples, settled the output format.

- 16-bit integer with 2.3 EV of storage headroom and BaselineExposure +2 works in Lightroom Classic, Lightroom mobile and Apple Photos. Highlights and Exposure recover the headroom.
- Float16 with Deflate works in Adobe apps but fails in Photos and Finder.
- Lightroom Classic and mobile list all three embedded profiles; in Classic, only the Profile browser shows them. Photos shows no choice but applies the main profile's tone curve.

Capture One and darktable weren't checked. Lightroom Classic showed no initial white balance until As Shot was reselected, so the writer now sets `crs:WhiteBalance="As Shot"` in the XMP.

### Gray curves

Per-frame gray curves cut color error by a quarter on the original frames and by about half on new, harder ones. Measured on 22 D5100 frames from 6 rolls. Cells are chroma error ΔE, then RMS L.

| Frames | Engine before, as shipped | Engine before, hand-set area | Built model |
| --- | --- | --- | --- |
| Original 14 | 10.3 / 19.8 | 10.3 / 19.8 | 7.7 / 8.8 |
| Forest roll (4) | 18.1 / 57.7 | 11.2 / 17.7 | 8.0 / 6.7 |
| Metro roll (4) | 16.0 / 39.0 | 13.9 / 22.7 | 8.0 / 10.7 |

The built-model column used tighter alignment and a slightly different mask; the matching prototype scored 8.0 / 9.2, 8.8 / 7.8 and 7.7 / 11.4. The search method and exposure mix were chosen after seeing the new frames, so their scores are optimistic. The curve, correction, exposure constant and toe were frozen on the original frames.

| Test | Result | Decision |
| --- | --- | --- |
| RGB/CMY sliders | Best per-frame RGB offsets still left warm hues 21° off and 400D at 26 | Leave out |
| Per-channel S-curves | One bend 8.26, two bends 8.81, free form 8.90 (color error, original 14) | Leave out |
| Density crosstalk correction | Without it blues drift 18–21° and warm tones 5–10°; with it 7–12° and 2–7°. Greens drift about 7–10° further | Keep, tune greens later |
| Light pull toward an averaged curve | 8.10 vs 8.26 alone; a fixed 50% blend hurts, at 9.18 | Keep the light pull |
| Narrowing search band with equal bins | Forest roll 14.9 → 9.4; `_DSC3335` 20.9 → 9.8 | Keep |
| Highlight anchor for the curve | About 16–18, because blue skies aren't neutral | Reject |
| Offset from the film base | Worse on every group, metro roll 15.2 | Keep the base anchor |
| Roll pooling of curves | Forest roll 12.5, but original frames 8.8 and metro roll 10.4 | Curves stay per frame |
| Exposure from highlights and scene average | Brightness 9.9 / 9.6 / 15.9; highlights alone 11.3 / 14.7 / 17.8; average alone 9.0 / 9.0 / 23.1; best per frame about 8 | Keep the mix |
| Center-weighted average | Within about 1 of the plain average | Drop |
| Shared toe with γ 0.7 | Brightness improved on all groups; settings chosen on the original frames | Keep |

Published minilab patents describe the same family of methods: bin pixels by green density, keep the near-neutral ones, fit red and blue curves per film type, keep a shared toe apart from per-channel balance, and correct in density with a matrix. None of this shows what any shipping scanner does.

### Other color tests

Measured against 8 hand-finished reference scans of one film stock.

- **Saturation:** a built-in linear saturation after inversion beat the same correction in density (chroma error ΔE 7.77 vs 8.66). A calibrated 3×3 matrix is untested.
- **Demosaic:** balancing to the film base first lowered fine color noise by 6.9% and fine luminance detail by 3.2%. Crops looked practically identical, so it wasn't adopted.
- **Input white balance:** needs no setting, because dividing by the film base cancels any per-channel gain.

### Measured area

Scored on 22 frames with reference scans. 85% was chosen because its gap to 80% is small and it had the lowest neutral cast (3.77).

| Area | Color error | Brightness error |
| --- | --- | --- |
| Holder detection | 7.79 | 8.79 |
| 95% | 7.83 | 11.09 |
| 90% | 7.71 | 8.87 |
| 85% | 7.63 | 8.69 |
| 80% | 7.53 | 8.48 |
| 70% | 7.47 | 8.59 |

At 95% the area takes in holders.

### Tone controls and profiles

Display clipping is recoverable up to the stored limit. Real loss was 0–0.7% of the picture across the test frames. On one test frame cropped to the image area, Whites −100 took display clipping from 3.0% to 0%. A curve-less profile looked blown out, which led to Flat and Vivid. Vivid's saturation was eased from 1.25 to 1.15 after review.

### Preview against editors

Camera Raw is the reference for the preview. On `_DSC5384` in Safari on a MacBook, all three looks are virtually identical to Camera Raw, with a negligible shadow difference. The comparison hasn't been repeated on another OS or machine.

- **Dropped:** a fitted highlight roll-off with hue-stable tone matched Lightroom screenshots of `_DSC5379` closely (mean error 2.9 against 11.5 levels out of 255 under Neutral), but was too much custom rendering.
- **Camera Raw:** a shoulder in the profile's own curve with a −2 EV offset looks the same as the standard DNG, so Camera Raw follows the curve and adds no compression of its own.
- **Apple Photos:** a Flat DNG with a −2 EV offset on its main profile opened too bright (average 188 out of 255, against 227 if the offset were ignored and 147 if honored). Lowering BaselineExposure itself, with no offset on the main profile and +2 EV on the others, fixed it and looks the same in Camera Raw and Finder previews. Photos keeps its own rendering, with a slight shift left.
- **Whites at 0:** a jump at the slider's zero came from a shoulder that dropped the raw editor's roll-off. Starting the shoulder from that roll-off removed it. The stored DNG changed by only 9 bytes, which look like rounding in the Flat profile's curve.

### File size

A 16 MP D5100 scan came to about 62–64 MB with lossless JPEG, two frames measured. Other sizes and JPEG XL weren't measured.

### Speed

Using all cores took a single preview from 65 ms to 17 ms and a single DNG from 1.1 s to 0.8 s, byte-identical to the earlier build on one NEF. Twelve frames save in 4.4 s on the development build and 6.1 s on the app's. Parallel decodes with an OpenMP build of LibRaw took 162 s for those twelve, so decoding runs one frame at a time.

### Film base detection

Reading the largest even patch keeps the base on clear film, and moved color ratios by about 1% at most on 25 samples. The old method took the median of the brightest 3% of pixels. It drew a whole strip through holes and markings, and landed on picture content on frames with no visible edge.

| Measure, new against old (25 frames) | Result |
| --- | --- |
| Green base level | +0.1% to −6.2%, median about −1.6% |
| Red to green ratio | −0.3% to +1.2% |
| Blue to green ratio | −1.3% to +0.9% |
| Frames reported as no clear film | 5: `_DSC4444`, `_DSC5037` to `_DSC5040` |

The patch must lie within 7% of a high percentile of the even cells' levels. A 10% range was tried and rejected, because it lowered the base by up to 7.6% on frames that already read well.

Of the five unsure frames, four barely show a film edge. `_DSC4444` has a visible edge, so that one is a miss. `_DSC4062` counts as found from a thin sliver at the scan's edge, which may not be clear film.

The 6% limit for flagging a reading in the roll review comes from nine similar frames, `_DSC5376` and `_DSC5384` to `_DSC5391`, whose green base stayed between 0.381 and 0.392. That they share a roll is assumed from their file numbers, and the limit hasn't been tried in the review on a single real roll.

We reviewed the results by eye on November 5 and adopted the new method: its hits are good, and saying detection failed in unclear cases is preferred to marking an unwanted area. It hasn't been scored against the reference scans. Only base values were compared, not finished images, so the color error figures in this doc still describe the old detection.

### Sharing a film base across a roll

A shared base helps some rolls and hurts others, so it is opt-in and shares only the base. Color error against finished scans:

| Roll | Per frame | Shared base | Shared base and curves |
| --- | --- | --- | --- |
| 8 frames | 6.41 | 6.38 | 7.89 |
| 4 frames | 6.43 | 6.23 | 6.92 |
| 2 scored frames | 14.48 | 12.62 | 12.71 |

On the last roll the neutral cast rose from 2.59 to 7.5 with a shared base. Cropping six frames of one roll to their center moved their own base by up to 0.31 density; on a 4-frame roll with a different look it stayed within 0.02.

## Sources

Forum threads are user reports, not vendor documentation.

### Patents

- [US6160634: neutral estimation and gradation balance](https://patents.google.com/patent/US6160634)
- [US6081343: per-film balance tables](https://patents.google.com/patent/US6081343)
- [EP0612183B1: density-dependent gray balance](https://patents.google.com/patent/EP0612183B1/de)
- [US6233069: printing density, toe and scene balance](https://patents.google.com/patent/US6233069)
- [US6204940B1: inversion and shared inverse-S](https://patents.google.com/patent/US6204940B1)

### Color science

- [Wikipedia: Cineon](https://en.wikipedia.org/wiki/Cineon)

### DNG and raw decoding

- [Adobe DNG downloads: spec 1.7.1, DNG SDK 1.7.1 build 2611, command-line spec, patent license](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html)
- [Adobe Community: DNG SDK terms of use](https://community.adobe.com/questions-563/adobe-dng-sdk-terms-of-use-licensing-182173)
- [Adobe Community: why Denoise DNGs are so large](https://community.adobe.com/t5/lightroom-classic-discussions/why-are-dng-files-produced-bt-denoise-ai-6-times-larger-than-the-orignal/m-p/13748097)
- [Life After Photoshop: Lightroom and iPhone ProRAW linear DNGs](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/)
- [DxO forum: DNG 1.7 JPEG XL support across editors](https://forum.dxo.com/t/compressed-dng-brought-to-pl9-but-with-a-bug/54494/38)
- [LibRaw](https://www.libraw.org/) and its [license (LGPL 2.1 / CDDL 1.0)](https://third-party-mirror.googlesource.com/libraw/+/refs/heads/master/LICENSE)
- [DNGLab / rawler on GitHub](https://github.com/dnglab/dnglab)
