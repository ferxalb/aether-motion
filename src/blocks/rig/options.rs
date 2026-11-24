//! Rig parameters: the table of names and defaults each rig lists in the block catalog, and their
//! validation into typed options shared by the 2D and 3D emitters.

use super::motion::{check_channel, preset, Motion, Sequence, Source, Timeline, View};
use super::skeleton::{Pose, CHANNELS};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;

/// Which rig is being generated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flavor {
    Flat,
    Solid,
}

/// Parameters before the per-channel pose offsets, as (name, default).
const COMMON: &[(&str, &str)] = &[
    ("motion", "idle"),
    ("cycle", "auto"),
    ("phase", "0"),
    ("delay", "0"),
    ("duration", "6"),
    ("sequence", ""),
    ("timeline", ""),
    ("loop", "false"),
    ("travel", "0"),
    ("ground", "lock"),
    ("size", "600"),
    ("skin", "#e2b08f"),
    ("hair", "#2d1f18"),
    ("shirt", "#4a78ff"),
    ("pants", "#283247"),
    ("shoes", "#16181f"),
    ("sleeves", "short"),
];

const FLAT: &[(&str, &str)] = &[
    ("id", "human"),
    ("view", "side"),
    ("facing", "right"),
    ("x", "960"),
    ("y", "900"),
    ("shadow", "on"),
];

const SOLID: &[(&str, &str)] = &[
    ("id", "human3d"),
    ("scene", "full"),
    ("x", "0"),
    ("z", "0"),
    ("turn", "0"),
    ("start", "0"),
    ("track", "1"),
    ("left", "0"),
    ("top", "0"),
    ("width", "1920"),
    ("height", "1080"),
    ("antialias", "3"),
    ("camera-angle", "25"),
    ("camera-height", "330"),
    ("distance", "1500"),
    ("fov", "35"),
    ("orbit", "0"),
    ("floor", "#262d3d"),
    ("grid", "#343d52"),
];

/// Every parameter of the rig as (name, default), in listing order: the rig's own, then the
/// shared ones, then one static offset per pose channel (default 0).
pub fn params(flavor: Flavor) -> Vec<(String, String)> {
    let own = match flavor {
        Flavor::Flat => FLAT,
        Flavor::Solid => SOLID,
    };
    own.iter()
        .chain(COMMON)
        .map(|(n, d)| (n.to_string(), d.to_string()))
        .chain(CHANNELS.iter().map(|c| (c.to_string(), "0".to_string())))
        .collect()
}

/// Sleeve length: what color the upper arm and forearm wear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sleeves {
    None,
    Short,
    Long,
}

/// Colors of the figure (CSS colors).
pub struct Palette {
    pub skin: String,
    pub hair: String,
    pub shirt: String,
    pub pants: String,
    pub shoes: String,
    pub sleeves: Sleeves,
}

impl Palette {
    pub fn upper_arm(&self) -> &str {
        if self.sleeves == Sleeves::None {
            &self.skin
        } else {
            &self.shirt
        }
    }

    pub fn forearm(&self) -> &str {
        if self.sleeves == Sleeves::Long {
            &self.shirt
        } else {
            &self.skin
        }
    }
}

/// How far the figure moves along its facing direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Travel {
    /// px (scene units in 3D) per second.
    Speed(f64),
    /// Matched to the stride of the motion so the planted foot does not slide.
    Auto,
}

/// Validated parameters.
pub struct Options {
    pub id: String,
    pub view: View,
    pub source: Source,
    pub delay: f64,
    pub duration: f64,
    pub travel: Travel,
    pub ground_lock: bool,
    /// Figure height in px (scene units in 3D).
    pub size: f64,
    pub palette: Palette,
    /// Static offsets added to every pose.
    pub offsets: Pose,
    raw: BTreeMap<String, String>,
}

impl Options {
    /// Reads `values` over the defaults of `flavor`; `view` picks the preset variant.
    pub fn parse(
        flavor: Flavor,
        values: &BTreeMap<String, String>,
        view_of: impl Fn(&BTreeMap<String, String>) -> Result<View>,
    ) -> Result<Self> {
        let table = params(flavor);
        for key in values.keys() {
            if !table.iter().any(|(n, _)| n == key) {
                let names: Vec<&str> = table.iter().map(|(n, _)| n.as_str()).collect();
                bail!("unknown parameter '{key}'; available: {}", names.join(", "));
            }
        }
        let mut raw: BTreeMap<String, String> = table.into_iter().collect();
        for (k, v) in values {
            raw.insert(k.clone(), v.trim().to_string());
        }
        let view = view_of(&raw)?;
        let get = |k: &str| raw[k].clone();

        let id = get("id");
        let valid_id = id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && id.len() <= 64;
        if !valid_id {
            bail!("id must start with a letter and use only letters, digits, '-' and '_' (at most 64): {id}");
        }

        let number = |k: &str, lo: f64, hi: f64| -> Result<f64> {
            let text = &raw[k];
            let v: f64 = text
                .parse()
                .map_err(|_| anyhow!("{k} must be a number, got '{text}'"))?;
            if !v.is_finite() || v < lo || v > hi {
                bail!("{k} must be between {lo} and {hi}, got {text}");
            }
            Ok(v)
        };

        let motion = Motion::parse(&get("motion"))?;
        let delay = number("delay", 0.0, 3600.0)?;
        let duration = number("duration", 0.1, 3600.0)?;
        let phase = number("phase", 0.0, 1.0)?;
        let looped = match get("loop").as_str() {
            "true" | "on" | "yes" | "1" => true,
            "false" | "off" | "no" | "0" => false,
            other => bail!("loop must be true or false, got '{other}'"),
        };
        let timeline = get("timeline");
        let sequence = get("sequence");
        let explicit_motion = values.contains_key("motion") && motion != Motion::None;
        let source = if !timeline.is_empty() {
            if explicit_motion || !sequence.is_empty() {
                bail!("timeline replaces motion and sequence; pass only one of them");
            }
            Source::Keys {
                timeline: Timeline::parse(&timeline)?,
                looped,
            }
        } else if !sequence.is_empty() {
            if explicit_motion {
                bail!("sequence replaces motion; leave motion unset when you pass a sequence");
            }
            Source::Chain {
                sequence: Sequence::parse(&sequence, view)?,
                looped,
            }
        } else {
            match preset(motion, view) {
                None => Source::Still,
                Some(p) => {
                    let cycle = match get("cycle").as_str() {
                        "auto" => p.cycle,
                        _ => number("cycle", 0.1, 60.0)?,
                    };
                    Source::Cycle {
                        preset: p,
                        cycle,
                        phase,
                    }
                }
            }
        };

        let travel = match get("travel").as_str() {
            "auto" => {
                if matches!(source, Source::Still)
                    || (matches!(source, Source::Cycle { .. }) && !motion.travels())
                {
                    bail!("travel=auto follows the feet of motion=walk or motion=run, a sequence, or a timeline; give a speed in px per second for other motions");
                }
                if view == View::Front {
                    bail!("travel=auto needs view=side: a front view cannot walk across the frame");
                }
                Travel::Auto
            }
            _ => Travel::Speed(number("travel", -100_000.0, 100_000.0)?),
        };
        let ground_lock = match get("ground").as_str() {
            "lock" => true,
            "free" => false,
            other => bail!("ground must be lock or free, got '{other}'"),
        };
        let size = number("size", 20.0, 20_000.0)?;

        let color = |k: &str| -> Result<String> {
            let c = get(k);
            if c.is_empty() || c.len() > 64 || c.contains(['<', '>', '"', '\'', ';', '{', '}']) {
                bail!("{k} must be a CSS color such as #e2b08f or rgb(226, 176, 143), got '{c}'");
            }
            if flavor == Flavor::Flat && c.contains("var(") {
                bail!("{k}: the 2D rig draws SVG, which cannot read var(); give a color value");
            }
            Ok(c)
        };
        let palette = Palette {
            skin: color("skin")?,
            hair: color("hair")?,
            shirt: color("shirt")?,
            pants: color("pants")?,
            shoes: color("shoes")?,
            sleeves: match get("sleeves").as_str() {
                "none" => Sleeves::None,
                "short" => Sleeves::Short,
                "long" => Sleeves::Long,
                other => bail!("sleeves must be none, short, or long, got '{other}'"),
            },
        };

        let mut offsets = [0.0; CHANNELS.len()];
        for (ch, name) in CHANNELS.iter().enumerate() {
            let text = &raw[*name];
            let v: f64 = text
                .parse()
                .map_err(|_| anyhow!("{name} must be a number, got '{text}'"))?;
            check_channel(ch, v)?;
            offsets[ch] = v;
        }

        Ok(Self {
            id,
            view,
            source,
            delay,
            duration,
            travel,
            ground_lock,
            size,
            palette,
            offsets,
            raw,
        })
    }

    /// What drives the figure, for the block's leading comment.
    pub fn describe_motion(&self) -> String {
        match self.source {
            Source::Keys { .. } => "a timeline".to_string(),
            Source::Chain { .. } => "a sequence".to_string(),
            _ => format!("motion {}", self.text("motion")),
        }
    }

    /// A parameter's text after defaults.
    pub fn text(&self, key: &str) -> &str {
        &self.raw[key]
    }

    /// A numeric parameter in `lo..=hi`.
    pub fn number(&self, key: &str, lo: f64, hi: f64) -> Result<f64> {
        let text = self.text(key);
        let v: f64 = text
            .parse()
            .with_context(|| format!("{key} must be a number, got '{text}'"))?;
        if !v.is_finite() || v < lo || v > hi {
            bail!("{key} must be between {lo} and {hi}, got {text}");
        }
        Ok(v)
    }

    /// The pose at `t` seconds after the motion starts, with the static offsets added (before
    /// ground placement).
    pub fn pose(&self, t: f64) -> Pose {
        let mut pose = self.source.pose(t.max(0.0));
        for (value, offset) in pose.iter_mut().zip(self.offsets) {
            *value += offset;
        }
        pose
    }

    /// Scale from rig units to output px.
    pub fn scale(&self) -> f64 {
        self.size / super::skeleton::STATURE
    }
}

/// The rest pose, for tests.
#[cfg(test)]
pub fn rest_pose() -> Pose {
    super::skeleton::rest()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> Result<Options> {
        let values = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Options::parse(Flavor::Flat, &values, |_| Ok(View::Side))
    }

    #[test]
    fn validates_parameters() {
        assert!(opts(&[]).is_ok());
        assert!(opts(&[("motion", "fly")]).is_err());
        assert!(opts(&[("nope", "1")]).is_err());
        assert!(opts(&[("id", "9x")]).is_err());
        assert!(opts(&[("skin", "red\"><script>")]).is_err());
        assert!(opts(&[("skin", "var(--a)")]).is_err());
        assert!(opts(&[("r-elbow", "abc")]).is_err());
        assert!(opts(&[("travel", "auto")]).is_err());
        assert!(opts(&[("motion", "walk"), ("travel", "auto")]).is_ok());
        assert!(opts(&[("motion", "walk"), ("timeline", "[{\"at\":0}]")]).is_err());
        assert!(opts(&[
            ("motion", "walk"),
            ("sequence", "[{\"motion\":\"walk\",\"duration\":1}]")
        ])
        .is_err());
        assert!(opts(&[
            ("travel", "auto"),
            (
                "sequence",
                "[{\"motion\":\"walk\",\"duration\":2.2},{\"motion\":\"wave\",\"duration\":2}]"
            )
        ])
        .is_ok());
        assert!(opts(&[("timeline", "[{\"at\":0},{\"at\":1,\"r-elbow\":40}]")]).is_ok());
        let o = opts(&[("motion", "none"), ("r-elbow", "30")]).unwrap();
        assert_eq!(
            o.pose(0.0)[super::super::skeleton::RIGHT.elbow],
            rest_pose()[11] + 30.0
        );
    }
}
