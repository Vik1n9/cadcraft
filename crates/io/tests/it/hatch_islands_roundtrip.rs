//! HATCH islands (inner boundary loops) survive a DXF and a DWG save: every loop, its order and
//! its boundary-path flags (group 92), the hatch style (group 75), and the pattern name, scale and
//! angle (groups 2, 41, 52). DWG goes through the same code path as the app's Save: the CADCraft
//! DXF writer, the DWG bridge (acadrust), and the CADCraft DXF reader.

use cadcraft_doc::*;
use cadcraft_geom::{PolyVertex, Vec2};
use cadcraft_io::{read_dxf, write_dxf};

fn vert(x: f64, y: f64) -> PolyVertex {
    PolyVertex::new(Vec2::new(x, y))
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> HatchLoop {
    HatchLoop { vertices: vec![vert(x0, y0), vert(x1, y0), vert(x1, y1), vert(x0, y1)], outer: false, sources: Vec::new() }
}

fn square(lo: f64, hi: f64) -> HatchLoop {
    rect(lo, lo, hi, hi)
}

fn hatch(pattern: &str, loops: Vec<HatchLoop>, style: u8, associative: bool) -> Hatch {
    Hatch {
        pattern: pattern.into(),
        solid: false,
        loops,
        scale: 2.5,
        angle: 30f64.to_radians(),
        associative,
        style,
        elevation: 0.0,
        gradient: None,
        origin: Vec2::ZERO,
        background: None,
        pattern_lines: Vec::new(),
    }
}

/// The file formats a drawing is saved to and read back from. DWG is native-only.
fn roundtrips(d: &Drawing) -> Vec<(&'static str, Drawing)> {
    let mut out = vec![("dxf", read_dxf(write_dxf(d).as_bytes()).expect("read dxf"))];
    #[cfg(not(target_arch = "wasm32"))]
    out.push(("dwg", cadcraft_io::read(&cadcraft_io::write(d, "x.dwg").expect("write dwg"), "x.dwg").expect("read dwg")));
    out
}

fn hatches(d: &Drawing) -> Vec<Hatch> {
    d.model.iter().filter_map(|e| if let EntityKind::Hatch(h) = &e.kind { Some(h.clone()) } else { None }).collect()
}

fn points(l: &HatchLoop) -> Vec<(f64, f64)> {
    l.vertices.iter().map(|v| (v.p.x, v.p.y)).collect()
}

fn assert_loops(what: &str, h: &Hatch, expected: &[HatchLoop]) {
    assert_eq!(h.loops.len(), expected.len(), "{what}: loop count");
    for (i, (got, want)) in h.loops.iter().zip(expected).enumerate() {
        assert_eq!(points(got), points(want), "{what}: loop {i} vertices and order");
        assert_eq!(got.outer, want.outer, "{what}: loop {i} outer flag (group 92)");
    }
}

/// (group code, value) pairs of a DXF text.
fn pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    lines.chunks(2).filter_map(|c| Some((c.first()?.trim().parse().ok()?, c.get(1)?.trim().to_string()))).collect()
}

fn join(p: &[(i32, String)]) -> String {
    p.iter().map(|(c, v)| format!("{c}\n{v}\n")).collect()
}

/// The group-92 values of the first HATCH record: each loop's boundary path type flags.
fn loop_flags(p: &[(i32, String)]) -> Vec<String> {
    let start = p.iter().position(|(c, v)| *c == 0 && v == "HATCH").expect("HATCH");
    let end = p.iter().skip(start + 1).position(|(c, v)| *c == 0 && *v != "HATCH").map_or(p.len(), |k| start + 1 + k);
    let rec = &p[start..end];
    let loops = rec.iter().position(|(c, _)| *c == 91).and_then(|k| rec.get(k + 1)).and_then(|(_, n)| n.parse::<usize>().ok()).unwrap_or(0);
    let mut out: Vec<String> = rec.iter().filter(|(c, _)| *c == 92).map(|(_, v)| v.clone()).collect();
    out.truncate(loops);
    out
}

/// The same DXF with the first HATCH record's group-92 values replaced, as a file from another
/// writer would hold them.
fn with_loop_flags(d: &Drawing, flags: &[&str]) -> String {
    let mut p = pairs(&write_dxf(d));
    let start = p.iter().position(|(c, v)| *c == 0 && v == "HATCH").expect("HATCH");
    let end = p.iter().skip(start + 1).position(|(c, v)| *c == 0 && *v != "HATCH").map_or(p.len(), |k| start + 1 + k);
    let mut seen = 0;
    for (c, v) in &mut p[start..end] {
        if *c == 92 {
            if let Some(f) = flags.get(seen) {
                *v = (*f).to_string();
            }
            seen += 1;
        }
    }
    assert_eq!(seen, flags.len(), "loop count");
    join(&p)
}

/// Mid points of the pattern-line runs a hatch draws.
fn pattern_line_mids(d: &Drawing) -> Vec<Vec2> {
    let l = cadcraft_render::build(d, &Space::Model, &cadcraft_render::Options::default());
    l.prims
        .iter()
        .filter(|p| p.kind == cadcraft_render::Kind::Polyline)
        .filter_map(|p| {
            let pts = l.points(p);
            Some(pts.first()?.mid(*pts.last()?))
        })
        .collect()
}

fn inside_island(p: &Vec2) -> bool {
    p.x > 5.0 && p.x < 15.0 && p.y > 5.0 && p.y < 15.0
}

/// Feature 1: a pattern hatch with one island, style Normal.
#[test]
fn pattern_hatch_keeps_its_island_and_style() {
    let mut d = Drawing::new_imperial();
    let mut outer = square(0.0, 20.0);
    outer.outer = true;
    let loops = vec![outer, square(5.0, 15.0)];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops.clone(), 0, false))).expect("add");
    // External boundary = 1 (external) | 2 (polyline); the island = 16 (outermost) | 2.
    let p = pairs(&write_dxf(&d));
    assert_eq!(loop_flags(&p), vec!["3".to_string(), "18".to_string()], "written boundary path flags");
    for (what, back) in roundtrips(&d) {
        let hs = hatches(&back);
        assert_eq!(hs.len(), 1, "{what}");
        let h = &hs[0];
        assert_loops(what, h, &loops);
        assert_eq!(h.style, 0, "{what}: style (group 75)");
        assert_eq!(h.pattern, "ANSI31", "{what}: pattern name");
        assert!((h.scale - 2.5).abs() < 1e-9, "{what}: scale {0}", h.scale);
        assert!((h.angle - 30f64.to_radians()).abs() < 1e-9, "{what}: angle {0}", h.angle);
        // The island stays unfilled when the pattern is drawn from the re-read hatch.
        let mids = pattern_line_mids(&back);
        assert!(!mids.is_empty(), "{what}: pattern lines drawn");
        assert!(!mids.iter().any(inside_island), "{what}: the island must stay unfilled");
        assert!(mids.iter().any(|p| !inside_island(p)), "{what}: the rest is filled");
    }
}

/// Feature 2: Outer (75 = 1) and Ignore (75 = 2) keep their style.
#[test]
fn outer_and_ignore_styles_are_kept() {
    let mut d = Drawing::new_imperial();
    for (style, name) in [(1u8, "OUTER"), (2, "IGNORE")] {
        let mut outer = square(0.0, 20.0);
        outer.outer = true;
        let loops = vec![outer, square(5.0, 15.0)];
        let mut h = hatch("ANSI31", loops, style, false);
        h.pattern = name.into();
        d.add(&Space::Model, Common::default(), EntityKind::Hatch(h)).expect("add");
    }
    for (what, back) in roundtrips(&d) {
        let styles: Vec<(String, u8)> = hatches(&back).into_iter().map(|h| (h.pattern, h.style)).collect();
        assert_eq!(styles, vec![("OUTER".to_string(), 1), ("IGNORE".to_string(), 2)], "{what}");
    }
}

/// Feature 3: an associative hatch keeps its source boundary objects (97/330) through DXF and
/// DWG. The loops themselves are also kept either way.
#[test]
fn associative_hatch_keeps_its_source_boundaries() {
    let mut d = Drawing::new_imperial();
    let poly = |d: &mut Drawing, x0: f64, y0: f64, x1: f64, y1: f64| {
        let l = rect(x0, y0, x1, y1);
        d.add(
            &Space::Model,
            Common::default(),
            EntityKind::LwPolyline(LwPolyline { vertices: l.vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false }),
        )
        .expect("add")
    };
    let outer_handle = poly(&mut d, 0.0, 0.0, 20.0, 20.0);
    let island_handle = poly(&mut d, 5.0, 5.0, 15.0, 15.0);
    let mut outer = square(0.0, 20.0);
    outer.outer = true;
    outer.sources = vec![outer_handle];
    let mut island = square(5.0, 15.0);
    island.sources = vec![island_handle];
    let loops = vec![outer, island];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops.clone(), 0, true))).expect("add");
    for (what, back) in roundtrips(&d) {
        let hs = hatches(&back);
        assert_eq!(hs.len(), 1, "{what}");
        let h = &hs[0];
        assert_loops(what, h, &loops);
        assert!(h.associative, "{what}: associative flag");
        let sources: Vec<Vec<Handle>> = h.loops.iter().map(|l| l.sources.clone()).collect();
        assert_eq!(sources, vec![vec![outer_handle], vec![island_handle]], "{what}: source boundaries");
    }
}

/// Feature 4: an island inside an island keeps all three loops, their order and their flags.
#[test]
fn nested_islands_keep_every_loop_in_order() {
    let mut d = Drawing::new_imperial();
    let mut outer = square(0.0, 30.0);
    outer.outer = true;
    let mut inner_solid = square(10.0, 20.0);
    inner_solid.outer = true;
    let loops = vec![outer, square(5.0, 25.0), inner_solid];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops.clone(), 0, false))).expect("add");
    // External, first island (outermost), island-in-an-island (a deeper default tier).
    let p = pairs(&write_dxf(&d));
    assert_eq!(loop_flags(&p), vec!["3".to_string(), "18".to_string(), "2".to_string()], "written boundary path flags");
    // A default-type loop (group 92 = 2) has no external bit, so the deepest loop reads back
    // not-outer; CADCraft draws the fill from the nesting depth, not from this bit.
    let mut want = loops.clone();
    want[2].outer = false;
    for (what, back) in roundtrips(&d) {
        let hs = hatches(&back);
        assert_eq!(hs.len(), 1, "{what}");
        assert_loops(what, &hs[0], &want);
    }
}

/// A file whose loops carry no tier bit (the 2/3 pattern older CADCraft saves wrote): the reader
/// derives the fill parity from the geometry, and a save restores the tier flags.
#[test]
fn deeper_islands_without_a_tier_bit_keep_their_fill_parity() {
    let mut d = Drawing::new_imperial();
    let mut outer = square(0.0, 30.0);
    outer.outer = true;
    let mut inner_solid = square(10.0, 20.0);
    inner_solid.outer = true;
    let loops = vec![outer, square(5.0, 25.0), inner_solid];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops.clone(), 0, false))).expect("add");
    let back = read_dxf(with_loop_flags(&d, &["3", "2", "2"]).as_bytes()).expect("read");
    assert_eq!(hatches(&back)[0].loops.iter().map(|l| l.outer).collect::<Vec<_>>(), [true, false, true]);
    assert_eq!(loop_flags(&pairs(&write_dxf(&back))), vec!["3".to_string(), "18".to_string(), "2".to_string()]);
}

/// An AutoCAD-style file keeps its flags-derived fill parity even when the geometry disagrees: two
/// disjoint squares, the second tagged as the outermost island tier (group 92 = 18). The reader
/// must not replace that with the geometric depth, which is 0 for both.
#[test]
fn autocad_tier_flags_win_over_geometry_on_read() {
    let mut d = Drawing::new_imperial();
    let mut first = rect(0.0, 0.0, 10.0, 10.0);
    first.outer = true;
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", vec![first, rect(20.0, 0.0, 30.0, 10.0)], 0, false))).expect("add");
    let back = read_dxf(with_loop_flags(&d, &["3", "18"]).as_bytes()).expect("read");
    assert_eq!(hatches(&back)[0].loops.iter().map(|l| l.outer).collect::<Vec<_>>(), [true, false]);
}

/// An AutoCAD-style deeper tier (group 92 = 2) keeps the parity its flags describe: the
/// island-in-an-island stays not-outer even though the geometric depth would make it outer.
#[test]
fn autocad_deeper_tier_keeps_the_files_parity() {
    let mut d = Drawing::new_imperial();
    let mut outer = square(0.0, 30.0);
    outer.outer = true;
    let loops = vec![outer, square(5.0, 25.0), square(10.0, 20.0)];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops, 0, false))).expect("add");
    let back = read_dxf(with_loop_flags(&d, &["3", "18", "2"]).as_bytes()).expect("read");
    assert_eq!(hatches(&back)[0].loops.iter().map(|l| l.outer).collect::<Vec<_>>(), [true, false, false]);
}

/// An island that shares one edge with the outer boundary is still an island. Its first edge lies
/// exactly on the outer loop, so a probe taken from that edge cannot decide containment.
#[test]
fn island_sharing_an_edge_with_the_outer_loop_stays_an_island() {
    let mut d = Drawing::new_imperial();
    let mut outer = square(0.0, 20.0);
    outer.outer = true;
    // The island's first edge, (5,20)-(15,20), lies on the outer loop's top edge.
    let island =
        HatchLoop { vertices: vec![vert(5.0, 20.0), vert(15.0, 20.0), vert(15.0, 10.0), vert(5.0, 10.0)], outer: false, sources: Vec::new() };
    let loops = vec![outer, island];
    d.add(&Space::Model, Common::default(), EntityKind::Hatch(hatch("ANSI31", loops.clone(), 0, false))).expect("add");
    // External, then the island (outermost | polyline).
    let p = pairs(&write_dxf(&d));
    assert_eq!(loop_flags(&p), vec!["3".to_string(), "18".to_string()], "written boundary path flags");
    for (what, back) in roundtrips(&d) {
        let hs = hatches(&back);
        assert_eq!(hs.len(), 1, "{what}");
        assert_loops(what, &hs[0], &loops);
    }
}
