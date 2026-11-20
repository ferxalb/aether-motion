# One Night

A 15-second illustrated coastal short at 1920×1080, 24 fps. A lone walker approaches a cliff, stops short of its edge, and watches moonlight on the water. The film cuts from a close follow shot to a wider hold, then reverses to a low view from the water: the walker becomes a small figure on a tall sea cliff while the camera pulls back to reveal its scale.

- `index.html` sequences three external scenes and plays the continuous original coastal ambience, raised 12.2 dB (`data-volume="4.07"`) to about −26 LUFS integrated with its own fade-out intact.
- `scenes/walk.html` follows the walker from behind. The camera's framing anchor drifts as it follows, so he climbs and moves right in frame while the path slides under him.
- `scenes/horizon.html` follows his last strides and holds the view as he stops. It opens with his coat at the screen position shot 1 left it (an axial cut), then tilts up to the sea.
- `scenes/night.html` looks back at the headland from the sea, opening on the walker at the screen position he held at the end of shot 2, pulls back from him to the whole cliff, and fades to black.
- `assets/` contains original SVG depth planes (sky, sea, the clifftop path, receding headlands, out-of-focus foreground rocks, and for the last shot the bluff, its surf, a far point, and wet foreground rocks), the layered coat and 16 gait poses, plus the synthesized surf, wind, and footfalls.

During the stance phase, the foot shifts against the direction of travel by exactly the walker's own advance, so it stays planted. The shoulders sit lowest in double support and rise and shift over the planted foot at mid-stance; on the last step the coat drops onto both feet and settles. During the swing phase, it lifts and returns forward while the coat and shoulders transfer weight. The plates stay at a fixed scale within each shot so changes in camera framing can reuse their rasterizations.

From the repository root:

```sh
target/release/aether-motion check examples/one-night/index.html --json
target/release/aether-motion render examples/one-night/index.html \
  --output out/one-night.mp4 --size 1080p --fps 24 --threads 2 --quality high
```
