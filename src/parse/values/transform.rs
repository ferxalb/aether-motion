//! `transform` parsing: translate (px, or % of the element's own border box), uniform scale, and
//! rotate; and `transform-origin` (the pivot of that scale and rotation).
use crate::model::Length;
use crate::parse::values::length::parse_length;
use anyhow::{anyhow, bail, Context, Result};

/// A parsed `transform`, composed left to right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) struct TransformParts {
    /// Translation in px.
    pub tx: f32,
    pub ty: f32,
    /// Uniform scale (the square root of the area scale when `shape` is set).
    pub scale: f32,
    /// Rotation in degrees, clockwise.
    pub rotate: f32,
    /// Percent part of the translation as
    /// `[x per % of width, x per % of height, y per % of width, y per % of height]` (in fractions
    /// of the box, after the functions written before it), resolved once layout is known.
    pub percent: [f32; 4],
    /// What `skew()`, `skewX()`, `skewY()`, and non-uniform `scale()` add beyond `scale` and
    /// `rotate`: a row-major 2x2 matrix with determinant 1 applied before them, so the whole
    /// linear part is `scale · R(rotate) · shape`. `None` for a plain scale and rotation.
    pub shape: Option<[f32; 4]>,
}

/// `transform` as (translate x, translate y, uniform scale, rotation in degrees, percent part):
/// the forms that CSS `@keyframes` can animate. Skew and non-uniform scale are errors here.
pub(in crate::parse) fn parse_transform_with_percent(
    raw: &str,
) -> Result<(f32, f32, f32, f32, [f32; 4])> {
    let parts = parse_transform(raw)?;
    if parts.shape.is_some() {
        bail!("transform: skew() and non-uniform scale() cannot be animated; put them in a static transform (on a parent if this element animates its transform)");
    }
    Ok((parts.tx, parts.ty, parts.scale, parts.rotate, parts.percent))
}

/// Parses a static `transform`: `translate`, `translateX`, `translateY` (px or %), `scale`,
/// `scaleX`, `scaleY` (non-negative factors), `rotate`, `skew`, `skewX`, and `skewY`.
pub(in crate::parse) fn parse_transform(raw: &str) -> Result<TransformParts> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(TransformParts {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
            rotate: 0.0,
            percent: [0.0; 4],
            shape: None,
        });
    }
    // A translate argument as (px, fraction of the box side).
    let px = |arg: &str| -> Result<(f32, f32)> {
        match parse_length(arg, "transform")? {
            Length::Px(n) => Ok((n, 0.0)),
            Length::Percent(n) if n.is_finite() => Ok((0.0, n / 100.0)),
            Length::Percent(_) => bail!("transform: translate % is not finite"),
        }
    };
    let factor = |arg: &str| -> Result<f32> {
        let n: f32 = arg
            .parse()
            .with_context(|| format!("transform: invalid scale factor: {arg}"))?;
        if !n.is_finite() || n < 0.0 {
            bail!("transform: scale must be a non-negative number");
        }
        Ok(n)
    };
    let angle = |arg: &str| -> Result<f32> {
        let (number, per_unit) = if let Some(n) = arg.strip_suffix("deg") {
            (n, 1.0)
        } else if let Some(n) = arg.strip_suffix("turn") {
            (n, 360.0)
        } else if let Some(n) = arg.strip_suffix("rad") {
            (n, 180.0 / std::f32::consts::PI)
        } else if arg == "0" {
            ("0", 1.0)
        } else {
            bail!("transform: angles need deg, rad or turn: {arg}");
        };
        let n: f32 = number
            .trim()
            .parse()
            .with_context(|| format!("transform: invalid angle: {arg}"))?;
        if !n.is_finite() {
            bail!("transform: angle is not finite");
        }
        Ok(n * per_unit)
    };
    // The slope of a skew angle; at a quarter turn the element would become a line.
    let slope = |arg: &str| -> Result<f64> {
        let degrees = angle(arg)?;
        let turns = (f64::from(degrees) / 180.0 - 0.5).rem_euclid(1.0);
        if (turns - 0.0).abs() < 1e-6 || (turns - 1.0).abs() < 1e-6 {
            bail!("transform: skew of {arg} flattens the element into a line");
        }
        Ok(f64::from(degrees).to_radians().tan())
    };
    let mut state = Compose::default();
    let mut rest = raw;
    while !rest.trim().is_empty() {
        let text = rest.trim_start();
        let (open, close) = text
            .find('(')
            .zip(text.find(')'))
            .filter(|(open, close)| open < close)
            .ok_or_else(|| anyhow!("invalid transform: {raw}"))?;
        let name = text[..open].trim().to_ascii_lowercase();
        let args: Vec<_> = text[open + 1..close].split(',').map(str::trim).collect();
        let zero = (0.0, 0.0);
        let diagonal = |a: f32, b: f32| [f64::from(a), 0.0, 0.0, f64::from(b)];
        match (name.as_str(), args.as_slice()) {
            ("translate", [x]) | ("translatex", [x]) => state.shift(px(x)?, zero),
            ("translatey", [y]) => state.shift(zero, px(y)?),
            ("translate", [x, y]) => state.shift(px(x)?, px(y)?),
            ("scale", [k]) => {
                let k = factor(k)?;
                state.apply(diagonal(k, k), Some((k, 0.0)));
            }
            ("scale", [a, b]) => {
                let (a, b) = (factor(a)?, factor(b)?);
                state.apply(diagonal(a, b), (a == b).then_some((a, 0.0)));
            }
            ("scalex", [a]) => {
                let a = factor(a)?;
                state.apply(diagonal(a, 1.0), (a == 1.0).then_some((1.0, 0.0)));
            }
            ("scaley", [b]) => {
                let b = factor(b)?;
                state.apply(diagonal(1.0, b), (b == 1.0).then_some((1.0, 0.0)));
            }
            ("rotate", [a]) => {
                let turn = angle(a)?;
                let (sin, cos) = f64::from(turn).to_radians().sin_cos();
                state.apply([cos, -sin, sin, cos], Some((1.0, turn)));
            }
            ("skew", [x]) | ("skewx", [x]) => {
                let kx = slope(x)?;
                state.apply([1.0, kx, 0.0, 1.0], (kx == 0.0).then_some((1.0, 0.0)));
            }
            ("skewy", [y]) => {
                let ky = slope(y)?;
                state.apply([1.0, 0.0, ky, 1.0], (ky == 0.0).then_some((1.0, 0.0)));
            }
            ("skew", [x, y]) => {
                let (kx, ky) = (slope(x)?, slope(y)?);
                state.apply(
                    [1.0, kx, ky, 1.0],
                    (kx == 0.0 && ky == 0.0).then_some((1.0, 0.0)),
                );
            }
            _ => bail!(
                "unsupported transform: {name}(...); use translate, translateX, translateY, scale, scaleX, scaleY, rotate, skew, skewX, or skewY"
            ),
        }
        rest = &text[close + 1..];
    }
    let Compose {
        tx,
        ty,
        mut scale,
        mut rotate,
        percent,
        matrix,
    } = state;
    let shape = match matrix {
        None => None,
        Some(m) => {
            let (s, turn, shape) = decompose(m, raw)?;
            (scale, rotate) = (s, turn);
            shape
        }
    };
    Ok(TransformParts {
        tx,
        ty,
        scale,
        rotate,
        percent,
        shape,
    })
}

/// Running composition of transform functions, left to right.
struct Compose {
    tx: f32,
    ty: f32,
    scale: f32,
    rotate: f32,
    percent: [f32; 4],
    /// The whole linear part, kept in f64 once a skew or non-uniform scale makes it more than a
    /// scale and rotation; until then `scale` and `rotate` stay exact as written, so plain
    /// transforms parse exactly as before skew existed.
    matrix: Option<[f64; 4]>,
}

impl Default for Compose {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
            rotate: 0.0,
            percent: [0.0; 4],
            matrix: None,
        }
    }
}

impl Compose {
    /// Adds a translation by `(x px + fx of the width, y px + fy of the height)`, mapped by the
    /// linear part written before it (`C ∘ F`: a later translation is scaled, turned, skewed).
    fn shift(&mut self, (x, fx): (f32, f32), (y, fy): (f32, f32)) {
        if let Some([a, b, c, d]) = self.matrix {
            let (x, y, fx, fy) = (f64::from(x), f64::from(y), f64::from(fx), f64::from(fy));
            self.tx += (a * x + b * y) as f32;
            self.ty += (c * x + d * y) as f32;
            self.percent[0] += (a * fx) as f32;
            self.percent[1] += (b * fy) as f32;
            self.percent[2] += (c * fx) as f32;
            self.percent[3] += (d * fy) as f32;
            return;
        }
        let (sin, cos) = self.rotate.to_radians().sin_cos();
        let scale = self.scale;
        self.tx += scale * (x * cos - y * sin);
        self.ty += scale * (x * sin + y * cos);
        self.percent[0] += scale * fx * cos;
        self.percent[1] -= scale * fy * sin;
        self.percent[2] += scale * fx * sin;
        self.percent[3] += scale * fy * cos;
    }

    /// Multiplies the linear part by `m` on the right. `similarity` is `Some((scale, degrees))`
    /// when `m` is a plain scale and rotation, which stays in the exact scalar fields until the
    /// first function that is not.
    fn apply(&mut self, m: [f64; 4], similarity: Option<(f32, f32)>) {
        if let (None, Some((k, turn))) = (self.matrix, similarity) {
            self.scale *= k;
            self.rotate += turn;
            return;
        }
        let [a, b, c, d] = self.matrix.unwrap_or_else(|| {
            let (sin, cos) = f64::from(self.rotate).to_radians().sin_cos();
            let k = f64::from(self.scale);
            [k * cos, -k * sin, k * sin, k * cos]
        });
        let [e, f, g, h] = m;
        self.matrix = Some([a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h]);
    }
}

/// Splits a linear map `[a, b, c, d]` into `s · R(θ) · K`: θ turns its first column onto the x
/// axis, `s` is the square root of its determinant, and `K` (determinant 1) is upper triangular.
/// `K` is `None` when it is the identity, so a product that turns out to be a plain scale and
/// rotation draws like one.
fn decompose(m: [f64; 4], raw: &str) -> Result<(f32, f32, Option<[f32; 4]>)> {
    let [a, b, c, d] = m;
    let det = a * d - b * c;
    let column = a.hypot(c);
    if !det.is_finite() || det <= 1e-12 || column <= 1e-9 {
        bail!("transform: {raw} flattens or mirrors the element (a skew or scale that leaves no area); use smaller angles or non-zero scales");
    }
    let theta = c.atan2(a);
    let s = det.sqrt();
    // R(-θ) · m = [[column, (a·b + c·d) / column], [0, det / column]].
    let shape = [
        column / s,
        (a * b + c * d) / column / s,
        0.0,
        det / column / s,
    ];
    let identity = [1.0, 0.0, 0.0, 1.0];
    let near = shape
        .iter()
        .zip(identity)
        .all(|(value, unit)| (value - unit).abs() < 1e-6);
    let shape = (!near).then(|| shape.map(|v| v as f32));
    Ok((s as f32, theta.to_degrees() as f32, shape))
}

/// `transform-origin`: one or two values (`left`/`center`/`right`, `top`/`center`/`bottom`, px,
/// or %) and an optional depth that must be zero. `None` is the default center.
pub(in crate::parse) fn parse_transform_origin(raw: &str) -> Result<Option<[Length; 2]>> {
    let raw = raw.trim();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    // A keyword's axis: `Some(true)` horizontal only, `Some(false)` vertical only, `None`
    // (`center`) either axis. Lengths are `Err(length)`.
    let value = |token: &str| -> Result<std::result::Result<(Length, Option<bool>), Length>> {
        Ok(Ok(match token.to_ascii_lowercase().as_str() {
            "left" => (Length::Percent(0.0), Some(true)),
            "right" => (Length::Percent(100.0), Some(true)),
            "top" => (Length::Percent(0.0), Some(false)),
            "bottom" => (Length::Percent(100.0), Some(false)),
            "center" => (Length::Percent(50.0), None),
            _ => return Ok(Err(parse_length(token, "transform-origin")?)),
        }))
    };
    let center = Length::Percent(50.0);
    let (x, y) = match tokens.as_slice() {
        [one] => match value(one)? {
            Ok((v, Some(false))) => (center, v),
            Ok((v, _)) | Err(v) => (v, center),
        },
        [a, b] | [a, b, _] => match (value(a)?, value(b)?) {
            // Two keywords may come in either order (`top left`, `center right`).
            (Ok((a, axis_a)), Ok((b, axis_b))) => match (axis_a, axis_b) {
                (Some(h), Some(v)) if h == v => {
                    bail!("transform-origin: both values name the same axis: {raw}")
                }
                (Some(false), _) | (_, Some(true)) => (b, a),
                _ => (a, b),
            },
            // With a length, the first value is horizontal and the second vertical.
            (Ok((_, Some(false))), Err(_)) | (Err(_), Ok((_, Some(true)))) => {
                bail!("transform-origin: with a length, give the horizontal value first: {raw}")
            }
            (Ok((a, _)) | Err(a), Ok((b, _)) | Err(b)) => (a, b),
        },
        _ => bail!("transform-origin takes one to three values: {raw}"),
    };
    if let [_, _, z] = tokens.as_slice() {
        if parse_length(z, "transform-origin")? != Length::Px(0.0) {
            bail!("transform-origin: a depth (3D origin) is not supported; use two values");
        }
    }
    let is_center = |v: Length| v == center;
    Ok((!(is_center(x) && is_center(y))).then_some([x, y]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full linear part `scale · R(rotate) · shape` of a parsed transform.
    fn linear(parts: &TransformParts) -> [f64; 4] {
        let (sin, cos) = f64::from(parts.rotate).to_radians().sin_cos();
        let k = f64::from(parts.scale);
        let [e, f, g, h] = parts
            .shape
            .map_or([1.0, 0.0, 0.0, 1.0], |m| m.map(f64::from));
        let [a, b, c, d] = [k * cos, -k * sin, k * sin, k * cos];
        [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h]
    }

    #[test]
    fn similarity_transforms_keep_their_exact_scale_and_rotation() {
        let parts =
            parse_transform("translate(3px, 1px) scale(2) rotate(720deg) scale(1.5, 1.5)").unwrap();
        assert_eq!(
            (parts.tx, parts.ty, parts.scale, parts.rotate),
            (3.0, 1.0, 3.0, 720.0)
        );
        assert_eq!(parts.shape, None);
        for raw in [
            "skew(0deg)",
            "skewX(0)",
            "skewY(0turn)",
            "scaleX(1) scaleY(1)",
            "skew(0, 0)",
        ] {
            let parts = parse_transform(raw).unwrap();
            assert_eq!(
                (parts.scale, parts.rotate, parts.shape),
                (1.0, 0.0, None),
                "{raw}"
            );
        }
    }

    #[test]
    fn skew_and_non_uniform_scale_decompose_into_scale_rotation_and_shape() {
        let tan = |deg: f64| deg.to_radians().tan();
        let cases: [(&str, [f64; 4]); 6] = [
            ("skewX(30deg)", [1.0, tan(30.0), 0.0, 1.0]),
            ("skewY(-20deg)", [1.0, 0.0, tan(-20.0), 1.0]),
            ("skew(10deg, 5deg)", [1.0, tan(10.0), tan(5.0), 1.0]),
            ("scale(2, 0.5)", [2.0, 0.0, 0.0, 0.5]),
            ("scaleX(3)", [3.0, 0.0, 0.0, 1.0]),
            // R(90°)·skewX(45°) = [[0, -1], [1, 0]]·[[1, 1], [0, 1]] = [[0, -1], [1, 1]].
            ("rotate(90deg) skewX(45deg)", [0.0, -1.0, 1.0, 1.0]),
        ];
        for (raw, expected) in cases {
            let parts = parse_transform(raw).unwrap();
            let shape = parts.shape.unwrap_or_else(|| panic!("{raw} has no shape"));
            assert!(
                (f64::from(shape[0] * shape[3] - shape[1] * shape[2]) - 1.0).abs() < 1e-5,
                "{raw}"
            );
            assert_eq!(shape[2], 0.0, "{raw}");
            for (got, want) in linear(&parts).iter().zip(expected) {
                assert!(
                    (got - want).abs() < 1e-5,
                    "{raw}: {:?} vs {expected:?}",
                    linear(&parts)
                );
            }
        }
        // A translation written after a skew is skewed: skewX(45deg) maps (0, 4) to (4, 4).
        let parts = parse_transform("skewX(45deg) translate(0, 4px)").unwrap();
        assert!((parts.tx - 4.0).abs() < 1e-5 && (parts.ty - 4.0).abs() < 1e-5);
        // And its percent part: y% of the height also moves x.
        let parts = parse_transform("skewX(45deg) translateY(50%)").unwrap();
        assert!((parts.percent[1] - 0.5).abs() < 1e-6 && (parts.percent[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn degenerate_and_animated_shapes_are_errors() {
        for raw in [
            "skewX(90deg)",
            "skew(-90deg)",
            "skewY(0.25turn)",
            "skew(45deg, 45deg)",
            "scaleX(0)",
            "scale(2, 0)",
            "skew(10)",
        ] {
            assert!(parse_transform(raw).is_err(), "{raw}");
        }
        let error = parse_transform_with_percent("skewX(10deg)")
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot be animated"), "{error}");
        assert!(parse_transform_with_percent("scale(2) rotate(10deg)").is_ok());
    }

    #[test]
    fn transform_origin_follows_css_value_order() {
        let p = Length::Percent;
        let cases = [
            ("center", None),
            ("50% 50%", None),
            ("center center 0", None),
            ("left", Some([p(0.0), p(50.0)])),
            ("top", Some([p(50.0), p(0.0)])),
            ("top left", Some([p(0.0), p(0.0)])),
            ("left top", Some([p(0.0), p(0.0)])),
            ("right bottom", Some([p(100.0), p(100.0)])),
            ("bottom center", Some([p(50.0), p(100.0)])),
            ("10px 25%", Some([Length::Px(10.0), p(25.0)])),
            ("0 0", Some([Length::Px(0.0), Length::Px(0.0)])),
            ("left 4px", Some([p(0.0), Length::Px(4.0)])),
            ("center left", Some([p(0.0), p(50.0)])),
            ("4px bottom", Some([Length::Px(4.0), p(100.0)])),
            ("RIGHT TOP", Some([p(100.0), p(0.0)])),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_transform_origin(raw).unwrap(), expected, "{raw}");
        }
        for raw in [
            "",
            "left right",
            "top bottom",
            "4px left",
            "top 4px",
            "0 0 10px",
            "1 2 3 4",
            "50em",
        ] {
            assert!(parse_transform_origin(raw).is_err(), "{raw}");
        }
    }
}
