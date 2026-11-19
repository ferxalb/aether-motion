"""Draw the original coat, head and twelve-pose walk cycle. Run on macOS with sips."""
from math import cos, hypot, pi, sin, sqrt
from pathlib import Path
import re
from subprocess import run
from tempfile import TemporaryDirectory

DEST = Path(__file__).resolve().parent
STEPS = 12
# A stance leg travels rearward five artwork units per cel. The shot's stepped translation
# adds five artwork units per cel in the opposite direction at its chosen sprite size.
STANCE_TRAVEL = 30


def path(d, fill, edge="none", width=1):
    return (f'<path d="{d}" fill="{fill}" stroke="{edge}" stroke-width="{width}" '
            'stroke-linejoin="round" stroke-linecap="round"/>')


def point(x, y):
    return f"{x:.3f},{y:.3f}"


def footprint(phase, far):
    """Return an ankle and sole pivot. The stance pivot stays fixed in street space."""
    phase %= 1
    floor = 252 - (2 if far else 0)
    if phase < .5:
        progress = 2 * phase
        x = 113 - STANCE_TRAVEL * progress
        # Sink from heel strike into a flat sole, then roll about the toe.
        if progress < .26:
            tilt, pivot = -10 * (1 - progress / .26)**2, -7
        elif progress > .64:
            tilt, pivot = 16 * ((progress - .64) / .36)**2, 21
        else:
            tilt, pivot = 0, 4
    else:
        progress = 2 * (phase - .5)
        x = 83 + STANCE_TRAVEL * (.5 - .5 * cos(pi * progress))
        floor -= 16 * sin(pi * progress)**1.45
        tilt = 16 * (1-progress)**2 - 10 * progress**2
        pivot = 21 - 28 * progress
    x -= 5 if far else 0
    angle = tilt * pi / 180
    # Correct the ankle position after rotation so the sole pivots at the intended contact.
    ankle_x = x + pivot - (pivot * cos(angle) - 11 * sin(angle))
    ankle_y = floor - (pivot * sin(angle) + 11 * cos(angle))
    return ankle_x, ankle_y, tilt, x + pivot, floor


def contact(index):
    art = []
    for shift, far in [(0, False), (.5, True)]:
        phase = (index / STEPS + shift) % 1
        if phase < .5:
            _, _, _, x, floor = footprint(phase, far)
            art.append(f'<ellipse cx="{x:.2f}" cy="{floor+1:.2f}" rx="16" ry="4.3" fill="url(#shoe-shadow)"/>')
    return "".join(art)


def leg(phase, bob, far):
    x, y, tilt, _, _ = footprint(phase, far)
    hip_x, hip_y = (94 if far else 89), 150 + bob
    dx, dy = x - hip_x, y - hip_y
    distance = hypot(dx, dy)
    bend = sqrt(max(2, 48**2 - distance**2 / 4))
    kx = (x + hip_x) / 2 + bend * dy / distance
    ky = (y + hip_y) / 2 - bend * dx / distance
    pants = "#262626" if far else "url(#trouser)"
    rim = "#3a3a3a" if far else "#626262"
    return [
        path(f"M{point(hip_x-10,hip_y)} Q{point(kx-13,ky-6)} {point(kx-8,ky+3)} "
             f"L{point(x-7,y+2)} {point(x+5,y+3)} Q{point(kx+3,ky+9)} {point(kx+8,ky+1)} "
             f"Q{point(kx+8,ky-14)} {point(hip_x+8,hip_y-4)}Z", pants, rim, .8),
        path(f"M{point(kx+4,ky-5)} Q{point(kx+2,ky+8)} {point(x+1,y-5)}",
             "none", "#777777" if not far else "#424242", 1),
        f'<g transform="translate({point(x,y)}) rotate({tilt:.2f})">',
        path("M-7-4 L6-4 Q8 1 13 4 L21 5 Q25 7 23 11 L-7 11 Q-10 9-8 5Z",
             "#141414", rim, .8),
        path("M-6 9H21M2-2l6 3", "none", "#828282" if not far else "#4f4f4f", .7),
        '</g>',
    ]


def figure(index):
    phase = index / STEPS
    bob = -3.1 * sin(phase * 4 * pi - .55)
    swing = cos((phase + .05) * 2 * pi)
    coat = 4.5 * sin(2 * pi * phase - .9)
    body = leg(phase + .5, bob, True) + leg(phase, bob, False)
    body.append(f'<g transform="translate(0,{bob:.3f}) rotate({1.2+1.65*swing:.2f} 92 153)">')
    body.extend([
        f'<g transform="rotate({-14*swing:.2f} 108 91)">',
        path("M96 78Q112 76 119 95L123 129 120 147 113 145 115 125 106 105Z", "#282828", "#3c3c3c", .7),
        '</g>',
        path(f"M82 64 C72 71 65 84 65 106 L61 150 Q56 177 {46+coat:.2f} 200 "
             f"Q49 208 67 204 L88 208 109 202 120 203 Q117 176 111 156 "
             "L122 106 Q117 81 105 75Z", "url(#coat)", "#525252", .75),
        path(f"M78 84Q68 121 67 158 Q62 183 {51+coat:.2f} 198 L68 200 80 181 89 106Z",
             "#252525"),
        path("M84 101Q78 124 79 153L75 193M96 113Q98 148 92 173L96 201M109 159L115 195",
             "none", "#777777", .85),
        path("M72 122L81 125 80 130 71 128ZM96 133l12-2-1 5-11 2Z",
             "#2c2c2c", "#727272", .6),
        path("M88 55L111 50 109 70 116 77 103 92 83 74Z", "#555555"),
        path(f"M109 73Q90 66 80 71 L56 75 {29+coat:.2f} {76-coat:.2f} 53 83 79 82 99 92 116 82Z",
             "#303030", "#707070", 1),
        path("M81 45L88 28 109 27 119 37 119 49 125 56 116 59 114 69 104 72 86 62Z",
             "url(#face)", "#363636", .8),
        path("M79 52L78 40 82 33 79 29 90 26 89 20 103 23 110 18 116 25 127 31 "
             "121 33 125 41 116 38 113 45 108 39 101 47 95 46 96 57 88 57 86 65Z",
             "#212121", "#686868", .85),
        path("M84 58L93 54M112 48H117M112 62L117 60M91 31L111 25M103 32L118 29",
             "none", "#8c8c8c", .85),
        path("M79 78Q68 88 65 106M87 84L101 96M97 102L90 124M109 177L116 199",
             "none", "#767676", .85),
        f'<g transform="rotate({15*swing:.2f} 100 93)">',
        path("M97 90Q109 88 113 101 L115 124 112 152 102 153 99 130 93 109Z",
             "url(#sleeve)", "#686868", .9),
        path("M103 150L112 149 112 159 108 167 103 161Z", "#737373", "#292929", .8),
        path("M107 95Q112 107 111 127L108 146M98 113L101 129", "none", "#858585", .8),
        '</g></g>',
    ])
    return ''.join(body)


GRADIENTS = """<linearGradient id="coat" x2="1" y2="1">
<stop stop-color="#767676"/><stop offset=".58" stop-color="#424242"/><stop offset="1" stop-color="#292929"/>
</linearGradient><linearGradient id="face" x2="1" y2=".3">
<stop stop-color="#696969"/><stop offset="1" stop-color="#898989"/>
</linearGradient><linearGradient id="trouser" x2="1" y2="1">
<stop stop-color="#414141"/><stop offset="1" stop-color="#252525"/>
</linearGradient><linearGradient id="sleeve" x2="1" y2="1">
<stop stop-color="#606060"/><stop offset="1" stop-color="#303030"/>
</linearGradient>
<radialGradient id="shoe-shadow"><stop stop-color="#050505" stop-opacity=".75"/>
<stop offset=".54" stop-color="#050505" stop-opacity=".42"/><stop offset="1" stop-color="#050505" stop-opacity="0"/>
</radialGradient>"""


def shadow_value(svg):
    def darken(match):
        value = round(int(match[0][1:3], 16) * .54)
        return "#" + f"{value:02x}" * 3

    return re.sub(r"#[0-9a-f]{6}", darken, svg)


def render():
    with TemporaryDirectory() as scratch:
        source = Path(scratch) / "cel.svg"
        for i in range(STEPS):
            shape = figure(i)
            for mode in ("lit", "shadow", "reflection", "contact"):
                if mode == "reflection":
                    # Compress and invert the reflection above its foot-line. The layer in HTML
                    # supplies the stationary pool mask and mixes the reflection into the asphalt.
                    dims = 'width="350" height="154" viewBox="0 0 175 77"'
                    art = f'<g transform="translate(-5 80.64) skewX(6) scale(1 -.32)">{shape}</g>'
                elif mode == "contact":
                    dims = 'width="350" height="58" viewBox="0 235 175 29"'
                    art = contact(i)
                else:
                    dims = 'width="350" height="528" viewBox="0 0 175 264"'
                    art = shape
                svg = f'<svg xmlns="http://www.w3.org/2000/svg" {dims}><defs>{GRADIENTS}</defs>{art}</svg>'
                source.write_text(shadow_value(svg) if mode == "shadow" else svg)
                dest = DEST / mode / f"cel-{i+1:02}.png"
                dest.parent.mkdir(exist_ok=True)
                run(["sips", "-s", "format", "png", str(source), "--out", str(dest)],
                    check=True, capture_output=True)


if __name__ == "__main__":
    render()
