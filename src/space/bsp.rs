//! Exact back-to-front order of the planes of a space: a binary space partition (BSP) tree over
//! their boxes, which splits a plane where another one cuts through it.
//!
//! Each input polygon is a plane's border box (`half` size, plane-local px around its center)
//! carried by its world transform. The tree is built in paint order: the first polygon of a list
//! splits the rest into the ones in front of its plane, behind it, and on it (coplanar, kept in
//! paint order, so planes at one depth draw like 2D siblings). A polygon that crosses the splitting
//! plane becomes two fragments, each recording the half-plane it keeps, in its own plane-local
//! coordinates, as `c0 + cu * u + cv * v >= 0`. Walking the tree away from the eye yields fragments
//! that never hide something drawn after them, including planes that intersect. The result depends
//! only on the transforms and the eye.

use super::math::{clip_half_plane, Affine};

/// Planes closer than this (px) to a splitting plane count as lying on it.
const ON_PLANE: f64 = 1e-3;
/// Above this many fragments the tree is abandoned for a plain distance sort.
pub const MAX_FRAGMENTS: usize = 4096;

/// One plane to order.
#[derive(Clone, Debug)]
pub struct Polygon {
    /// The caller's identifier, returned with every fragment.
    pub id: usize,
    pub plane: Affine,
    /// Half width and half height of the box (plane-local px).
    pub half: [f64; 2],
}

/// A visible piece of a plane: the plane `id` cut by `clips` (none for a whole plane).
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    pub id: usize,
    pub clips: Vec<[f64; 3]>,
}

/// Where the camera is: a point (perspective) or a direction toward the viewer (orthographic).
#[derive(Clone, Copy, Debug)]
pub enum Eye {
    Point([f64; 3]),
    Direction([f64; 3]),
}

/// A polygon during partitioning: its plane-local outline and the cuts made so far.
#[derive(Clone)]
struct Piece {
    index: usize,
    outline: Vec<[f64; 2]>,
    clips: Vec<[f64; 3]>,
}

struct Node {
    splitter: usize,
    coplanar: Vec<Piece>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
}

/// Unit normal and a point of polygon `p`'s plane, or `None` for a degenerate (zero-size) plane.
fn plane_of(p: &Polygon) -> Option<([f64; 3], [f64; 3])> {
    let n = p.plane.axis(2);
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (length > 1e-12 && length.is_finite()).then(|| (n.map(|v| v / length), p.plane.t))
}

/// Signed distance of `x` from the plane `(normal, point)`.
fn side(normal: [f64; 3], point: [f64; 3], x: [f64; 3]) -> f64 {
    normal[0] * (x[0] - point[0]) + normal[1] * (x[1] - point[1]) + normal[2] * (x[2] - point[2])
}

/// Orders `polygons` (given in paint order) back to front as seen from `eye`. Returns `None` when
/// the partition would exceed [`MAX_FRAGMENTS`]; the caller then sorts by distance instead.
pub fn order(polygons: &[Polygon], eye: Eye) -> Option<Vec<Fragment>> {
    let planes: Vec<Option<([f64; 3], [f64; 3])>> = polygons.iter().map(plane_of).collect();
    let pieces: Vec<Piece> = polygons
        .iter()
        .enumerate()
        .filter(|(i, _)| planes[*i].is_some())
        .map(|(index, p)| Piece {
            index,
            outline: vec![
                [-p.half[0], -p.half[1]],
                [p.half[0], -p.half[1]],
                [p.half[0], p.half[1]],
                [-p.half[0], p.half[1]],
            ],
            clips: Vec::new(),
        })
        .collect();
    let mut budget = MAX_FRAGMENTS;
    let tree = build(pieces, polygons, &planes, &mut budget)?;
    let mut out = Vec::with_capacity(polygons.len());
    walk(&tree, polygons, &planes, eye, &mut out);
    Some(out)
}

fn build(
    mut pieces: Vec<Piece>,
    polygons: &[Polygon],
    planes: &[Option<([f64; 3], [f64; 3])>],
    budget: &mut usize,
) -> Option<Option<Box<Node>>> {
    if pieces.is_empty() {
        return Some(None);
    }
    let first = pieces.remove(0);
    let (normal, point) = planes[first.index].expect("pieces have planes");
    let mut coplanar = vec![first];
    let (mut front, mut back) = (Vec::new(), Vec::new());
    for piece in pieces {
        let plane = &polygons[piece.index].plane;
        let distances: Vec<f64> = piece
            .outline
            .iter()
            .map(|&[u, v]| side(normal, point, plane.apply([u, v, 0.0])))
            .collect();
        let ahead = distances.iter().any(|&d| d > ON_PLANE);
        let behind = distances.iter().any(|&d| d < -ON_PLANE);
        match (ahead, behind) {
            (false, false) => coplanar.push(piece),
            (true, false) => front.push(piece),
            (false, true) => back.push(piece),
            (true, true) => {
                *budget = budget.checked_sub(1)?;
                // The signed distance is affine in the piece's own (u, v).
                let a0 = plane.axis(0);
                let a1 = plane.axis(1);
                let cut = [
                    side(normal, point, plane.t),
                    normal[0] * a0[0] + normal[1] * a0[1] + normal[2] * a0[2],
                    normal[0] * a1[0] + normal[1] * a1[1] + normal[2] * a1[2],
                ];
                for (clip, list) in [(cut, &mut front), (cut.map(|c| -c), &mut back)] {
                    let outline = clip_half_plane(&piece.outline, clip);
                    if outline.len() >= 3 {
                        let mut clips = piece.clips.clone();
                        clips.push(clip);
                        list.push(Piece {
                            index: piece.index,
                            outline,
                            clips,
                        });
                    }
                }
            }
        }
    }
    let splitter = coplanar[0].index;
    Some(Some(Box::new(Node {
        splitter,
        coplanar,
        front: build(front, polygons, planes, budget)?,
        back: build(back, polygons, planes, budget)?,
    })))
}

fn walk(
    node: &Option<Box<Node>>,
    polygons: &[Polygon],
    planes: &[Option<([f64; 3], [f64; 3])>],
    eye: Eye,
    out: &mut Vec<Fragment>,
) {
    let Some(node) = node else {
        return;
    };
    let (normal, point) = planes[node.splitter].expect("splitters have planes");
    let facing = match eye {
        Eye::Point(p) => side(normal, point, p),
        Eye::Direction(d) => normal[0] * d[0] + normal[1] * d[1] + normal[2] * d[2],
    };
    let (far, near) = if facing >= 0.0 {
        (&node.back, &node.front)
    } else {
        (&node.front, &node.back)
    };
    walk(far, polygons, planes, eye, out);
    out.extend(node.coplanar.iter().map(|piece| Fragment {
        id: polygons[piece.index].id,
        clips: piece.clips.clone(),
    }));
    walk(near, polygons, planes, eye, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(id: usize, plane: Affine) -> Polygon {
        Polygon {
            id,
            plane,
            half: [100.0, 100.0],
        }
    }

    const EYE: Eye = Eye::Point([0.0, 0.0, 1000.0]);

    #[test]
    fn parallel_planes_draw_far_to_near_and_coplanar_in_paint_order() {
        let near = square(0, Affine::pose([0.0, 0.0, 50.0], 0.0, 0.0, 0.0, 1.0));
        let far = square(1, Affine::pose([0.0, 0.0, -50.0], 0.0, 0.0, 0.0, 1.0));
        let flat_a = square(2, Affine::pose([300.0, 0.0, -50.0], 0.0, 0.0, 0.0, 1.0));
        let order = order(&[near, far, flat_a], EYE).unwrap();
        let ids: Vec<usize> = order.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec![1, 2, 0]);
        assert!(order.iter().all(|f| f.clips.is_empty()));
        // From behind, the order flips.
        let behind = order_ids(Eye::Point([0.0, 0.0, -1000.0]));
        assert_eq!(behind, vec![0, 1, 2]);
    }

    fn order_ids(eye: Eye) -> Vec<usize> {
        let near = square(0, Affine::pose([0.0, 0.0, 50.0], 0.0, 0.0, 0.0, 1.0));
        let far = square(1, Affine::pose([0.0, 0.0, -50.0], 0.0, 0.0, 0.0, 1.0));
        let flat_a = square(2, Affine::pose([300.0, 0.0, -50.0], 0.0, 0.0, 0.0, 1.0));
        order(&[near, far, flat_a], eye)
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect()
    }

    #[test]
    fn intersecting_planes_are_split_into_ordered_fragments() {
        let wall = square(0, Affine::pose([0.0; 3], 0.0, 0.0, 0.0, 1.0));
        let cross = square(1, Affine::pose([0.0; 3], 0.0, 60.0, 0.0, 1.0));
        let order = order(&[wall, cross], EYE).unwrap();
        // The crossing plane is cut in two: its far half first, the wall, then its near half.
        assert_eq!(order.len(), 3);
        assert_eq!((order[0].id, order[1].id, order[2].id), (1, 0, 1));
        assert_eq!(order[0].clips.len(), 1);
        let [c0, cu, _] = order[2].clips[0];
        // The near half keeps u < 0 (rotateY 60 turns +u away from the camera).
        assert!(c0.abs() < 1e-9 && cu < 0.0);
    }

    #[test]
    fn a_floor_under_upright_planes_sorts_by_geometry() {
        // A floor reaching from far behind to the front, and an upright card standing on it.
        let floor = Polygon {
            id: 0,
            plane: Affine::pose([0.0, 100.0, -500.0], 90.0, 0.0, 0.0, 1.0),
            half: [400.0, 600.0],
        };
        let card = square(1, Affine::pose([0.0, 0.0, -800.0], 0.0, 0.0, 0.0, 1.0));
        let ids: Vec<usize> = order(&[card.clone(), floor.clone()], EYE)
            .unwrap()
            .iter()
            .map(|f| f.id)
            .collect();
        // The card stands on the floor: the part of the floor behind it draws first.
        assert_eq!(ids.first(), Some(&0));
        assert!(ids.contains(&1));
        assert!(order(&[floor, card], EYE).is_some());
    }
}
