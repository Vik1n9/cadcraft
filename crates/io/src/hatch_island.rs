//! Hatch boundary-loop nesting: how deeply each loop sits inside the others, which decides the
//! boundary-path type flag (group 92) CADCraft writes. The DXF Reference defines those types as
//! default (no bit), external (bit 1) and outermost (bit 16); a hatch's outer boundary is external,
//! the first tier of islands is outermost, and deeper tiers are the default type. AutoCAD's Outer
//! hatch style uses those types. What AutoCAD's Normal style draws from them is not verified here.

use cadcraft_doc::HatchLoop;
use cadcraft_geom::{Bounds2, Line, PolyVertex, Polyline, Vec2, point_in_polygon};

/// Boundary path type flag: the hatch's external boundary (group 92 bit 0).
pub(crate) const EXTERNAL: i64 = 1;
/// Boundary path type flag: a polyline boundary path (group 92 bit 1).
pub(crate) const POLYLINE: i64 = 2;
/// Boundary path type flag: the first tier of inner boundaries, the islands (group 92 bit 4).
pub(crate) const OUTERMOST: i64 = 16;

/// Upper bound on the nesting test's sampled-vertex × vertex work for one hatch (hostile files).
const MAX_WORK: u64 = 20_000_000;
/// Upper bound on the tessellated points of all loops of one hatch.
const MAX_POINTS: usize = 1_000_000;
/// Vertices of an inner loop tested against an enclosing loop before deciding. An island that
/// shares an edge puts some of its vertices exactly on the enclosing loop, so one probe is not
/// enough.
const PROBES: usize = 8;

/// The group-92 flags for a loop at `depth` (how many other loops enclose it).
pub(crate) fn path_flags(depth: usize) -> i64 {
    POLYLINE
        | match depth {
            0 => EXTERNAL,
            1 => OUTERMOST,
            _ => 0,
        }
}

/// True when the group-92 flags carry no tier information: no loop is `OUTERMOST` and at most one
/// is `EXTERNAL` (every loop is 2 or 3). That is the pattern older CADCraft versions wrote; only
/// then does the reader fall back to the geometry instead of the file's flags.
pub(crate) fn no_tier_information(flags: &[i64]) -> bool {
    let mut external = 0usize;
    for &flags in flags {
        if flags & OUTERMOST != 0 || flags & !(EXTERNAL | POLYLINE) != 0 {
            return false;
        }
        external += usize::from(flags & EXTERNAL != 0);
    }
    !flags.is_empty() && external <= 1
}

/// The nesting depth of every loop: how many other loops enclose it. `None` when the loops are
/// too large to test within the work budget, or a loop has no usable outline (then the caller keeps
/// the flags it has).
pub(crate) fn depths(loops: &[HatchLoop]) -> Option<Vec<usize>> {
    let count = loops.len();
    let raw: usize = loops.iter().map(|l| l.vertices.len()).sum();
    if count == 0 || raw.saturating_mul(count).saturating_mul(PROBES) > MAX_WORK as usize {
        return None;
    }
    let bounds = Bounds2::from_points(loops.iter().flat_map(|l| l.vertices.iter().map(|v| v.p)));
    if bounds.is_empty() || !bounds.width().is_finite() || !bounds.height().is_finite() {
        return None;
    }
    // Every loop's outline, with the tolerance that decides whether a tested vertex lies on it.
    let mut shapes: Vec<(Vec<Vec2>, f64)> = Vec::with_capacity(count);
    let mut total = 0usize;
    for l in loops {
        let vertices: Vec<PolyVertex> = l.vertices.iter().copied().filter(|v| v.p.is_finite()).collect();
        if vertices.len() < 2 {
            return None;
        }
        // A coarse tolerance is enough to place arcs for a containment test, and keeps the point
        // count of a curved boundary bounded. It follows the loop's own size so a small island in
        // a large drawing is not flattened to a chord.
        let lb = Bounds2::from_points(vertices.iter().map(|v| v.p));
        let tol = (lb.width().max(lb.height()) * 1e-4).max(1e-9);
        let points = Polyline { vertices, closed: true }.tessellate(tol);
        if points.len() < 3 {
            return None;
        }
        total = total.saturating_add(points.len());
        if total > MAX_POINTS {
            return None;
        }
        shapes.push((points, tol));
    }
    // Each sampled vertex costs up to two passes over the enclosing loop: the on-boundary test and
    // the point-in-polygon test.
    if total.saturating_mul(count).saturating_mul(PROBES).saturating_mul(2) > MAX_WORK as usize {
        return None;
    }
    let mut depths = Vec::with_capacity(count);
    for (i, (points, _)) in shapes.iter().enumerate() {
        let depth = shapes.iter().enumerate().filter(|(j, (outer, tol))| *j != i && encloses(outer, *tol, points)).count();
        depths.push(depth);
    }
    Some(depths)
}

/// Whether `outer` encloses `inner`: a majority of `inner`'s sampled vertices lie strictly inside
/// `outer`. Vertices within `tol` of `outer`'s edges sit on a shared edge and are skipped, so an
/// island that touches the outer boundary is still enclosed by it.
fn encloses(outer: &[Vec2], tol: f64, inner: &[Vec2]) -> bool {
    if outer.len() < 3 || inner.len() < 3 {
        return false;
    }
    let step = inner.len().div_ceil(PROBES).max(1);
    let mut inside = 0usize;
    let mut tested = 0usize;
    for p in inner.iter().step_by(step).take(PROBES) {
        if on_boundary(outer, *p, tol) {
            continue;
        }
        tested += 1;
        if point_in_polygon(outer, *p) {
            inside += 1;
            // More than half of the probes are inside, so the rest cannot change the answer.
            if inside * 2 > PROBES {
                return true;
            }
        }
    }
    tested > 0 && inside * 2 > tested
}

/// Whether `p` lies on one of `poly`'s edges, within `tol`.
fn on_boundary(poly: &[Vec2], p: Vec2, tol: f64) -> bool {
    let n = poly.len();
    (0..n).any(|i| match (poly.get(i), poly.get((i + 1) % n)) {
        (Some(&a), Some(&b)) => Line::new(a, b).dist(p) <= tol,
        _ => false,
    })
}
