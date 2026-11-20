# One Night

A 15-second illustrated coastal short. The camera follows a walker up a clifftop path, settles as he stops, then pulls back. A match dissolve carries the pullback into a reverse view from the sea.

- [Main composition](index.html) joins the two scenes with a 1.4-second crossfade over continuous coastal ambience.
- [Clifftop](scenes/clifftop.html) uses `data-hf-shots` to follow the walker and reveal the coast. The [walker cels](scenes/walker/) run at eight drawings per second and hand off to a matching standing pose.
- [Reverse view](scenes/night.html) pulls back to reveal the cliff, then fades to black.

From the repository root:

```sh
aether-motion check examples/one-night/index.html
aether-motion render examples/one-night/index.html \
  --output out/one-night.mp4 --size 1080p --fps 24 --quality high
```
