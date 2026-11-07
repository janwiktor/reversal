# Reversal: Supporting Research

This doc holds the evidence behind the conversion and file format in the [Product Spec](spec.md): what we found about inverting negatives, the technical options we weighed, and what tests showed. What we chose lives in the spec. All measurements are from Nov 2026.

## How to read the tests

Each test says what was tried, what changed and what it decided. Most compare a conversion with a finished scan of the same frame, made with another tool. That target shows one look rather than the truth, so a score is a guide, not a verdict. The experiment code and reference scans aren't published.

All scores are differences from the finished scan in CIELAB, and lower is better:

| Score | What it measures |
| --- | --- |
| Color error | Average difference in color (a\*b\*, or chroma ΔE), ignoring brightness |
| Brightness error | RMS difference in L\*, on its 0–100 scale |
| Neutral cast | The tint left on grays: the average a\*b\* difference over near-neutral pixels |
| Hue drift | How many degrees a color's hue sits from the finished scan |

The main test set is 22 frames from 6 rolls, all scanned with a Nikon D5100: 14 original frames, and two harder rolls of 4 frames each, called forest and metro below. The two rolls came in after the model was first fitted. Scores predate the current film base detection (see Film base) unless a test says otherwise.

## Color and negative inversion

### How negatives are usually converted

The usual approaches either keep the file a negative or write a large rendered file. Reversal was built to fill the gap between them.

| Approach | How it converts | What you get |
| --- | --- | --- |
| Profile and curves on the raw | Camera profile, tone curves and settings over the negative | Still a negative; correct only where the profile and settings travel |
| Rendered export | Inverts, then renders to TIFF or JPEG | A positive, but large (TIFF) or lossy (JPEG), with no raw latitude |
| Inversion inside one raw editor | A module in that editor's own pipeline | A positive only inside that editor |

The first approach causes the most trouble in everyday editing. Sliders act on the negative, later curves shift the color balance, previews break where the profile doesn't travel, and leaving the editor means a TIFF or JPEG.

### Inversion model

Naive 1 − x inversion is a known pitfall, and density-based inversion is the industry basis, as in [Cineon printing density](https://en.wikipedia.org/wiki/Cineon). We compared curves or LUTs, per-channel power and a full density model, and chose density. An early plan to mimic enlarger heads and lab scanners with controls was dropped after the first test, because it added nothing over familiar digital controls.

Input white balance needs no setting, because dividing by the film base cancels any per-channel gain.

### Film base

Tested on 25 frames, comparing base values rather than finished images. The last test scores the method against the reference scans.

| Tested | Effect | Decision |
| --- | --- | --- |
| Largest even patch, against the median of the brightest 3% of pixels | The old method drew a strip through holes and markings, and landed on picture content when no edge showed. The green base moved +0.1% to −6.2% (median about −1.6%); red to green −0.3% to +1.2%, blue to green −1.3% to +0.9% | Adopted after review by eye |
| Reporting frames with no clear film | 5 of 25 reported: 4 barely show a film edge, 1 is a real miss. 1 further frame counted as found from a thin sliver that may not be clear film | Warn instead of marking an unwanted area |
| How close a patch must be to the brightest even areas | A 10% range lowered the base by up to 7.6% on frames that already read well | Use 7% |
| Current detection on the main test set, against the old one, with the 85% measured area | Color error 7.55 against 7.63, brightness error 8.67 against 8.69, neutral cast 3.82 against 3.77. The old scores came from earlier scripts, so this is close rather than exact | No score penalty; keep |

### Sharing a base across a roll

| Tested | Effect | Decision |
| --- | --- | --- |
| Shared base, against each frame's own, on rolls of 8, 4 and 2 scored frames | Color error from 6.41 to 6.38, 6.43 to 6.23, and 14.48 to 12.62. On the last, neutral cast rose from 2.59 to 7.5 | Opt-in, never automatic |
| Sharing gray curves as well | Color error rose to 7.89 and 6.92 on the first two rolls, and 12.71 on the last | Share the base only |
| Spread of readings on nine similar frames | Green base stayed between 0.381 and 0.392 | Flag readings over 6% from the roll's. That the frames share a roll is assumed from their file numbers, and the limit hasn't been tried on a real roll |

### Gray curves

Published minilab patents describe the same family of methods: bin pixels by green density, keep the near-neutral ones, fit red and blue curves per film type, keep a shared toe apart from per-channel balance, and correct in density with a matrix. None of this shows what any shipping scanner does.

The curve, correction, exposure constant and toe were frozen on the original 14 frames. The search method and exposure mix were chosen after seeing the two harder rolls, so their scores on those rolls are optimistic.

| Tested | Effect | Decision |
| --- | --- | --- |
| Per-frame gray curves, against per-channel black and white points | Color error from 10.3 to 7.7 on the original frames, 11.2–18.1 to 8.0 on forest and 13.9–16.0 to 8.0 on metro. Brightness error from 19.8 to 8.8, 17.7–57.7 to 6.7 and 22.7–39.0 to 10.7. Ranges run from a hand-set area to the area as shipped. A prototype with looser alignment and a slightly different mask reached color error 7.7–8.8 | Adopt |
| Curve shape | Color error 8.26 with one bend, 8.81 with two, 8.90 free form | One bend |
| Where the curve is anchored | Color error about 8 through the film base. Anchored on highlights, 16–18, because blue skies aren't neutral. With a free offset from the base, worse on every group, at 15.2 on metro | Through the base |
| Search band for neutral pixels | Narrowing it with equal bins took color error from 14.9 to 9.4 on forest, and from 20.9 to 9.8 on its worst frame | Keep |
| Light pull toward an averaged curve | Color error from 8.26 to 8.10. A fixed 50% blend made it worse, at 9.18 | Keep the light pull |
| One curve per roll, against one per frame | Color error from about 8 to 8.8–12.5 | One per frame |
| RGB or CMY sliders, using the best possible per-frame offsets | Warm hues still drifted 21° | No such controls |

### Hardening the gray search

Tested on the main test set with the current film base detection, where the shipped search scores color error 7.55 and neutral cast 3.82.

| Tested | Effect | Decision |
| --- | --- | --- |
| Stronger pull toward the averaged curve | Color error 7.64 at the lightest step, double today's pull, and 11.42 on the averaged curve alone. Choosing the best pull or blend for each frame with hindsight reaches only 7.07 | Keep today's pull |
| Pull set per frame by how much gray evidence the frame has | Color error 7.73 on rolls held out of choosing the rule, though the best rule reached 7.42 on the frames it was chosen from. Pixel counts, density coverage and spread around the curve predict poorly which frames benefit | Reject |
| Leaving foliage hues out of the search | Color error 7.57–7.81 and neutral cast 4.19–4.60 at settings that change results, with a loose setting matching 7.55. Foliage turned teal on two forest frames, where color error rose by up to 6.4, because foliage is much of what the fit has to work with | Reject |
| Leaving sky hues out of the search | Color error 7.66–7.97 at settings that change results. On one forest frame it rose from 7.86 to 17.28 | Reject |
| Leaving foliage out, unless that removes over 15% of a pass's candidate pixels | Color error 7.38 and neutral cast 3.76, with 11 frames better and 3 worse. The guard was chosen after seeing the failures, and nearby settings give 7.33–7.55. One frame gives almost 40% of the gain. On 10 color frames with no reference scan the average change was 0.53 ΔE, and at most 1.27. Crops looked much the same at contact-sheet size | Don't adopt: too small and too tuned to justify two constants and a hue rule |

### Crosstalk, exposure and toe

| Tested | Effect | Decision |
| --- | --- | --- |
| Density crosstalk correction | Hue drift from 18–21° to 7–12° on blues, and from 5–10° to 2–7° on warm tones. Greens drift about 7–10° further | Keep; greens are still to tune |
| Exposure from highlights and scene average | Brightness error across the three groups: 9.6–15.9 with the mix, 11.3–17.8 on highlights alone, 9.0–23.1 on the average alone. About 8 is the best any per-frame value reaches | Keep the mix, for its better worst case |
| Center-weighted scene average | Within about 1 of the plain average | Drop |
| Shared toe with γ 0.7 | Brightness error improved on all groups; settings chosen on the original frames | Keep |

### Predicting crosstalk from spectra

A forward model of the scan can predict the crosstalk correction instead of fitting it: integrate light, [sensor](https://www.gujinwei.org/research/camspec/db.html) and dye spectra to get how each dye reaches each channel, then invert that. Inputs were the measured D5100 curves, dye curves read from a published plot based on the [VISION3 50D datasheet](https://www.kodak.com/content/products-brochures/Film/VISION3_50D_5203_TI-2657_Technical-Data_EN.pdf), and a film mask fitted to the measured base colors. The scans used a CineStill CS-Lite (stated CRI 95+). Its spectrum isn't published, so the lights below are estimates.

Tested on the main test set with the current film base detection, where the fitted correction scores color error 7.55 and neutral cast 3.82. Saturation was set so each variant has the same overall chroma, because the model is about 2.9× as strong as the fitted correction and would otherwise score mostly as oversaturation. Hue errors use their own hue bins, on colored pixels only: warm, green and blue are 16.6°, 14.8° and 19.1° for the fitted correction, so they don't compare with the hue drift above.

| Tested | Effect | Decision |
| --- | --- | --- |
| Predicted correction, for the estimated lights | Color error 7.88 with a broad 5000K light (2 frames better, 19 worse), and 8.12–8.19 with white LED estimates. With no correction it is 7.76. Hue error 19.3°, 13.5° and 21.5° for the broad light, so only greens improve | Keep the fitted correction |
| Predicted correction for narrowband RGB, a light these scans didn't use | Color error 7.45 and neutral cast 3.66, better than the estimates of the real light, mainly through a weaker red row | The model's light dependence isn't supported by these scores |
| Direction and strength, against the fitted correction | Same direction at about a third of the strength (cosine 0.86). The model at half strength scores 7.65, against a median of 9.32 and a best of 7.46 for 24 random corrections of that strength | The fitted correction has a physical basis, but the model doesn't set its strength |
| Fitted correction with the model's green row | Color error 8.00 halfway and 8.42 in full, 20 frames worse | Reject |
| Fitted correction with the model's blue row, and saturation about 1.0 | Color error 7.35 and neutral cast 3.67, with 17 frames better and 1 worse. Hue error 17.3°, 11.6° and 17.9°. The change is −0.20, with a 95% range of −0.45 to +0.06 when resampling by roll, and the row was chosen on these same frames. Across other lights and dye shifts: 7.35–7.49. By eye on a contact sheet it reads marginally less saturated, with no significant difference | Don't adopt: too small to tell from zero, and not visible enough. It would need frames not used to choose it |
| Sensitivity of the prediction | The blue-from-green term is −0.88 for the base case. It moves to −0.46 with magenta dye shifted 15 nm, and −1.53 with dye side absorptions added. Lights give −0.59 to −1.24 | Not reliable enough to generate a matrix without real dye data and the light's spectrum |
| Prediction across the 28 cameras in the database, with one light | Blue-from-green from −0.41 to −2.40 (standard deviation 0.41), and red-from-green from −0.12 to −1.11 | A correction fitted to one camera may not suit others. No per-camera matrices on this evidence, and a second camera is untested: the database has no curves for our Fujifilm scans |

The sensor database is under CC BY-NC-SA, so check its terms before shipping any matrix derived from it.

### Measured area

| Tested | Effect | Decision |
| --- | --- | --- |
| Whole scan | A black holder edge landed on the white point | Reject |
| Frame detection and holder detection | Holder detection reached color error 7.79 and brightness error 8.79, no better than a plain centered cut | Remove both |
| Centered area from 95% to 70% | Color error from 7.83 to 7.47. Brightness error 11.09 at 95%, where the area takes in holders, falling to 8.48 at 80% and 8.59 at 70%. At 85%: 7.63 and 8.69, with the lowest neutral cast, 3.77 | 85%, adjustable, with drawing by hand as the fallback |

### Leaving the holder out of the measuring

The first holder test above had only the D5100 frames, where 85% already clears the holder. A second camera, with square frames inside a 3:2 scan, keeps the holder inside any area wide enough to hold the frame. Blocked light reads near black in the scan, so it took the white point: 7 of 12 frames opened far too dark, with exposure anchors of 1.45–1.87 against 0.58–0.97 at a 55% area. Those 12 frames have no reference scans, so 55% is a comparison and not a target.

| Tested | Effect | Decision |
| --- | --- | --- |
| Tighter area on the second camera | Every frame settled by 55%. At 60%, 3 frames were still off | Too tight a default for frames that fill their scan |
| Dense cells connected to the scan's edge, against every dense pixel | Exposure gap from the 55% result: from a mean of 0.45 and a worst of 1.02 to a mean of about 0.05 and a worst of 0.13. A plain density limit behaved about the same on these frames, but also takes dense picture inside the frame | Connected to the edge |
| Density limit of 1.0, 1.2, 1.4 and 1.6 | 1.0 took real picture on 2 frames, growing the mask from 6.6% to 15.9% and from 0% to 7.8% of the scan. 1.6 left the anchor 0.22 off on 1 frame | 1.2 |
| Margin around the holder, from 0 to 80 analysis pixels | Without one, the holder's soft edge misled the gray fit: 18.7 ΔE from the 55% result on 1 frame and 3.9 on another. At 10 to 40 pixels, about 0.6 and 1.0–1.5 | 20 on a 1,508-pixel side, as a share of the long side |
| Masks on 44 frames, judged by eye at contact-sheet size, with no hand-labeled pixels | No holder missed and no main picture taken. 3 frames lost up to 0.2% of the scan on a neighboring frame at the edge, with no change in exposure. No mask reached the central 50% | Use by default. If dense cells reach the center, they aren't treated as holder |
| Reference frames at each area, without and with the mask | At 85%, color error, brightness error and neutral cast didn't change (7.55, 8.67, 3.82). At 100%, 8.86, 16.80, 3.59 became 7.75, 8.76, 4.08. At 95%, 7.71, 10.92, 3.89 became 7.73, 8.81, 4.02 | Keep 85% as the default |

The settings were chosen on these same frames, and the holders tested are opaque. A translucent holder or one that doesn't reach the scan's edge won't be found. A picture with dense areas touching the edge, as on a scan filled edge to edge, can lose them from the measuring. That's why the mask shows in Show areas, and a drawn area replaces it.

### Saturation and demosaic

Measured against 8 hand-finished scans of one film stock.

| Tested | Effect | Decision |
| --- | --- | --- |
| Linear saturation after inversion, against the same correction in density | Color error from 8.66 in density to 7.77 in linear. A calibrated 3×3 matrix is untested | Linear, after inversion |
| Balancing to the film base before demosaic | Fine color noise down 6.9%, fine luminance detail down 3.2%. Crops looked practically identical | Don't adopt |

## Technical solutions

### Raw decoding

| Option | Pros | Cons | Outcome |
| --- | --- | --- | --- |
| [LibRaw](https://www.libraw.org/), [LGPL 2.1 or CDDL 1.0](https://third-party-mirror.googlesource.com/libraw/+/refs/heads/master/LICENSE) | Reads 1,000+ cameras and DNG; gives levels and matrices | New cameras need LibRaw updates | Chosen, under the CDDL in every build; see [License](#license) |
| [Adobe DNG SDK](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html), 1.7.1 build 2611 | Every DNG feature, including JPEG XL | Reads DNG only; terms [not stated on the download page](https://community.adobe.com/questions-563/adobe-dng-sdk-terms-of-use-licensing-182173) | Rejected |
| [Adobe DNG Converter](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html) | Adobe's camera coverage, with a command line | Separate install, macOS and Windows only | Fallback for users with unsupported cameras |
| Convert in a raw editor first | No work for us | Extra manual step | Rejected |
| rawler from [DNGLab](https://github.com/dnglab/dnglab) | Pure Rust, LGPL-2.1 | Self-described alpha, unstable API, may panic on bad input | Rejected |

We also decided against our own raw-to-DNG step, since it adds nothing to the inversion.

Building LibRaw without OpenMP, lcms and libjpeg kept DNGs byte-identical on three test files. The cost is that DNGs with lossy JPEG compression can't be read.

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

### How the DNG behaves in editors

Hand-made positive DNGs from D5100 samples settled the format. Capture One and darktable weren't checked.

| Tested | Effect | Decision |
| --- | --- | --- |
| 16-bit integer with about 2.3 stops of headroom | Works in Lightroom Classic, Lightroom mobile and Apple Photos, and Highlights and Exposure recover the headroom | Keep |
| Float16 with Deflate | Works in Adobe apps, fails in Photos and Finder | Reject |
| Several embedded profiles | Lightroom Classic and mobile list all three; in Classic, only the Profile browser shows them. Photos shows no choice but applies the main profile's curve | Embed all three, with the chosen one as main |
| What the headroom keeps | Display clipping is recoverable up to the stored limit, and real loss was 0–0.7% of the picture. On one frame cropped to the image, Whites −100 took display clipping from 3.0% to 0% | Tone shaping stays in the profile curve |
| A profile with no tone curve | Looked blown out | Every profile has a curve, which led to Flat and Vivid |
| Initial white balance in Lightroom Classic | None shown until As Shot was reselected | Write `crs:WhiteBalance="As Shot"` in the XMP |
| A shoulder in the profile's own curve, with a −2 EV offset, in Camera Raw | Looks the same as the standard DNG, so Camera Raw follows the curve and adds no compression of its own | Flat can roll off in its curve |
| Flat as the main profile, in Apple Photos | With a −2 EV offset on the main profile it opened too bright: average 188 out of 255, against 147 if the offset were honored and 227 if ignored. Lowering BaselineExposure itself, with +2 EV on the other profiles, fixed it and looks the same in Camera Raw and Finder previews | BaselineExposure follows the main profile. Photos keeps its own rendering, with a slight shift left |

### Language

| Option | Pros | Cons | Outcome |
| --- | --- | --- | --- |
| Rust | Memory-safe parsing of untrusted files; easy distribution | LibRaw is C++, so it needs FFI | Chosen |
| C++ | LibRaw and the DNG SDK link directly | Parser memory-safety burden | Rejected |

### Speed and file size

The pipeline is per-pixel and runs in plain Rust. GPU work remains an untried option.

| Tested | Effect | Decision |
| --- | --- | --- |
| Pixel work on all cores | A preview from 65 ms to 17 ms, a DNG from 1.1 s to 0.8 s, byte-identical to the earlier build on one NEF | Keep |
| Decoding frames in parallel, with an OpenMP build of LibRaw | Twelve frames took 162 s, against 4.4 s with the development build and 6.1 s with the app's, each decoding one frame at a time | One decode at a time; the plain LibRaw we link isn't thread-safe either |
| File size with lossless JPEG | About 62–64 MB for a 16 MP D5100 scan, on two frames. Other sizes and JPEG XL weren't measured | No change |

### License

The aim was a program that stays open source and can be published and sold in app stores. Reversal has one copyright holder and no code ported from other programs, so it could change license.

| Option | Pros | Cons | Outcome |
| --- | --- | --- | --- |
| GPL with added permissions | The whole program stays open | Anyone passing it on can remove an added permission (GPL section 7), and LibRaw's LGPL option asks that users can relink it, which is hard to offer through an app store | Rejected |
| [MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/) | Changes to existing files stay open. Section 3.2 lets the executable go under different terms as long as recipients' source rights stay intact. No conflict with LibRaw under the [CDDL 1.0](https://opensource.org/license/cddl-1-0), whose section 3.5 says much the same | New files in a fork may be closed | Chosen |
| Permissive licenses | Fewest conditions on distribution | Improvements can be closed entirely | Rejected |
| Source-available | Full control over selling it | Not open source | Rejected |

There's App Store precedent for the MPL: [VLC for iOS returned in 2013](https://www.videolan.org/press/ios2out.html) with the MPL 2.0 as the license for that distribution, and [Firefox for iOS](https://github.com/mozilla-mobile/firefox-ios/blob/main/LICENSE) is under it.

The effect is on licenses and notices only: LibRaw's version and build options stay the same, the source of LibRaw and the crates stays attached to each release, as CDDL section 3.1 and MPL section 3.2 require, and DNGs stayed byte-identical on three test files.

## Sources

### Patents

- [US6160634: neutral estimation and gradation balance](https://patents.google.com/patent/US6160634)
- [US6081343: per-film balance tables](https://patents.google.com/patent/US6081343)
- [EP0612183B1: density-dependent gray balance](https://patents.google.com/patent/EP0612183B1/de)
- [US6233069: printing density, toe and scene balance](https://patents.google.com/patent/US6233069)
- [US6204940B1: inversion and shared inverse-S](https://patents.google.com/patent/US6204940B1)

### Color science

- [Wikipedia: Cineon](https://en.wikipedia.org/wiki/Cineon)
- [Camera spectral sensitivity database](https://www.gujinwei.org/research/camspec/db.html)
- [Kodak VISION3 50D technical data](https://www.kodak.com/content/products-brochures/Film/VISION3_50D_5203_TI-2657_Technical-Data_EN.pdf)

### DNG and raw decoding

- [Adobe DNG downloads: spec 1.7.1, DNG SDK 1.7.1 build 2611, command-line spec, patent license](https://www.adobe.com/si/support/downloads/dng/dng_sdk.html)
- [Adobe Community: DNG SDK terms of use](https://community.adobe.com/questions-563/adobe-dng-sdk-terms-of-use-licensing-182173)
- [Adobe Community: why Denoise DNGs are so large](https://community.adobe.com/t5/lightroom-classic-discussions/why-are-dng-files-produced-bt-denoise-ai-6-times-larger-than-the-orignal/m-p/13748097)
- [Life After Photoshop: Lightroom and iPhone ProRAW linear DNGs](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/)
- [DxO forum: DNG 1.7 JPEG XL support across editors](https://forum.dxo.com/t/compressed-dng-brought-to-pl9-but-with-a-bug/54494/38)
- [LibRaw](https://www.libraw.org/) and its [license (LGPL 2.1 / CDDL 1.0)](https://third-party-mirror.googlesource.com/libraw/+/refs/heads/master/LICENSE)
- [DNGLab / rawler on GitHub](https://github.com/dnglab/dnglab)
