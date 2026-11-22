//! Arc-length measurement of vector paths: curves are flattened into a polyline with a cumulative
//! length table, so a distance along the path maps to a point and a tangent angle by binary search.
//! Flattening uses a fixed subdivision per curve, so results are deterministic.

use anyhow::{anyhow, bail, Result};
use resvg::usvg::tiny_skia_path::{Path, PathSegment, Point};

/// Line segments per quadratic or cubic curve.
const CURVE_STEPS: usize = 48;
/// Most flattened points kept for one path.
const MAX_POINTS: usize = 200_000;

/// A flattened path: polyline points (one list per subpath) and cumulative lengths.
#[derive(Clone, Debug, PartialEq)]
pub struct PathMeasure {
    /// `(x, y, distance from the start)` of every polyline vertex in order. A jump between subpaths
    /// repeats the distance, so it adds no length.
    points: Vec<(f32, f32, f32)>,
    /// Index into `points` where each subpath starts.
    starts: Vec<usize>,
}

impl PathMeasure {
    /// Flattens `path` (in its own coordinates).
    pub fn new(path: &Path) -> Result<Self> {
        let mut points: Vec<(f32, f32, f32)> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut total = 0.0_f32;
        let mut current = Point::zero();
        let mut start = Point::zero();
        let mut push = |points: &mut Vec<(f32, f32, f32)>, p: Point, connect: bool| -> Result<()> {
            if connect {
                if let Some(&(x, y, _)) = points.last() {
                    total += (p.x - x).hypot(p.y - y);
                }
            }
            points.push((p.x, p.y, total));
            if points.len() > MAX_POINTS {
                bail!("path is too complex (more than {MAX_POINTS} flattened points)");
            }
            Ok(())
        };
        for segment in path.segments() {
            match segment {
                PathSegment::MoveTo(p) => {
                    starts.push(points.len());
                    push(&mut points, p, false)?;
                    (current, start) = (p, p);
                }
                PathSegment::LineTo(p) => {
                    push(&mut points, p, true)?;
                    current = p;
                }
                PathSegment::QuadTo(c, p) => {
                    for i in 1..=CURVE_STEPS {
                        let t = i as f32 / CURVE_STEPS as f32;
                        let u = 1.0 - t;
                        let q = Point::from_xy(
                            u * u * current.x + 2.0 * u * t * c.x + t * t * p.x,
                            u * u * current.y + 2.0 * u * t * c.y + t * t * p.y,
                        );
                        push(&mut points, q, true)?;
                    }
                    current = p;
                }
                PathSegment::CubicTo(c1, c2, p) => {
                    for i in 1..=CURVE_STEPS {
                        let t = i as f32 / CURVE_STEPS as f32;
                        let u = 1.0 - t;
                        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        let q = Point::from_xy(
                            a * current.x + b * c1.x + c * c2.x + d * p.x,
                            a * current.y + b * c1.y + c * c2.y + d * p.y,
                        );
                        push(&mut points, q, true)?;
                    }
                    current = p;
                }
                PathSegment::Close => {
                    push(&mut points, start, true)?;
                    current = start;
                }
            }
        }
        if points.is_empty() {
            bail!("path has no points");
        }
        Ok(Self { points, starts })
    }

    /// The flattened subpaths as closed outlines (`(x, y)` vertices), for filling.
    pub fn contours(&self) -> Vec<Vec<(f32, f32)>> {
        let ends = self
            .starts
            .iter()
            .skip(1)
            .copied()
            .chain([self.points.len()]);
        self.starts
            .iter()
            .zip(ends)
            .map(|(&start, end)| self.points[start..end].iter().map(|p| (p.0, p.1)).collect())
            .filter(|contour: &Vec<(f32, f32)>| contour.len() >= 3)
            .collect()
    }

    /// Parses SVG path data (`M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `A`, `Z`, absolute or relative).
    pub fn from_svg_data(data: &str) -> Result<Self> {
        if data.contains(['"', '<', '>', '&']) {
            bail!("path data may only contain path commands and numbers");
        }
        // The SVG parser handles every command (arcs become curves); a stroke keeps straight lines,
        // which have an empty fill area, from being dropped.
        let markup = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path d="{data}" fill="none" stroke="black"/></svg>"#
        );
        let tree = crate::render::svg::parse_svg(markup.as_bytes())?;
        let mut found = None;
        visit_paths(tree.root(), &mut |path| {
            if found.is_none() {
                found = Some(path.data().clone());
            }
        });
        let path = found.ok_or_else(|| anyhow!("path data draws nothing: {data}"))?;
        Self::new(&path)
    }

    /// Total length.
    pub fn length(&self) -> f32 {
        self.points.last().map_or(0.0, |p| p.2)
    }

    /// The first point.
    pub fn start(&self) -> (f32, f32) {
        let (x, y, _) = self.points[0];
        (x, y)
    }

    /// Point and tangent angle (degrees, clockwise from +x) at `distance` along the path, clamped
    /// to its ends.
    pub fn sample(&self, distance: f32) -> (f32, f32, f32) {
        let points = &self.points;
        if points.len() == 1 {
            let (x, y, _) = points[0];
            return (x, y, 0.0);
        }
        let d = distance.clamp(0.0, self.length());
        // The first vertex whose distance reaches `d`, past any zero-length jump.
        let next = points
            .partition_point(|p| p.2 < d)
            .clamp(1, points.len() - 1);
        let (mut i, mut j) = (next - 1, next);
        // Skip zero-length steps (subpath jumps, repeated points) for the tangent.
        while j + 1 < points.len() && points[j].2 - points[i].2 <= 0.0 {
            i = j;
            j += 1;
        }
        let (a, b) = (points[i], points[j]);
        let span = b.2 - a.2;
        let t = if span > 0.0 {
            ((d - a.2) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let angle = (b.1 - a.1).atan2(b.0 - a.0).to_degrees();
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, angle)
    }
}

/// Calls `f` for every path in the tree, in document order (including nested groups).
pub fn visit_paths(group: &resvg::usvg::Group, f: &mut dyn FnMut(&resvg::usvg::Path)) {
    for node in group.children() {
        match node {
            resvg::usvg::Node::Group(group) => visit_paths(group, f),
            resvg::usvg::Node::Path(path) => f(path),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_lines_and_samples_points() {
        let m = PathMeasure::from_svg_data("M0 0 L300 0 L300 400").unwrap();
        assert_eq!(m.length(), 700.0);
        assert_eq!(m.sample(150.0), (150.0, 0.0, 0.0));
        let (x, y, angle) = m.sample(500.0);
        assert_eq!((x, y), (300.0, 200.0));
        assert_eq!(angle, 90.0);
        assert_eq!(m.sample(-5.0).0, 0.0);
        assert_eq!(m.sample(1e9).1, 400.0);
    }

    #[test]
    fn curves_and_arcs_have_close_lengths() {
        // A half circle of radius 100 is about 314.16 long.
        let m = PathMeasure::from_svg_data("M0 0 A100 100 0 0 1 200 0").unwrap();
        assert!(
            (m.length() - std::f32::consts::PI * 100.0).abs() < 0.5,
            "{}",
            m.length()
        );
        let relative = PathMeasure::from_svg_data("m10 10 h50 v50").unwrap();
        assert_eq!(relative.length(), 100.0);
        assert_eq!(relative.start(), (10.0, 10.0));
    }

    #[test]
    fn subpath_jumps_add_no_length() {
        let m = PathMeasure::from_svg_data("M0 0 H10 M100 100 H110").unwrap();
        assert_eq!(m.length(), 20.0);
        assert_eq!(m.sample(15.0), (105.0, 100.0, 0.0));
    }

    #[test]
    fn rejects_bad_data() {
        assert!(PathMeasure::from_svg_data("not a path").is_err());
        assert!(PathMeasure::from_svg_data("M0 0\" onload=\"x").is_err());
    }
}
