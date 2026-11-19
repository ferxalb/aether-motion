//! `data-hf-shots`: the minimal shot composer. A JSON list of shots on a camera container, played
//! back to back from the clip start, compiled into the keys of one [`crate::camera::CameraRig`].
//!
//! Every preset is a few rig keys on the same camera, so consecutive shots share one continuous
//! camera: with the default `smooth` easing, velocity is continuous across shot boundaries.
//! Unrelated scenes are separate clips joined with `data-hf-transition`; shots never interpolate
//! between them.
//!
//! Presets: `static` (holds; a following camera comes to rest within `blend`), `follow` (frames
//! the subject; acquired over `blend` when the previous shot did not follow), `pan` (moves the
//! focus to `to`), `dolly-in` and `dolly-out` (zoom to `zoom`, default x1.25 or /1.25), and
//! `reveal` (pulls back to `zoom`, default /1.35, and when following lowers the subject's anchor
//! by 0.08 to open up the space above). Dollies and reveals keep the previous shot's follow; a
//! `to` releases the subject (the camera eases from framing it to that focus). Fields: `preset`, `duration` (required except on the last
//! shot, which fills the clip), `zoom`, `to` (`[x, y]` focus), `anchor` (`[x, y]` fractions at the
//! end of the shot), `follow` (`true`/`false`), `easing` (`smooth` or a keyframe easing), `blend`
//! (seconds, default 0.75), and, on the first shot only, `from` (`{zoom, x, y, anchor}`).
use crate::camera::curve::Interp;
use crate::camera::{FocusValue, RigKey, MAX_ZOOM, MIN_ZOOM};
use crate::parse::animation::camera::parse_camera_easing;
use crate::parse::animation::rig::RigOptions;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};

const MAX_SHOTS: usize = 64;
const DEFAULT_BLEND: f64 = 0.75;
const DOLLY: f64 = 1.25;
const REVEAL: f64 = 1.35;
const REVEAL_ANCHOR_DROP: f64 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) enum Preset {
    Static,
    Follow,
    Pan,
    DollyIn,
    DollyOut,
    Reveal,
}

impl Preset {
    const NAMES: &'static str = "static, follow, pan, dolly-in, dolly-out, reveal";

    fn parse(raw: &str) -> Result<Self> {
        Ok(match raw {
            "static" => Self::Static,
            "follow" => Self::Follow,
            "pan" => Self::Pan,
            "dolly-in" => Self::DollyIn,
            "dolly-out" => Self::DollyOut,
            "reveal" => Self::Reveal,
            other => bail!("unknown preset {other} (use {})", Self::NAMES),
        })
    }
}

/// Opening pose of the first shot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::parse) struct Opening {
    zoom: Option<f64>,
    focus: [Option<f64>; 2],
    anchor: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::parse) struct ShotSpec {
    preset: Preset,
    duration: Option<f64>,
    zoom: Option<f64>,
    to: Option<[f64; 2]>,
    anchor: Option<[f64; 2]>,
    follow: Option<bool>,
    easing: Option<Interp>,
    blend: Option<f64>,
    from: Option<Opening>,
}

fn reject_unknown(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("unknown field: {key} (use {})", allowed.join(", "));
        }
    }
    Ok(())
}

fn finite(value: &Value, name: &str) -> Result<f64> {
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| anyhow!("{name} must be a finite number"))
}

fn zoom(value: &Value) -> Result<f64> {
    let zoom = finite(value, "zoom")?;
    if !(MIN_ZOOM..=MAX_ZOOM).contains(&zoom) {
        bail!("zoom must be between {MIN_ZOOM} and {MAX_ZOOM}");
    }
    Ok(zoom)
}

fn point(value: &Value, name: &str, fractions: bool) -> Result<[f64; 2]> {
    let list = value
        .as_array()
        .filter(|list| list.len() == 2)
        .ok_or_else(|| anyhow!("{name} must be a list of two numbers [x, y]"))?;
    let xy = [finite(&list[0], name)?, finite(&list[1], name)?];
    if fractions && !xy.iter().all(|v| (0.0..=1.0).contains(v)) {
        bail!("{name} takes fractions from 0 to 1 ([0.5, 0.5] is the center)");
    }
    if !fractions && xy.iter().any(|v| v.abs() > 1_000_000.0) {
        bail!("{name} offsets must be at most 1,000,000 px");
    }
    Ok(xy)
}

pub(in crate::parse) fn parse_shots(raw: &str) -> Result<Vec<ShotSpec>> {
    let list: Vec<Map<String, Value>> =
        serde_json::from_str(raw).context("expected a JSON array of shot objects")?;
    if list.is_empty() {
        bail!("at least one shot is required");
    }
    if list.len() > MAX_SHOTS {
        bail!("at most {MAX_SHOTS} shots are supported per camera");
    }
    let last = list.len() - 1;
    list.iter()
        .enumerate()
        .map(|(index, shot)| {
            (|| -> Result<ShotSpec> {
                reject_unknown(
                    shot,
                    &[
                        "preset", "duration", "zoom", "to", "anchor", "follow", "easing", "blend",
                        "from",
                    ],
                )?;
                let preset = Preset::parse(
                    shot.get("preset")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("preset is required (use {})", Preset::NAMES))?,
                )?;
                let duration = shot
                    .get("duration")
                    .map(|v| finite(v, "duration"))
                    .transpose()?;
                if duration.is_some_and(|d| d <= 0.0) {
                    bail!("duration must be above 0 seconds");
                }
                if duration.is_none() && index != last {
                    bail!("duration is required on every shot but the last");
                }
                let blend = shot.get("blend").map(|v| finite(v, "blend")).transpose()?;
                if blend.is_some_and(|b| !(0.0..=10.0).contains(&b)) {
                    bail!("blend must be between 0 and 10 seconds");
                }
                let follow = match shot.get("follow") {
                    None => None,
                    Some(Value::Bool(on)) => Some(*on),
                    Some(_) => bail!("follow must be true or false"),
                };
                let easing = match shot.get("easing") {
                    None => None,
                    Some(value) => Some(parse_camera_easing(
                        value
                            .as_str()
                            .ok_or_else(|| anyhow!("easing must be a string"))?,
                    )?),
                };
                let from = match shot.get("from") {
                    None => None,
                    Some(_) if index != 0 => {
                        bail!("from is only allowed on the first shot (later shots start where the previous one ends)")
                    }
                    Some(Value::Object(from)) => {
                        reject_unknown(from, &["zoom", "x", "y", "anchor"])?;
                        Some(Opening {
                            zoom: from.get("zoom").map(zoom).transpose()?,
                            focus: [
                                from.get("x").map(|v| finite(v, "x")).transpose()?,
                                from.get("y").map(|v| finite(v, "y")).transpose()?,
                            ],
                            anchor: from
                                .get("anchor")
                                .map(|v| point(v, "anchor", true))
                                .transpose()?,
                        })
                    }
                    Some(_) => bail!("from must be an object {{zoom, x, y, anchor}}"),
                };
                Ok(ShotSpec {
                    preset,
                    duration,
                    zoom: shot.get("zoom").map(zoom).transpose()?,
                    to: shot.get("to").map(|v| point(v, "to", false)).transpose()?,
                    anchor: shot
                        .get("anchor")
                        .map(|v| point(v, "anchor", true))
                        .transpose()?,
                    follow,
                    easing,
                    blend,
                    from,
                })
            })()
            .with_context(|| format!("shot {}", index + 1))
        })
        .collect()
}

/// Compiles the shots of a camera clip of `clip_len` seconds into rig keys.
pub(in crate::parse) fn compile_shots(
    shots: &[ShotSpec],
    options: &RigOptions,
    clip_len: f64,
) -> Result<Vec<RigKey>> {
    let has_subject = options.subject.is_some();
    let opening = shots[0].from.unwrap_or_default();
    let mut zoom = opening.zoom.unwrap_or(1.0);
    let mut anchor = opening.anchor.unwrap_or(options.anchor);
    let first = &shots[0];
    let mut follow = match first.preset {
        Preset::Follow => 1.0,
        Preset::Static | Preset::Pan => 0.0,
        _ => f64::from(u8::from(first.follow.unwrap_or(has_subject))),
    };
    let default_interp = options.easing.unwrap_or(Interp::Smooth);
    let mut keys = vec![RigKey {
        at: 0.0,
        zoom,
        focus: opening
            .focus
            .map(|v| v.map_or(FocusValue::Inherit, FocusValue::Fixed)),
        anchor,
        follow,
        interp: first.easing.unwrap_or(default_interp),
    }];
    let mut t = 0.0;
    let last = shots.len() - 1;
    for (index, shot) in shots.iter().enumerate() {
        let label = || format!("shot {} ({:?})", index + 1, shot.preset);
        let duration = match shot.duration {
            Some(d) => d,
            None if index == last => clip_len - t,
            None => unreachable!("checked when parsed"),
        };
        if duration <= 1e-9 {
            bail!(
                "{}: the previous shots already fill the camera clip ({clip_len:.2}s)",
                label()
            );
        }
        let end = t + duration;
        if end > clip_len + 1e-9 {
            bail!("data-hf-shots last {end:.2}s, longer than the camera clip ({clip_len:.2}s)");
        }
        let interp = shot.easing.unwrap_or(default_interp);
        let blend = shot.blend.unwrap_or(DEFAULT_BLEND).min(duration);
        if shot.follow == Some(true) && !has_subject {
            bail!(
                "{}: follow needs data-hf-camera-subject=\"<id>\" on the camera",
                label()
            );
        }
        let start_follow = follow;
        let mut end_zoom = zoom;
        let mut end_anchor = shot.anchor.unwrap_or(anchor);
        let reject = |name: &str, given: bool| -> Result<()> {
            if given {
                bail!("{}: {name} is not used by this preset", label());
            }
            Ok(())
        };
        let end_follow = match shot.preset {
            Preset::Static => {
                reject("zoom", shot.zoom.is_some() && index > 0)?;
                reject("to", shot.to.is_some())?;
                reject("anchor", shot.anchor.is_some())?;
                reject("follow", shot.follow.is_some())?;
                0.0
            }
            Preset::Follow => {
                if !has_subject {
                    bail!(
                        "{}: needs data-hf-camera-subject=\"<id>\" on the camera",
                        label()
                    );
                }
                reject("zoom", shot.zoom.is_some() && index > 0)?;
                reject("to", shot.to.is_some())?;
                reject("follow", shot.follow.is_some())?;
                1.0
            }
            Preset::Pan => {
                if shot.to.is_none() {
                    bail!(
                        "{}: needs to: [x, y], the focus at the end of the pan",
                        label()
                    );
                }
                reject("follow", shot.follow.is_some())?;
                end_zoom = shot.zoom.unwrap_or(zoom);
                0.0
            }
            Preset::DollyIn | Preset::DollyOut | Preset::Reveal => {
                let (factor, grows) = match shot.preset {
                    Preset::DollyIn => (DOLLY, true),
                    Preset::DollyOut => (1.0 / DOLLY, false),
                    _ => (1.0 / REVEAL, false),
                };
                end_zoom = shot
                    .zoom
                    .unwrap_or((zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM));
                if grows && end_zoom <= zoom {
                    bail!("{}: zoom must be above the current zoom ({zoom:.3}); use dolly-out to pull back", label());
                }
                if !grows && end_zoom >= zoom {
                    bail!("{}: zoom must be below the current zoom ({zoom:.3}); use dolly-in to push in", label());
                }
                if shot.follow == Some(true) && shot.to.is_some() {
                    bail!("{}: to sets a fixed focus, which a following camera ignores; drop to or set follow: false", label());
                }
                // A target focus frames a place, so it releases the subject unless told otherwise.
                let follows = match (shot.follow, shot.to) {
                    (Some(on), _) => f64::from(u8::from(on)),
                    (None, Some(_)) => 0.0,
                    (None, None) => start_follow,
                };
                if shot.preset == Preset::Reveal && shot.anchor.is_none() && follows > 0.0 {
                    end_anchor[1] = (anchor[1] + REVEAL_ANCHOR_DROP).min(1.0);
                }
                follows
            }
        };
        if index == 0 {
            // The first static or follow shot may set its zoom from the start.
            if let (Preset::Static | Preset::Follow, Some(z)) = (shot.preset, shot.zoom) {
                keys[0].zoom = z;
                end_zoom = z;
            }
        }
        // A camera that stops following eases to rest where the subject was midway.
        let rest = |window: f64| FocusValue::TrackedAt(t + window / 2.0);
        let end_focus = match shot.to {
            Some(to) => to.map(FocusValue::Fixed),
            None if end_follow > 0.0 => [FocusValue::Inherit; 2],
            None if start_follow > 0.0 => {
                let window = if shot.preset == Preset::Static {
                    blend
                } else {
                    duration
                };
                [rest(window); 2]
            }
            None => [FocusValue::Inherit; 2],
        };
        let changes_follow = (end_follow - start_follow).abs() > 0.0;
        let settles = matches!(shot.preset, Preset::Static | Preset::Follow)
            && changes_follow
            && blend > 1e-6
            && blend < duration - 1e-6;
        if settles {
            keys.push(RigKey {
                at: t + blend,
                zoom: end_zoom,
                focus: end_focus,
                anchor: end_anchor,
                follow: end_follow,
                interp,
            });
            keys.push(RigKey {
                at: end,
                zoom: end_zoom,
                focus: [FocusValue::Inherit; 2],
                anchor: end_anchor,
                follow: end_follow,
                interp,
            });
        } else {
            keys.push(RigKey {
                at: end,
                zoom: end_zoom,
                focus: end_focus,
                anchor: end_anchor,
                follow: end_follow,
                interp,
            });
        }
        zoom = end_zoom;
        anchor = end_anchor;
        follow = end_follow;
        t = end;
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> RigOptions {
        RigOptions {
            subject: Some("hero".into()),
            ..RigOptions::default()
        }
    }

    #[test]
    fn three_shot_sequence_compiles_to_continuous_keys() {
        let shots = parse_shots(
            r#"[{"preset":"follow","duration":2,"from":{"zoom":1.35}},
                {"preset":"dolly-in","duration":2,"zoom":1.6},
                {"preset":"reveal","zoom":1.0,"to":[0,40]}]"#,
        )
        .unwrap();
        let keys = compile_shots(&shots, &subject(), 6.0).unwrap();
        let times: Vec<f64> = keys.iter().map(|k| k.at).collect();
        assert_eq!(times, [0.0, 2.0, 4.0, 6.0]);
        let zooms: Vec<f64> = keys.iter().map(|k| k.zoom).collect();
        assert_eq!(zooms, [1.35, 1.35, 1.6, 1.0]);
        let follows: Vec<f64> = keys.iter().map(|k| k.follow).collect();
        assert_eq!(follows, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(
            keys[3].focus,
            [FocusValue::Fixed(0.0), FocusValue::Fixed(40.0)]
        );
        assert!(keys.iter().all(|k| k.interp == Interp::Smooth));
    }

    #[test]
    fn presets_default_and_settle() {
        let shots = parse_shots(
            r#"[{"preset":"follow","duration":2},{"preset":"static","duration":2},{"preset":"dolly-in","duration":1},{"preset":"dolly-out"}]"#,
        )
        .unwrap();
        let keys = compile_shots(&shots, &subject(), 6.0).unwrap();
        // The static shot settles within the blend, then holds.
        assert_eq!(keys[2].at, 2.75);
        assert_eq!(keys[2].follow, 0.0);
        assert_eq!(keys[2].focus[0], FocusValue::TrackedAt(2.375));
        assert_eq!(keys[3].focus[0], FocusValue::Inherit);
        assert!((keys[4].zoom - 1.25).abs() < 1e-12);
        assert!((keys[5].zoom - 1.0).abs() < 1e-12);
        assert_eq!(keys[5].at, 6.0);
    }

    #[test]
    fn invalid_shots_explain_the_fix() {
        let error = |raw: &str, options: &RigOptions, len: f64| {
            let shots = parse_shots(raw);
            let message = match shots {
                Err(e) => format!("{e:#}"),
                Ok(shots) => format!("{:#}", compile_shots(&shots, options, len).unwrap_err()),
            };
            message
        };
        let none = RigOptions::default();
        assert!(error(r#"[{"preset":"orbit"}]"#, &none, 2.0).contains("unknown preset"));
        assert!(error(
            r#"[{"preset":"pan","duration":1},{"preset":"static"}]"#,
            &none,
            2.0
        )
        .contains("needs to"));
        assert!(error(r#"[{"preset":"follow"}]"#, &none, 2.0).contains("data-hf-camera-subject"));
        assert!(error(
            r#"[{"preset":"static"},{"preset":"pan","to":[0,0]}]"#,
            &none,
            2.0
        )
        .contains("duration is required"));
        assert!(error(r#"[{"preset":"static","duration":3}]"#, &none, 2.0).contains("longer than"));
        assert!(error(r#"[{"preset":"dolly-in","zoom":0.5}]"#, &none, 2.0)
            .contains("above the current zoom"));
        assert!(error(
            r#"[{"preset":"static","duration":1},{"preset":"static","from":{"zoom":2}}]"#,
            &none,
            2.0
        )
        .contains("only allowed on the first shot"));
        assert!(error(r#"[{"preset":"static","speed":1}]"#, &none, 2.0).contains("unknown field"));
    }
}
