# One Night

A 15-second illustrated coastal short at 1920×1080, 24 fps. A lone walker climbs a clifftop path, stops short of the edge, and watches moonlight on the water. The camera never cuts: one continuous take follows him close, settles as he stops, and pulls back to the wide clifftop; it then dissolves into the reverse view from the sea while both cameras are still pulling back, so the motion reads as one move until the walker is a small figure on a tall sea cliff.

- `index.html` sequences two external scenes with a 1.4 s `crossfade` and plays the continuous original coastal ambience, raised 12.2 dB (`data-volume="4.07"`) to about −26 LUFS integrated with its own fade-out intact.
- `scenes/clifftop.html` (11 s) is a single `data-hf-shots` rig: a `follow` shot at 1.9x zoom tracks the walker from behind while he walks for 5.6 s at a stride-matched pace (about 38 px/s, shrinking from 106% to 100% as he recedes) and decelerates to a stop; the framing anchor drifts from lower left toward the centre, so his advance reads clearly, keeps framing him as the camera settles, then a `reveal` eases back to the authored wide framing (zoom 1) by the end of the clip.
- `scenes/night.html` (5.4 s) looks back at the headland from the sea and pulls back from the walker to the whole cliff, then fades to black. Its first frames place the walker and the moon where shot 1 leaves them mid-dissolve (a match dissolve), so neither doubles.
- `assets/` contains original SVG depth planes (sky, sea, the clifftop path, receding headlands, out-of-focus foreground rocks, and for the last shot the bluff (irregular dipping strata, an undercut lip, buttresses, a chimney, rockfall scars, a sea cave, a stack, and talus), its surf, a far point, and wet foreground rocks), plus the synthesized surf, wind, and footfalls.

`scenes/walker/` holds eight full-body drawn cels on a shared registration, plus a matching standing pose. The walk cycle runs at eight drawings per second. Each drawing changes the boots, knees, and coat together, and the standing cel takes over after the final stride. Moonlit haze sits on a separate ocean plane behind the foreground. The walk cycle runs for 45 cels (5.625 s) so the standing cel continues its stance, and the standing cel lasts to the end of the clip.

From the repository root:

```sh
target/release/aether-motion check examples/one-night/index.html --json
target/release/aether-motion render examples/one-night/index.html \
  --output out/one-night-drawn.mp4 --size 1080p --fps 24 --threads 2 --quality high
```
