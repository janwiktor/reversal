# Reversal: Research Notes

> **A working file.** These notes index every choice, test and experiment made along the way, including options that were later replaced or rejected. They are kept for the record, not as a reading guide. For the current method end to end, with its argument and formulas, see [Conversion Method](conversion-method.md).

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
| Sprocket holes cut by the scan's edge, or not clipped, and grainy film: 224 frames, 126 of them reported as having no clear film | On 40 frames the holes hadn't clipped, so the brightest even patches were light through them, about 45% above the film in green. On others, grain made the film's edge uneven pixel by pixel: it varied 6–10% within a cell, against 2–4% on frames that read well. Looking below bright patches too short for a film edge, when they stand 0.1 in density clear of the rest and have even film of that next level around them, and judging a cell by the means of its quarters when the pixel test finds nothing, found the base on 118 more frames, 216 in all. The 98 frames found before read exactly as they did, because the pixel test still comes first. Judged by quarters, smooth picture passes too: 2 frames took a patch of picture at the edge until such a patch had to span a quarter of a side, which gave up 1 frame with a short clear edge. Checked by eye on contact sheets | Adopted. Model version 4 |
| Coarse grain: 5 frames of one roll with clear film all around the picture, all reported as having no clear film; 233 frames in all | A pixel of the clear film varied by 6% of its level, so the means of a cell's quarters differed by 3–8%, and about a third of the cells failed the 5% test: no patch ran a quarter of a side. The light also fell off by 15% across the scan, which left only the brighter side's edge within reach. Letting the quarters differ by 6 times the grain left in their means, with grain measured between neighboring pixels and capped at 8% of the level, and taking in single cells that still fail between two that pass, read the full edge on all 5, within 1% of each other. Such a patch must be two cells thick, since a line one cell thick of textured picture along the edge of a scan with no clear film passed the same way; no frame needed a thinner one. Used in place of the quarter test, it also passed smooth picture brighter than the film's edge and lost 2 frames; as a third search after it, the 220 frames found before read exactly as they did. Of the 8 left, 1 fills the scan, 4 have an edge of a few pixels beside clipped light, 2 are misprocessed film, and on 1 the light falls off by more than 15% along a film edge broken up by holes and markings, so no stretch of it stays within the 7% that counts as one level | Adopted as a third search |
| Searching for the film base only outside the measured area, which is mostly film edge, on the same 233 frames | Narrowing the edge band from 15% of each side to the 7.5% outside the default measured area found 1 frame and lost 1, and moved 9 readings by 2–4%. Setting the level of clear film from the edge band alone, so that bright picture can't raise it, found 1 more frame, of misprocessed film, and with the narrow band took light through holes on 2 frames, reading 81% and 106% high. Neither let a search be dropped: the grain search alone lost 44 frames, and in place of the quarter search it still lost 2. Without the search across the scan, 3 frames whose only clear film is the gap between frames were lost; a square picture in a 3:2 scan relies on it too, since the film beside it lies inside the measured area. Widening the level window from 7% to 10% found the frame with uneven light, and moved 58 readings 4–7% darker by taking in denser film | Not adopted. The edge band, the search across the scan and the 7% window stay |
| Letting the level of clear film drift along a patch, for scans where it falls off across the film, on the same 233 frames and on their centers cropped to 60% and 75%, which are mostly picture | Bare light clips in green and blue, and in red it fell 6.5% across a frame whose clear film fell 10.5%, so light explains only part of it, and no correction of the light was tried. Instead a patch could grow from the brightest clear film through even cells, each within a set step of its neighbor, down to a floor. Used in every search, it lost 1 frame and moved 58 readings 3–12% darker. As further searches after the three, it moved none, and at a 1.5% step and a 20% floor found the frame with uneven light and 1 of misprocessed film, but read smooth picture as film on 11 more of the 60% crops, sky and snow. At a 0.5% step and a 12% floor it found the one frame, 5% darker than its neighbor on the roll, and still read picture on 1 and 2 more crops. That frame needed the grain search: with pixel or quarter evenness alone nothing was found | Not adopted: one frame gained doesn't justify reading picture as film. The 7% window from one level stays |
| How close a patch must be to the brightest even areas | A 10% range lowered the base by up to 7.6% on frames that already read well | Use 7% |
| Current detection on the main test set, against the old one, with the 85% measured area | Color error 7.55 against 7.63, brightness error 8.67 against 8.69, neutral cast 3.82 against 3.77. The old scores came from earlier scripts, so this is close rather than exact | No score penalty; keep |
| No film base: each channel counts density from its own thin end, the 99.5th percentile of light in the measured area, on the argument that nothing on film is thinner than its base. Gray curves, anchor and black point as shipped, exposure offset and saturation set again, scored under the Neutral curve, where the shipped conversion scores color error 6.91, brightness error 7.61 and neutral cast 3.34 | With the film edge in the measured area where the scan shows it: 6.82, 7.59 and 2.97, with 12 frames better and 10 worse. The thin end sat 0.04 above the base in green, and blue's 0.015 below green's. The 99.9th and 99.99th percentiles scored 7.16 and 7.39, with blue varying 0.08 between frames. With bare film left out, as on a scan cropped to the picture: 7.11, 7.48 and 3.01, with 11 better and 11 worse, and one frame 6.6 worse. With everything thinner than 0.1, 0.2 and 0.3 above the base left out as well, as on a frame with no deep shadows, and scored on what remains: color error 9.27, 11.83 and 15.52 against 6.91, 6.78 and 6.65. Dropping the gray curves too, for red and blue as straight lines of green between each channel's thin and dense ends: 9.9–10.4, neutral cast 7.8–8.6 | Keep the film base. The thin end stands in for it only while the frame holds film near base density, and the conversion can't tell when it doesn't. Untested as a fallback for frames with no clear film |

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
| Pull toward the roll's own curve instead of the averaged one. The roll curve was pooled from the roll's other frames, 1 to 9 of them. Tested with the black point and the engine's current film base on every frame, where the shipped search scores color error 7.34 and neutral cast 3.77 | Frames of a roll agree more than frames across rolls: their curves spread 0.45–0.84 times as far within a roll as roll averages do from each other. At 5 times today's pull, color error 7.19–7.21 and neutral cast 3.81–3.84, with 12 frames better and 10 worse. The gain is two frames of one roll, the set's worst, pooled with frames whose place in that roll was a guess: without that roll the change is −0.03 to +0.03, and the largest roll, 10 frames, got worse at every strength. Choosing the pooling and the pull on held-out rolls gave 7.34, level with the shipped search. A blend weighted by each frame's gray evidence scored 7.47–8.03, and the roll curve alone 8.82 | Reject. Untested on a whole roll of 36 frames |
| Leaving foliage hues out of the search | Color error 7.57–7.81 and neutral cast 4.19–4.60 at settings that change results, with a loose setting matching 7.55. Foliage turned teal on two forest frames, where color error rose by up to 6.4, because foliage is much of what the fit has to work with | Reject |
| Leaving sky hues out of the search | Color error 7.66–7.97 at settings that change results. On one forest frame it rose from 7.86 to 17.28 | Reject |
| Leaving foliage out, unless that removes over 15% of a pass's candidate pixels | Color error 7.38 and neutral cast 3.76, with 11 frames better and 3 worse. The guard was chosen after seeing the failures, and nearby settings give 7.33–7.55. One frame gives almost 40% of the gain. On 10 color frames with no reference scan the average change was 0.53 ΔE, and at most 1.27. Crops looked much the same at contact-sheet size | Don't adopt: too small and too tuned to justify two constants and a hue rule |
| Gray curves measured along the mean of the three densities, or along densities weighted by the light each channel gets through the base, instead of along green | Color error 7.62–7.71 and neutral cast 3.92–4.00, with 3–7 frames better and 8–10 worse. Brightness error 8.68, against 8.67, when tone still came from green, and 8.78–8.96 when it came from the new axis, which darkened tones below L\* 70 by about 0.5 on a typical frame from one camera and 1.5–2 on the other. Green was the least noisy axis almost everywhere, so a steadier fit had little to gain. Gray balance varied between frames scanned together about as much as before on one camera, and slightly more on the other. On 43 frames from two cameras, most changed by less than 1 ΔE. Of the few that changed visibly, one lost a pink cast and the others turned slightly yellower, so no axis looked better overall | Keep green |

### Crosstalk, exposure and toe

| Tested | Effect | Decision |
| --- | --- | --- |
| Density crosstalk correction | Hue drift from 18–21° to 7–12° on blues, and from 5–10° to 2–7° on warm tones. Greens drift about 7–10° further | Keep; greens are still to tune |
| Exposure from highlights and scene average | Brightness error across the three groups: 9.6–15.9 with the mix, 11.3–17.8 on highlights alone, 9.0–23.1 on the average alone. About 8 is the best any per-frame value reaches | Keep the mix, for its better worst case |
| Center-weighted scene average | Within about 1 of the plain average | Drop |
| Exposure measured on textured areas only: a cell votes when the average densities of its four blocks span more than a threshold, so even sky, walls and bare film don't count, as in a photofinishing patent on scene analysis. Tested with the black point and the engine's current film base on every frame, where the shipped anchor with its offset set again scores brightness error 8.26, and 7.62 under the Neutral curve | At full strength, no block size from 4 to 16 or threshold from 0.03 to 0.12 beat the shipped anchor: 8.41–9.14, and 7.75–8.57 under Neutral. A half pull toward the middle of the textured cells' 5–95% range reached 8.00 and 7.38, with 12 frames better and 10 worse; the same pull toward all cells gave 8.31, so the gain is the texture's. On rolls held out of choosing the settings it scored 8.37 against 8.34, and 8.00 against 7.70 under Neutral. At full strength it fixed the anchor's worst frame, 16.5 to 11.8, and cost another frame 4.5. The best anchor per frame with hindsight reaches 7.12 and 6.32 | Reject: it moves error between frames, and doesn't hold on held-out rolls |
| Shared toe with γ 0.7 | Brightness error improved on all groups; settings chosen on the original frames | Keep |

### Black point and contrast

A low-contrast frame from outside the test set looked muddier than its finished scan. The picture spans 0.85 of green density, and its darkest content sits 0.24 above the film base. The engine counts black from the film base and uses one film gamma, so the Neutral curve left the picture between L\* 20 and 90, at its 1st and 99th percentiles, where the finished scan runs from 0.6 to 98. The finished scan is close to each channel stretched over its own density range.

The variants were tested on the main test set with the current film base detection. Each got its own exposure offset, for the lowest mean brightness error, and its own saturation gain, for the same chroma ratio as the shipped conversion (the conversion's mean chroma over the finished scan's), so none scores on being darker or more saturated. With its exposure offset set the same way, the shipped conversion scores color error 7.59 and brightness error 8.48 under the fixed preview curve the other tests use, and 7.10 and 8.13 under the Neutral curve. Frames better and worse are counted against those.

| Tested | Effect | Decision |
| --- | --- | --- |
| On that frame: the finished scan's tone curve on the conversion, one curve for all channels | Chroma ratio from 0.64 to 0.96, with color error still about 10. Temperature +20 and Tint +5 then took color error from 10.1 to 6.0. That balance left the sky about 4 pinker and 4 yellower in a\* and b\*, and the sea about 2 pinker and 4 yellower, and refitting the gray curves to the finished scan as well didn't remove it | The gap is tone and a warmer balance. The gray curves are within about 0.03 density of the finished scan's neutrals |
| Subtracting a share of the frame's darkest light, the same in every channel. Darkest is the 0.5th percentile of green equivalent density in the measured area, bare film left out | At half: color error 7.35 and brightness error 8.26, with 19 frames better and 2 worse by color, and 12 and 10 by brightness. Under the Neutral curve: 6.95 and 7.61, with 16 and 6, and 19 and 3. The 1st percentile of L\* sat 5.1 above the finished scans', from 11.6, or 7.4 from 13.2 on the center 70% of each scan, which leaves out film edges and holder. The saturation gain falls from 1.25 to 1.12. At three quarters: 7.28 and 8.53. In full: 7.27 and 9.08. Percentiles from 0.1 to 2 gave color error 7.35–7.37. On one roll of four frames brightness error rose from 8.92 to 9.20 under the preview curve, and stayed level under the Neutral curve, 8.20 and 8.21 | Adopt at half, for color film, as conversion model 5: a small gain in color on most frames, and in brightness under the Neutral curve. Black and white keeps black at the film base, untested |
| Toe placed at the frame's darkest density, not at the film base | Color error 7.53 and brightness error 8.22. With the toe's amount raised from 0.3 to 0.45: 7.50 and 8.21, with 17 frames better and 4 worse by color, and 12 and 10 by brightness. Under the Neutral curve: 6.98 and 7.41, with 19 and 3, and 18 and 4 | Reject for the subtraction: less gain in color and in the shadows, though more in brightness under the Neutral curve |
| Film gamma set per frame from the frame's density range, at a quarter, half and full strength | Color error 7.74, 7.93 and 8.38, and brightness error 8.52, 8.67 and 9.23. At half, 8 frames better and 14 worse by color | Reject |
| Film gamma of 0.65 or 0.6 for every frame | Color error 7.67 and 7.77, and brightness error 8.71 and 9.14. At 0.65, 3 frames better and 19 worse by color. Under the Neutral curve 0.65 is level, at 7.13 and 8.13 | Keep 0.7 |
| What predicts how much more contrast a finished scan has | Finished scans have roughly 10–15% more contrast than the conversion under the Neutral curve, depending on the measure. How that relates to a frame's density range wasn't settled: the measure took in film edges and holder, and two frames swing the correlation between −0.1 and −0.6 | No per-frame contrast: the direct test above scored worse |
| The subtraction on that frame | The 1st percentile of L\* fell by 4 at half and by 10 at three quarters, and stayed over 11 above the finished scan's. Color error stayed at 10.1 | That frame's look is contrast and balance, which stay with the sliders. Blacks at −100 alone leaves its 1st percentile of L\* near 6 |
| The subtraction at half as the engine does it: anchor offset 0.0777, from 0.1027, and saturation gain 1.12, against model 4 as it was | Color error 7.34 from 7.60 and brightness error 8.52 from 8.78, with 19 frames better and 3 worse by color, and 12 and 10 by brightness. Under the Neutral curve: 7.00 from 7.21 and 8.38 from 8.84, with 16 and 5, and 17 and 5. The 1st percentile of L\* sat 4.5 above the finished scans', from 10.7. On a 0.005 grid the subtraction's best exposure offset lay 0.020 below the baseline's under the preview curve and 0.025 under the Neutral curve, with saturation gains of 1.124 and 1.116. The engine's black level and pixels matched the test's on the frame above | Anchor offset 0.0777 for color, saturation gain 1.12 |

The strength, exposure offset and saturation gain were chosen on the same 22 frames they are scored on. The scores here are from the film base the engine finds today on every frame; on 5 frames an earlier run had used a base from before the film base searches of model 4, which put every score about 0.05 lower and changed no decision.

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

### Black and white

Black and white negatives carry one silver image, which blocks every color about equally, so the color model's gray curves, crosstalk and saturation have nothing to correct. The conversion keeps density above base per channel and averages the channels into one density, weighted by the light each gets through the base: shot noise in density falls as the signal grows, so this weighs each channel by the inverse of its noise. Toe, exposure anchor and γ are shared with color. There are no black and white reference scans, so the conversion was checked by eye on 6 frames from 6 rolls and two cameras, including Ilford HP5 Plus and FP4 Plus. In Apple's raw engine, the DNG opened gray and matched the app's preview.

Film type detection was tested on 220 scans: 160 black and white and 60 color, from a Nikon D5100 and a Fujifilm camera. Color includes several stocks, CineStill 400D, and a misprocessed roll with weak color. Every frame's type was confirmed by eye where the base color didn't settle it. The scores below are on the measured area, as density above base.

| Tested | Effect | Decision |
| --- | --- | --- |
| Base color, red against blue | On one light, black and white 0.62–0.72 and color 1.14–2.16. It depends on the light and white balance, and misread a color frame whose base detection landed on bare light | Reject |
| Spread around the frame's own gray curve, per analysis pixel (90th percentile) | Black and white reached 0.027 from grain and noise, color started at 0.025 | Reject: they overlap |
| The same on cells of 4 by 4 analysis pixels | Black and white 0.004–0.011, color 0.0197–0.18. The lowest color frames are near-gray scenes | Keep, below 0.02 |
| Slope of red against green density along that curve | Black and white 1.000–1.024 on both cameras, color 0.67–0.93, also on the near-gray scenes | Keep, within 0.05 of 1 |
| Both together | All 220 frames right. Spread alone would have taken a near-gray color frame, at 0.0197, for black and white | Adopt |
| Density bins from the base up, as for the gray curves | A black and white frame filling the scan, with no clear film, had its base guessed from the picture, so half its cells sat below it, and too few bins remained to tell | Bins reach 1.0 below the base: on black and white a guessed base moves every channel alike |

A frame needs 6 filled density bins, or it stays color. Both limits were chosen on these frames, and no frames were held out to check them.

The color conversion is unchanged: DNG pixels stayed byte-identical on two files, one from each camera.

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
| Black and white | Three equal channels, one channel | Three equal channels in linear Rec.2020, the file every editor already opens. One channel would be about a third of the size, and editors would treat it as a monochrome camera's, but it's untested in Photos and Lightroom. Apple's engine renders the three-channel file gray under the black and white profiles |
| Original in the file | Embedded, left out | Left out: [embedding it is why Denoise DNGs reach about 4× a raw](https://community.adobe.com/t5/lightroom-classic-discussions/why-are-dng-files-produced-bt-denoise-ai-6-times-larger-than-the-orignal/m-p/13748097), against [2–3× for linear DNGs generally](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/) |

ProRAW is [a linear DNG, demosaiced before Lightroom sees it](https://lifeafterphotoshop.com/lightroom-now-supports-apple-iphone-13-proraw-dngs-directly/), which is the precedent for this file type.

### How the DNG behaves in editors

Hand-made positive DNGs from D5100 samples settled the format. Capture One and darktable weren't checked.

| Tested | Effect | Decision |
| --- | --- | --- |
| 16-bit integer with about 2.3 stops of headroom | Works in Lightroom Classic, Lightroom mobile and Apple Photos, and Highlights and Exposure recover the headroom, except Exposure in Photos (below) | Keep |
| Float16 with Deflate | Works in Adobe apps, fails in Photos and Finder | Reject |
| Several embedded profiles | Lightroom Classic and mobile list all three; in Classic, only the Profile browser shows them. Photos shows no choice but applies the main profile's curve | Embed all three, with the chosen one as main |
| What the headroom keeps | Display clipping is recoverable up to the stored limit, and real loss was 0–0.7% of the picture. On one frame cropped to the image, Whites −100 took display clipping from 3.0% to 0% | Tone shaping stays in the profile curve |
| A profile with no tone curve | Looked blown out | Every profile has a curve, which led to Flat and Vivid, now Hard |
| Initial white balance in Lightroom Classic | None shown until As Shot was reselected | Write `crs:WhiteBalance="As Shot"` in the XMP |
| A shoulder in the profile's own curve, with a −2 EV offset, in Camera Raw | Looks the same as the standard DNG, so Camera Raw follows the curve and adds no compression of its own | Flat can roll off in its curve |
| Flat as the main profile, in Apple Photos | With a −2 EV offset on the main profile it opened too bright: average 188 out of 255, against 147 if the offset were honored and 227 if ignored. Lowering BaselineExposure itself, with +2 EV on the other profiles, fixed it and looks the same in Camera Raw and Finder previews | No exposure offset on the main profile. Since then, no look needs one (below) |
| Black level in Apple's raw engine, on gray ramps through `CIRAWFilter` | By default it subtracts one (`shadowBias` 5), crushing shadows: 8–32% of pixels went black on four frames, against about 0.3% expected. DefaultBlackRender set to None turns it off | Write DefaultBlackRender on every profile |
| ProfileToneCurve in Apple's raw engine, on the same ramps | Unevenly spaced samples are misread at any count: a Flat midtone read 0.27 instead of 0.38. Evenly spaced samples read exactly from about 33 up to 1024. Over 1024, or a curve not reaching (0, 0) and (1, 1), is replaced by a default curve, which also broke Blacks above 0 | 1024 evenly spaced samples, with both ends pinned |
| Data above display white | `CIRAWFilter` and `sips` clip it, and Photos compresses it only partly: Neutral's brightest tones read up to 3 L too bright in Photos. With the roll-off in the curve and BaselineExposure 0, they came within 0.2 L. Camera Raw opened the same (ΔE 0.15) and its sliders behaved as expected | Every curve carries the roll-off, and BaselineExposure is 0 for every look |
| Exports of one frame from Photos and Camera Raw, same file, before and after those changes | Neutral: ΔE 6.10 to 1.37, Photos darker by 4.8 L, then 0.6 L. Flat: ΔE 10.89 to 1.30, darker by 10.7 L, then 0.7 L | Keep. What's left is that lightness difference and about 0.6 toward green in colored areas |
| The same changes in Camera Raw | Opens 0.7 L brighter on Neutral and 1.5 L on Flat, which fits a small black subtraction, about 0.004 linear, going away. No export isolated the tag. The app's preview then matches it within 0.1 L on average, where it was 0.7 L and 1.5 L brighter | Accept: the preview, Camera Raw and Photos end up closer together |
| `boostShadow` in Apple's raw engine | Its default of 0.9 lowers deep shadows slightly, and no tag controlling it was found | Leave it |
| Exposure edits in Apple Photos, on gray ramps and a frame with sun patches (`_DSC4438`), rendered through Photos' own edit pipeline | Editing a raw adds a raw adjustment, and Photos then applies Exposure through a tone curve rebuilt from the main profile's curve at stored 0.2, 0.5 and 0.8. Neutral reads 0.68, 0.94 and 0.99 there, a Nikon raw about 0.43, 0.80 and 0.96. At +0.15 the frame's clipped share rose from 2.5% to 7.0%, against 3.3% without the raw adjustment, and saturated colors near white turned white. A camera NEF with a heavy highlight shows the same artifact in Photos | A limitation of Photos' raw editing, not of the file. The spec gives the workaround |
| Giving Photos camera-like samples, by storing frame white higher or with BaselineExposure +1 and a remapped curve | Photos' edits then follow its unedited rendering, but data above about 2.5–3 times frame white clips, in the file or by default in Apple's engine. On `_DSC4438` that is the sun patches, and clipping in Photos' edits stayed at 6.9% | Reject: it gives up headroom in every editor |

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

### Memory of a save

A phone gives an app a few gigabytes at most, and a full scan is 12 bytes a pixel as 32-bit float RGB: 193 MB at 16 megapixels, 730 MB at 61. Saving one 16-megapixel frame held 1,150 MB at its peak, measured as the process's peak memory footprint on a Mac. It kept the scan, a straightened copy, a cropped copy and a rotated copy at once, even when none of the three changed anything, then a 16-bit copy of the whole positive for the DNG.

| Tested | Effect | Decision |
| --- | --- | --- |
| Converting in the scan's own memory: straightening, cropping, mirroring and a half turn copy nothing when they have nothing to do, and crop, mirror and half turn work in place | One image where there were four. A quarter turn and a straighten still need a second image while they run | Keep |
| Storing samples a strip or a tile at a time, in place of a 16-bit copy of the image | 6 bytes a pixel less | Keep |
| Closing LibRaw before making the float image from its output | Its working buffers, about 10 bytes a pixel, are gone before ours is made | Keep |
| Asking once for the room the DNG's bytes need | 60 MB less at 16 megapixels: no doubling and copying as the file grows | Keep |
| Asking Apple's allocator to hand freed blocks back (`malloc_zone_pressure_relief`) | No effect on the peak | Dropped |

Together: 1,150 MB to 533 MB for a 16-megapixel frame, and 1,803 MB to 1,204 MB for a 24-megapixel X-Trans frame with a quarter turn. Eight conversions of a DNG, a NEF and a RAF, covering every turn, mirror, straighten, crop and both compressions, are byte-identical to the build before. A save takes the same time.

About half of what remains is memory already freed that macOS's allocator keeps for reuse. With that cache off (`MallocLargeCache=0`) the same saves peak at 280 MB and 626 MB, close to what the conversion holds at once: 18 bytes a pixel while decoding, 24 with a quarter turn. iOS keeps such a cache too. On an iPad Air (M3, iOS 27.2) the same 16-megapixel frame with a quarter turn peaked at 704 MB, and saving a roll of 138 frames peaked at 681 MB; opening a frame peaked at 505 MB. The app held 414 MB as the roll's save ended, 289 MB three seconds later and 36 MB after ten, with nothing asked of the allocator, and asking it then freed nothing more. That iPad allows an app about 5.1 GB.

An iPhone 16 Pro Max (iOS 27.0.1) allows about 3.4 GB. Each frame below was opened, turned a quarter and saved in a run of its own. There's no camera file above 24 megapixels at hand, so the two largest are uncompressed Bayer DNGs written for the test, which show what the conversion holds at that size and not what a camera's own format costs to decode.

| Frame | Peak after opening | Peak after saving |
| --- | --- | --- |
| 16 megapixels, NEF | 513 MB | 719 MB |
| 24 megapixels, X-Trans RAF | 823 MB | 1,068 MB |
| 45 megapixels, written DNG | 1,368 MB | 1,381 MB |
| 60 megapixels, written DNG | 1,806 MB | 1,854 MB |

So a 60-megapixel frame fits there with 1.5 GB to spare, and above 24 megapixels opening a frame costs as much as saving it. After a large save the app can hold up to 1.1 GB for more than a minute: freed memory that neither the allocator gives back when asked nor the engine holds in its cache. It's reused, not added to. Saves of 45, 60, 60, 45 and 16 megapixels in a row peaked at 2,093 MB, 240 MB above the largest alone, and the app was at 23 MB within a minute of the last. Phones with less memory are still to measure.

### Threads and saving off screen on iOS

Work on a whole image is shared out to a thread per core, and the thread that shares it out waits for them. On Apple's systems a thread started the usual way gets the default quality-of-service class, whatever class its starter has. The iOS app's requests reach the engine on a queue of the class for work the user waits for, so each preview had a thread of that class waiting for threads of a lower one: a priority inversion, which Xcode's Thread Performance Checker reported with `for_each_part` in its backtrace.

| Tested | Effect | Decision |
| --- | --- | --- |
| Threads that share out work take the class of the thread that waits for them, the class for drawing and input counting as the one below it | The checker reports nothing for opening a frame and saving a roll, where it reported the inversion before. The split of the work, and so the output, is the same | Keep |
| Lowering the queue's class to the default | Would also end the report, by making what the user waits for less urgent | Rejected |

A save continues off screen as a task iOS shows with its progress, but slower. On the same iPad a roll saved at 1.2 s a frame on screen and 5.8 s a frame off screen. About 1.2 cores were busy either way, and a frame cost 6.7 s of processor time off screen against 1.5 s on screen: the work isn't held back for lack of a turn, it runs on slower or slowed cores.

| Class of the threads that save | On screen | Off screen | Decision |
| --- | --- | --- | --- |
| Default, as threads start | 1.2 s a frame | 5.8 s a frame | Keep |
| The class for work the user waits for | 1.2 s | 5.9 s | No gain. Previews asked for during a save would also lose their lead over it |
| The class for long work with a progress bar | 3.1 s | 7.2 s | Rejected: slower on screen, no faster off it |

So the slowdown is how iOS treats an app that's off screen, and the app can't change it by ranking its threads. Returning to the app brings back full speed at once.

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
- [US5724456: brightness from textured areas of the scene](https://patents.google.com/patent/US5724456)

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
