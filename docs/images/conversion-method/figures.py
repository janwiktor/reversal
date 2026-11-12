#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Figures for docs/conversion-method.md, written as standalone SVG files next to this script.

    python3 docs/images/conversion-method/figures.py

Standard library only. Every curve is computed from the engine's own formulas and constants,
copied below from crates/reversal-core/src (engine 0.3.2, conversion model 3). When the engine
changes, update the constants here and run the script again.

Each SVG carries its own styles and a white background, so it reads the same wherever it is shown
as an image, light or dark theme.
"""
import math
import os

OUT = os.path.dirname(os.path.abspath(__file__))

# ---- Engine constants (invert.rs, render.rs, dng.rs) ----------------------------------------
GAMMA_REF = 0.7                                  # invert.rs
TOE_AMOUNT, TOE_SCALE, TOE_FLOOR = 0.3, 0.35, -0.3
GRAY_PRIOR_RED = (-0.2008, 0.9564)               # (bend, slope)
GRAY_PRIOR_BLUE = (0.1883, 0.8340)
STORE_SCALE = 0.2                                # dng.rs: stored value of frame white
STORED_REACH, ROLLOFF_KNEE = 4.0, 0.2            # render.rs
BLACKS_RANGE, BUMP_RANGE = 0.12, 0.9
CONTRAST_RANGE = 2.6
LOOKS = {"neutral": (1.5, 0.0), "flat": (0.4, -100.0), "hard": (2.2, 0.0)}  # (slope, whites)

# ---- Palette ---------------------------------------------------------------------------------
INK, MUTED, RULE = "#15191e", "#566170", "#d5dbe2"
ACCENT, ACCENT_SOFT = "#c0551a", "#f7e3d6"
RED, GREEN, BLUE = "#c2362f", "#2b8449", "#2d5fc0"

STYLE = f"""<style>
text {{ font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: 11px; fill: {MUTED}; }}
text.lbl {{ fill: {INK}; font-size: 11.5px; }}
text.ttl {{ font-family: -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; font-weight: 600; font-size: 13px; fill: {INK}; }}
text.sm {{ font-size: 10.5px; }}
text.on-dark {{ fill: #ffffff; }}
.grid {{ stroke: {RULE}; stroke-width: 1; fill: none; }}
.axis {{ stroke: {MUTED}; stroke-width: 1; fill: none; }}
.line {{ fill: none; stroke-width: 2.2; stroke-linejoin: round; stroke-linecap: round; }}
.ref {{ fill: none; stroke: {MUTED}; stroke-width: 1.2; stroke-dasharray: 4 4; }}
.box {{ fill: #ffffff; stroke: {MUTED}; stroke-width: 1.2; }}
.boxa {{ fill: {ACCENT_SOFT}; stroke: {ACCENT}; stroke-width: 1.4; }}
.arr {{ stroke: {MUTED}; stroke-width: 1.4; fill: none; }}
.c-ink {{ stroke: {INK}; }} .c-r {{ stroke: {RED}; }} .c-b {{ stroke: {BLUE}; }}
.c-a {{ stroke: {ACCENT}; }} .c-m {{ stroke: {MUTED}; }}
text.t-r {{ fill: {RED}; }} text.t-b {{ fill: {BLUE}; }} text.t-a {{ fill: {ACCENT}; }}
.dash {{ stroke-dasharray: 6 4; }}
</style>"""

ARROW = (f'<defs><marker id="ah" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
         f'markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="{MUTED}"/>'
         f'</marker></defs>')


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;")


def num(v):
    return f"{round(v, 2):g}"


def svg(w, h, label, body, pad=12):
    """A complete SVG document: white card with a margin, so it reads on any page background."""
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-pad} {-pad} {w + 2 * pad} '
            f'{h + 2 * pad}" width="{w + 2 * pad}" height="{h + 2 * pad}" role="img" '
            f'aria-label="{esc(label)}">{STYLE}'
            f'<rect x="{-pad}" y="{-pad}" width="{w + 2 * pad}" height="{h + 2 * pad}" '
            f'fill="#ffffff" rx="8"/>{body}</svg>\n')


def frange(a, b, n):
    return [a + (b - a) * i / n for i in range(n + 1)]


def chart(w, h, x, y, xt, yt, xl, yl, series, extra=None, xf=str, yf=str, title=None,
          m=(52, 18, 28, 46), legend=None):
    """Line chart in data units. m = left, right, top, bottom margins."""
    ml, mr, mt, mb = m
    x0, x1 = x
    y0, y1 = y

    def sx(v):
        return ml + (v - x0) / (x1 - x0) * (w - ml - mr)

    def sy(v):
        return h - mb - (v - y0) / (y1 - y0) * (h - mt - mb)

    s = []
    if title:
        s.append(f'<text class="ttl" x="{ml}" y="16">{esc(title)}</text>')
    for t in xt:
        s.append(f'<line class="grid" x1="{num(sx(t))}" x2="{num(sx(t))}" y1="{num(sy(y0))}" y2="{num(sy(y1))}"/>')
        s.append(f'<text x="{num(sx(t))}" y="{num(h - mb + 16)}" text-anchor="middle">{esc(xf(t))}</text>')
    for t in yt:
        s.append(f'<line class="grid" x1="{num(sx(x0))}" x2="{num(sx(x1))}" y1="{num(sy(t))}" y2="{num(sy(t))}"/>')
        s.append(f'<text x="{ml - 8}" y="{num(sy(t) + 4)}" text-anchor="end">{esc(yf(t))}</text>')
    s.append(f'<line class="axis" x1="{num(sx(x0))}" x2="{num(sx(x1))}" y1="{num(sy(y0))}" y2="{num(sy(y0))}"/>')
    s.append(f'<line class="axis" x1="{num(sx(x0))}" x2="{num(sx(x0))}" y1="{num(sy(y0))}" y2="{num(sy(y1))}"/>')
    s.append(f'<text x="{num((ml + w - mr) / 2)}" y="{h - 6}" text-anchor="middle">{esc(xl)}</text>')
    s.append(f'<text transform="translate(13 {num((mt + h - mb) / 2)}) rotate(-90)" text-anchor="middle">{esc(yl)}</text>')
    for se in series:
        d = "".join(("L" if i else "M") + f"{num(sx(px))} {num(sy(py))}" for i, (px, py) in enumerate(se["pts"]))
        cls = "line " + se["cls"] + (" " + se["dash"] if se.get("dash") else "")
        s.append(f'<path class="{cls}" d="{d}"/>')
        if "label" in se:
            (lx, ly), text = se["label"][:2]
            anchor = se["label"][2] if len(se["label"]) > 2 else "start"
            s.append(f'<text class="lbl {se.get("tcls", "")}" x="{num(sx(lx))}" y="{num(sy(ly))}" '
                     f'text-anchor="{anchor}">{esc(text)}</text>')
    if extra:
        s.append(extra(sx, sy))
    for i, (color, text) in enumerate(legend or []):
        lx, ly = ml + 14, mt + 14 + i * 18
        s.append(f'<line x1="{lx}" x2="{lx + 22}" y1="{ly - 4}" y2="{ly - 4}" stroke="{color}" stroke-width="2.6"/>'
                 f'<text class="lbl" x="{lx + 30}" y="{ly}">{esc(text)}</text>')
    return "".join(s)


# ---- Figure 1: pipeline ----------------------------------------------------------------------
def fig_pipeline():
    bw, bh, gx, x0, y1, y2 = 168, 78, 26, 8, 14, 140
    row1 = [
        ("Decode", "Raw → linear RGB", "LibRaw, AHD, gamma 1", False),
        ("Measure", "Base, area, film type,", "gray curves, anchor", True),
        ("Density above base", "−log10(T / T_base)", "removes the mask", True),
        ("Gray curves", "R, B onto green density", "fitted per frame", True),
        ("Crosstalk", "Fixed density matrix", "neutrals unchanged", False),
    ]
    row2 = [
        ("Toe", "Shared, near the base", "0.3 · exp(−E / 0.35)", False),
        ("Back to linear", "Anchor, γ = 0.7, sliders", "in stops per channel", True),
        ("Saturation", "Around Rec.2020 luma", "gain 1.25 × (1 + s/100)", False),
        ("Geometry", "Mirror, quarter turns", "after inversion", False),
        ("Write DNG", "16-bit, headroom", "lossless JPEG tiles", False),
    ]
    w = x0 * 2 + 5 * bw + 4 * gx
    s = [ARROW]

    def row(items, y):
        for i, (title, a, b, measured) in enumerate(items):
            x = x0 + i * (bw + gx)
            s.append(f'<rect class="{"boxa" if measured else "box"}" x="{x}" y="{y}" width="{bw}" height="{bh}" rx="6"/>')
            s.append(f'<text class="ttl" x="{x + 12}" y="{y + 24}">{esc(title)}</text>')
            s.append(f'<text class="sm" x="{x + 12}" y="{y + 45}">{esc(a)}</text>')
            s.append(f'<text class="sm" x="{x + 12}" y="{y + 62}">{esc(b)}</text>')
            if i < len(items) - 1:
                s.append(f'<line class="arr" x1="{x + bw + 3}" y1="{y + bh / 2}" x2="{x + bw + gx - 3}" '
                         f'y2="{y + bh / 2}" marker-end="url(#ah)"/>')

    row(row1, y1)
    row(row2, y2)
    xe, xs = x0 + 4 * (bw + gx) + bw / 2, x0 + bw / 2
    s.append(f'<path class="arr" d="M{xe} {y1 + bh + 2}V{y1 + bh + 22}H{xs}V{y2 - 4}" marker-end="url(#ah)"/>')
    s.append(f'<text class="sm" x="{x0}" y="236">Straighten and crop run before “Density above base”.</text>')
    s.append(f'<rect class="boxa" x="{x0 + 520}" y="226" width="14" height="9" rx="2"/>'
             f'<text class="sm" x="{x0 + 540}" y="235">uses values measured on this frame</text>')
    return svg(w, 240, "Ten processing steps in two rows, from decode to writing the DNG", "".join(s))


# ---- Figure 2: what the analysis looks at ------------------------------------------------------
def fig_scan():
    s = [
        '<defs>',
        '<pattern id="hatch" width="7" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">'
        f'<rect width="7" height="7" fill="#1b1e22"/><line x1="0" y1="0" x2="0" y2="7" stroke="{RED}" stroke-width="2.2"/></pattern>',
        '<linearGradient id="pic" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#6c4a2f"/>'
        '<stop offset=".45" stop-color="#8a6240"/><stop offset=".7" stop-color="#3f3a36"/>'
        '<stop offset="1" stop-color="#26231f"/></linearGradient>',
        '</defs>',
        '<rect x="10" y="10" width="500" height="320" fill="url(#hatch)"/>',        # holder
        '<rect x="40" y="30" width="440" height="280" fill="#b5651e"/>',             # film base
        '<rect x="70" y="50" width="380" height="218" fill="url(#pic)"/>',          # picture
        '<circle cx="360" cy="110" r="22" fill="#2b2622"/>',
        f'<rect x="100" y="278" width="150" height="14" fill="none" stroke="{ACCENT_SOFT}" stroke-width="2.4"/>',
        f'<rect x="47.5" y="34" width="425" height="272" fill="none" stroke="{BLUE}" stroke-width="2.4" stroke-dasharray="7 5"/>',
        '<text class="lbl on-dark" x="258" y="289">film base patch</text>',
        '<text class="lbl on-dark" x="82" y="82">picture</text>',
        '<text class="lbl on-dark" x="18" y="24">holder</text>',
        '<text class="lbl on-dark" x="150" y="47">dashed: measured area (85%)</text>',
    ]
    # Legend
    for i, (color, text) in enumerate([
        (ACCENT_SOFT, "Film base: largest even patch of the brightest film"),
        (BLUE, "Measured area: centered 85% of each side"),
        (RED, "Holder: dense cells connected to the scan's edge, ignored"),
    ]):
        y = 352 + i * 18
        s.append(f'<rect x="10" y="{y - 9}" width="14" height="10" rx="2" fill="{color}" stroke="{MUTED}" stroke-width=".6"/>'
                 f'<text class="lbl" x="32" y="{y}">{esc(text)}</text>')
    return svg(520, 400, "A film scan with holder, film base patch and the centered measured area", "".join(s))


# ---- Figure 3: density above base ---------------------------------------------------------------
def fig_density():
    base, px = [0.62, 0.38, 0.17], [0.31, 0.15, 0.04]
    dens = [-math.log10(v / b) for v, b in zip(px, base)]
    cols, names = [RED, GREEN, BLUE], ["R", "G", "B"]
    s = [ARROW]

    def bars(x0, vals, ymax, label, y0, hh):
        s.append(f'<text class="ttl" x="{x0}" y="{y0 - 20}">{esc(label)}</text>')
        s.append(f'<line class="axis" x1="{x0}" x2="{x0 + 250}" y1="{y0 + hh}" y2="{y0 + hh}"/>')
        for g in (0, 1):
            bx = x0 + 10 + g * 125
            for i, v in enumerate(vals[g]):
                bhh = max(v / ymax * hh, 0.5)
                s.append(f'<rect x="{bx + i * 36}" y="{num(y0 + hh - bhh)}" width="28" height="{num(bhh)}" fill="{cols[i]}"/>')
                s.append(f'<text text-anchor="middle" x="{bx + i * 36 + 14}" y="{num(y0 + hh - bhh - 5)}">'
                         f'{"0" if v < 0.005 else f"{v:.2f}"}</text>')
                s.append(f'<text text-anchor="middle" x="{bx + i * 36 + 14}" y="{y0 + hh + 14}">{names[i]}</text>')
            s.append(f'<text class="lbl" text-anchor="middle" x="{bx + 52}" y="{y0 + hh + 34}">'
                     f'{"scene pixel" if g else "film base"}</text>')

    bars(10, [base, px], 0.7, "Linear signal T (camera RGB)", 50, 170)
    bars(330, [[0, 0, 0], dens], 0.7, "Density D = −log10(T / T_base)", 50, 170)
    s.append('<line class="arr" x1="278" y1="140" x2="318" y2="140" marker-end="url(#ah)"/>')
    s.append('<text class="sm" x="298" y="128" text-anchor="middle">÷ base, log</text>')
    return svg(600, 280, "Linear base and pixel signals on the left, density above base on the right", "".join(s))


# ---- Figure 4: gray curves ---------------------------------------------------------------------
def gray_density(bend, slope, top, g):
    """Red or blue density of a neutral at green density g, as GrayCurve::equivalent inverts it:
    the quadratic up to `top`, then a straight line whose slope is the curve's at about 0.95 · top,
    kept within 1/1.6 to 1/0.6."""
    if bend < 0:
        top = min(top, 0.9 * -slope / (2 * bend))
    if g <= top:
        return (bend * g + slope) * g
    rate = min(max(1 / max(bend * 1.9 * top + slope, 1e-3), 0.6), 1.6)
    return (bend * top + slope) * top + (g - top) / rate


def fig_gray():
    top = 1.0
    gs = frange(0, 1.6, 80)
    series = [
        {"pts": [(g, g) for g in gs], "cls": "c-m", "dash": "dash", "label": ((1.18, 1.1), "green (identity)")},
        {"pts": [(g, gray_density(*GRAY_PRIOR_RED, top, g)) for g in gs], "cls": "c-r", "tcls": "t-r",
         "label": ((1.5, 0.98), "red", "end")},
        {"pts": [(g, gray_density(*GRAY_PRIOR_BLUE, top, g)) for g in gs], "cls": "c-b", "tcls": "t-b",
         "label": ((1.36, 1.56), "blue", "end")},
    ]

    def extra(sx, sy):
        return (f'<line class="ref" x1="{num(sx(top))}" x2="{num(sx(top))}" y1="{num(sy(0))}" y2="{num(sy(1.6))}"/>'
                f'<text class="sm" x="{num(sx(top) + 6)}" y="{num(sy(0.12))}">top = 1.0 (illustrative)</text>'
                f'<text class="sm" x="{num(sx(top) + 6)}" y="{num(sy(0.04))}">straight line beyond</text>')

    body = chart(620, 380, (0, 1.6), (0, 1.6), [0, 0.4, 0.8, 1.2, 1.6], [0, 0.4, 0.8, 1.2, 1.6],
                 "Green density above base, g", "Red or blue density of a neutral", series, extra,
                 xf=lambda t: f"{t:.1f}", yf=lambda t: f"{t:.1f}")
    return svg(620, 380, "Red and blue density of a neutral against green density", body)


# ---- Figure 5: toe and exposure ----------------------------------------------------------------
def fig_toe():
    anchor, log2 = 1.0, math.log10(2)

    def stops(e, toe):
        t = TOE_AMOUNT * math.exp(-max(e, TOE_FLOOR) / TOE_SCALE) if toe else 0
        return (e - t - anchor) / GAMMA_REF / log2

    es = frange(-0.3, 1.8, 84)
    series = [
        {"pts": [(e, stops(e, False)) for e in es], "cls": "c-m", "dash": "dash", "label": ((-0.22, -4.3), "no toe")},
        {"pts": [(e, stops(e, True)) for e in es], "cls": "c-a", "tcls": "t-a", "label": ((0.45, -4.4), "with toe")},
    ]
    per_unit = 1 / (GAMMA_REF * log2)

    def extra(sx, sy):
        return (f'<line class="ref" x1="{num(sx(-0.3))}" x2="{num(sx(1.8))}" y1="{num(sy(0))}" y2="{num(sy(0))}"/>'
                f'<text class="sm" x="{num(sx(-0.28))}" y="{num(sy(0) - 6)}">frame white (L = 1)</text>'
                f'<text class="sm" x="{num(sx(1.2))}" y="{num(sy(3.2))}" text-anchor="end">'
                f'slope {per_unit:.2f} stops per density unit</text>')

    body = chart(620, 380, (-0.3, 1.8), (-10, 4), [0, 0.5, 1.0, 1.5], [-10, -8, -6, -4, -2, 0, 2, 4],
                 "Green-equivalent density E (above base)", "Scene exposure, stops from frame white",
                 series, extra, xf=lambda t: f"{t:.1f}", yf=lambda t: f"+{t}" if t > 0 else str(t))
    return svg(620, 380, "Scene exposure in stops against density, with and without the toe", body)


# ---- Figure 6: DNG layout ----------------------------------------------------------------------
def fig_dng():
    s = [ARROW,
         '<rect class="box" x="8" y="8" width="310" height="284" rx="8"/>',
         '<text class="ttl" x="22" y="32">IFD0  ·  thumbnail and DNG tags</text>']
    for i, t in enumerate(["256 px RGB thumbnail", "UniqueCameraModel: Reversal Positive",
                           "ColorMatrix1 / ForwardMatrix1 (D65)", "AsShotNeutral 1, 1, 1",
                           "BaselineExposure 0", "Main profile + 2 extra profiles", "XMP: file name, all settings"]):
        s.append(f'<rect x="22" y="{44 + i * 34}" width="282" height="26" rx="4" fill="none" stroke="{RULE}"/>')
        s.append(f'<text class="lbl" x="32" y="{61 + i * 34}">{esc(t)}</text>')
    s.append('<rect class="boxa" x="392" y="8" width="320" height="150" rx="8"/>')
    s.append('<text class="ttl" x="406" y="32">SubIFD 0  ·  the image</text>')
    for i, t in enumerate(["LinearRaw, 3 × 16-bit", "BlackLevel 0, WhiteLevel 65535",
                           "256 px tiles, lossless JPEG", "Frame white stored at 0.2"]):
        s.append(f'<text class="lbl" x="406" y="{58 + i * 22}">{esc(t)}</text>')
    s.append('<rect class="box" x="392" y="178" width="320" height="114" rx="8"/>')
    s.append('<text class="ttl" x="406" y="202">SubIFD 1  ·  preview</text>')
    for i, t in enumerate(["1024 px sRGB JPEG, no chroma subsampling", "Rendered through the main profile"]):
        s.append(f'<text class="lbl" x="406" y="{228 + i * 22}">{esc(t)}</text>')
    s.append('<path class="arr" d="M318 90H392" marker-end="url(#ah)"/><path class="arr" d="M318 230H392" marker-end="url(#ah)"/>')
    s.append('<text class="sm" x="324" y="82">SubIFDs</text>')
    return svg(720, 300, "DNG layout: IFD0 with thumbnail, tags, profiles and XMP; sub-IFDs for the image and the preview", "".join(s))


# ---- Figure 7: headroom ------------------------------------------------------------------------
def fig_headroom():
    w, h, left = 720, 190, 20
    width = w - 40
    white = round(STORE_SCALE * 65535)
    headroom = math.log2(1 / STORE_SCALE)

    def X(v):
        return left + v / 65535 * width

    s = [f'<rect x="{num(X(0))}" y="50" width="{num(X(white) - X(0))}" height="44" fill="{GREEN}" opacity=".85"/>',
         f'<rect x="{num(X(white))}" y="50" width="{num(X(65535) - X(white))}" height="44" fill="{ACCENT}" opacity=".55"/>',
         f'<text class="ttl" x="{left}" y="30">Stored 16-bit value</text>',
         f'<text class="lbl on-dark" x="{num((X(white) + left) / 2)}" y="77" text-anchor="middle">image</text>',
         f'<text class="lbl" x="{num((X(white) + X(65535)) / 2)}" y="77" text-anchor="middle">'
         f'headroom: {headroom:.2f} stops above frame white</text>']
    ticks = [(0, "L = 0"), (white, "L = 1 (frame white)"), (2 * white, "L = 2"), (4 * white, "L = 4"),
             (65535, "L = 5")]
    for i, (v, lab) in enumerate(ticks):
        anchor = "start" if i == 0 else "end" if i == len(ticks) - 1 else "middle"
        s.append(f'<line class="axis" x1="{num(X(v))}" x2="{num(X(v))}" y1="94" y2="106"/>')
        s.append(f'<text x="{num(X(v))}" y="122" text-anchor="{anchor}">{v}</text>')
        s.append(f'<text class="sm" x="{num(X(v))}" y="138" text-anchor="{anchor}">{lab}</text>')
    s.append(f'<text class="sm" x="{num(X(65535))}" y="152" text-anchor="end">WhiteLevel</text>')
    s.append(f'<text class="sm" x="{left}" y="176">stored = round( clamp(L × 0.2, 0, 1) × 65535 )   ·   '
             f'frame white keeps {white:,} levels</text>')
    return svg(w, h, "The 16-bit range: frame white at 13107, headroom up to 65535", "".join(s))


# ---- Figures 8 and 9: tone curves (render.rs::tone_curve) ---------------------------------------
def unit(v):
    return max(-100.0, min(100.0, v)) / 100


def rolloff(l):
    if l <= ROLLOFF_KNEE:
        return l
    r = 1 - ROLLOFF_KNEE
    return ROLLOFF_KNEE + r * (1 - math.exp(-(l - ROLLOFF_KNEE) / r))


def shoulder(l, amount):
    start = rolloff(l)
    if amount < 0:
        rolled = l * (1 + l / STORED_REACH ** 2) / (1 + l)   # extended Reinhard, white at full scale
        return start + -amount * (rolled - start)
    return start * (1 + amount * start)


def clamp01(v):
    return max(0.0, min(1.0, v))


def tone_encoded(look, x, contrast=0.0, whites=0.0, blacks=0.0, shadows=0.0, highlights=0.0):
    """Display value of stored value x, gamma-2.2 encoded. The profile's curve holds this value
    raised to 2.2; plotting it encoded keeps the shadows readable."""
    if x >= 1:
        return 1.0
    if x <= 0:
        return 0.0
    slope0, whites0 = LOOKS[look]
    amount = unit(whites0 + whites)
    slope = slope0 * CONTRAST_RANGE ** unit(contrast)
    norm = math.tanh(slope * 0.5)
    black = -unit(blacks) * BLACKS_RANGE
    sh, hi = unit(shadows) * BUMP_RANGE, unit(highlights) * BUMP_RANGE
    p = clamp01(shoulder(x * STORED_REACH, amount)) ** (1 / 2.2)
    p = clamp01((p - black) / (1 - black))
    q = clamp01(0.5 * (1 + math.tanh(slope * (p - 0.5)) / norm))
    q = q + sh * q * (1 - q) * (1 - q) + hi * q * q * (1 - q)
    return clamp01(q)


SMAX, SMIN = math.log2(1 / STORE_SCALE), -9


def tone_series(look, **tone):
    pts = []
    for s in frange(SMIN, SMAX, 260):
        x = 1.0 if s >= SMAX - 1e-9 else STORE_SCALE * 2 ** s
        pts.append((s, tone_encoded(look, x, **tone)))
    return pts


def tone_chart(series, w, h, label, m=(52, 18, 28, 46), legend=None):
    def extra(sx, sy):
        return (f'<line class="ref" x1="{num(sx(0))}" x2="{num(sx(0))}" y1="{num(sy(0))}" y2="{num(sy(1))}"/>'
                f'<text class="sm" x="{num(sx(0) - 5)}" y="{num(sy(0) - 8)}" text-anchor="end">frame white</text>'
                f'<text class="sm" x="{num(sx(SMAX) - 4)}" y="{num(sy(0) - 8)}" text-anchor="end">full scale</text>')

    body = chart(w, h, (SMIN, SMAX), (0, 1), [-8, -6, -4, -2, 0, 2], [0, 0.25, 0.5, 0.75, 1],
                 "Stored value, stops from frame white", "Display value (gamma 2.2)", series, extra,
                 xf=lambda t: f"+{t}" if t > 0 else str(t), yf=lambda t: f"{t:.2f}", m=m, legend=legend)
    return svg(w, h, label, body)


def tone_figure(variants, label, size):
    """variants: (stroke class, legend color, legend text, look, tone sliders)."""
    series = [{"pts": tone_series(look, **tone), "cls": cls} for cls, _, _, look, tone in variants]
    legend = [(color, text) for _, color, text, _, _ in variants]
    return tone_chart(series, label=label, legend=legend, **size)


LARGE = dict(w=620, h=380)
SMALL = dict(w=420, h=320, m=(50, 14, 14, 44))


def fig_looks():
    return tone_figure([
        ("c-b", BLUE, "Flat", "flat", {}),
        ("c-ink", INK, "Neutral", "neutral", {}),
        ("c-a", ACCENT, "Hard", "hard", {}),
    ], "Tone curves of the three looks", LARGE)


def fig_contrast():
    return tone_figure([
        ("c-b", BLUE, "Contrast −100", "neutral", {"contrast": -100}),
        ("c-ink", INK, "0", "neutral", {}),
        ("c-a", ACCENT, "+100", "neutral", {"contrast": 100}),
    ], "Neutral curve at Contrast −100, 0 and +100", SMALL)


def fig_whites():
    return tone_figure([
        ("c-b", BLUE, "Whites −100", "neutral", {"whites": -100}),
        ("c-ink", INK, "0", "neutral", {}),
        ("c-a", ACCENT, "+100", "neutral", {"whites": 100}),
    ], "Neutral curve at Whites −100, 0 and +100", SMALL)


FIGURES = {
    "01-pipeline.svg": fig_pipeline,
    "02-scan-areas.svg": fig_scan,
    "03-density-above-base.svg": fig_density,
    "04-gray-curves.svg": fig_gray,
    "05-toe-exposure.svg": fig_toe,
    "06-dng-layout.svg": fig_dng,
    "07-headroom.svg": fig_headroom,
    "08-looks.svg": fig_looks,
    "09a-contrast.svg": fig_contrast,
    "09b-whites.svg": fig_whites,
}

if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    for name, make in FIGURES.items():
        with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
            f.write(make())
        print("wrote", name)
