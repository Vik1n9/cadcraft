//! Paper-space layouts survive saving to DWG (and DXF): each keeps its own entities and
//! viewports (view centre, height and scale), per-viewport frozen layers, the header linetype
//! scales and its own LAYOUT flags, and every paper-space block record stays linked to its own
//! LAYOUT. Lose one of those and AutoCAD opens blank layouts, fails to switch between them or
//! drops the per-viewport layer freeze (the field report behind this test).

use std::io::Cursor;

use cadcraft_doc::*;
use cadcraft_geom::{Vec2, Vec3};
use cadcraft_io::{SaveOptions, Version, read, write_with};

/// (name, tab order, LAYOUT group 70 flags): PSLTSCALE on, PSLTSCALE off, LIMCHECK on.
const LAYOUTS: [(&str, u32, u32); 3] = [("East Wing", 1, 1), ("West Wing", 2, 0), ("North Wing", 3, 2)];

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn sheet(name: &str) -> Space {
    Space::Paper(name.into())
}

fn text(value: &str, at: Vec3) -> EntityKind {
    EntityKind::Text(Text {
        insert: at,
        align_pt: None,
        height: 2.5,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Left,
        valign: VAlign::Baseline,
    })
}

fn viewport(id: u32, center: Vec2, view_height: f64, frozen: Vec<String>) -> EntityKind {
    EntityKind::Viewport(Viewport {
        center: Vec3::new(200.0, 150.0, 0.0),
        width: 380.0,
        height: 280.0,
        view_center: center,
        view_height,
        id,
        locked: true,
        frozen_layers: frozen,
        ..Default::default()
    })
}

/// Three paper-space layouts, each with its own note, line, sheet viewport (id 1) and model
/// viewport (id 2, its own view), one of them freezing two layers in its model viewport.
fn synthetic() -> Drawing {
    let mut d = Drawing::new_metric();
    d.layers.push(Layer { name: "Walls".into(), ..Layer::default() });
    d.layers.push(Layer { name: "Hidden".into(), ..Layer::default() });
    d.header.set_i64("PSLTSCALE", 1);
    d.header.set_f64("CELTSCALE", 1.0);
    d.header.set_f64("LTSCALE", 1.0);
    d.layouts = LAYOUTS.iter().map(|(name, tab, flags)| Layout { flags: *flags, ..Layout::new(name, *tab) }).collect();
    let views = [(Vec2::new(10.0, 20.0), 50.0), (Vec2::new(100.0, 200.0), 80.0), (Vec2::new(-5.0, 7.0), 25.0)];
    for (i, (name, _, _)) in LAYOUTS.iter().enumerate() {
        let (center, height) = views[i];
        let frozen = if i == 0 { vec!["Walls".to_string(), "Hidden".to_string()] } else { Vec::new() };
        d.add(&sheet(name), Common::default(), text(&format!("{name} note"), Vec3::new(10.0, 10.0, 0.0))).unwrap();
        d.add(
            &sheet(name),
            Common::default(),
            EntityKind::Line(Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(10.0 * (i as f64 + 1.0), 5.0, 0.0) }),
        )
        .unwrap();
        d.add(&sheet(name), Common::default(), viewport(1, Vec2::ZERO, 280.0, Vec::new())).unwrap();
        d.add(&sheet(name), Common::default(), viewport(2, center, height, frozen)).unwrap();
    }
    d
}

fn dwg(d: &Drawing) -> Vec<u8> {
    write_with(d, "layouts.dwg", &SaveOptions { version: Some(Version::R2018), binary: false }).expect("write the DWG")
}

fn dxf(d: &Drawing) -> Vec<u8> {
    write_with(d, "layouts.dxf", &SaveOptions { version: Some(Version::R2018), binary: false }).expect("write the DXF")
}

/// The viewports of one layout: (id, view centre, view height, sheet width, sheet height, frozen).
fn viewports(d: &Drawing, name: &str) -> Vec<(u32, Vec2, f64, f64, f64, Vec<String>)> {
    let l = d.layouts.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("layout {name}"));
    let mut out: Vec<_> = l
        .entities
        .iter()
        .filter_map(|e| match &e.kind {
            EntityKind::Viewport(v) => {
                let mut frozen = v.frozen_layers.clone();
                frozen.sort();
                Some((v.id, v.view_center, v.view_height, v.width, v.height, frozen))
            }
            _ => None,
        })
        .collect();
    out.sort_by_key(|v| v.0);
    out
}

/// The paper-space notes and lines of one layout, in a stable order.
fn notes(d: &Drawing, name: &str) -> (Vec<String>, Vec<(Vec2, Vec2)>) {
    let l = d.layouts.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("layout {name}"));
    let mut texts = Vec::new();
    let mut lines = Vec::new();
    for e in l.entities.iter() {
        match &e.kind {
            EntityKind::Text(t) => texts.push(t.value.clone()),
            EntityKind::Line(l) => lines.push((l.a.xy(), l.b.xy())),
            _ => {}
        }
    }
    texts.sort();
    lines.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    (texts, lines)
}

fn check_layouts(back: &Drawing, where_: &str) {
    let mut tabs: Vec<(u32, &str)> = back.layouts.iter().map(|l| (l.tab_order, l.name.as_str())).collect();
    tabs.sort();
    assert_eq!(tabs.iter().map(|t| t.1).collect::<Vec<_>>(), ["East Wing", "West Wing", "North Wing"], "{where_}: tab order/names");
    let expected_views = [(Vec2::new(10.0, 20.0), 50.0), (Vec2::new(100.0, 200.0), 80.0), (Vec2::new(-5.0, 7.0), 25.0)];
    for (i, (name, _, _)) in LAYOUTS.iter().enumerate() {
        let (texts, lines) = notes(back, name);
        assert_eq!(texts, [format!("{name} note")], "{where_}: {name} keeps exactly its own notes (none moved, duplicated or lost)");
        assert_eq!(lines.len(), 1, "{where_}: {name} keeps its line");
        assert!(close(lines[0].1.x, 10.0 * (i as f64 + 1.0)), "{where_}: {name}'s own line ({lines:?})");
        let vps = viewports(back, name);
        assert_eq!(vps.len(), 2, "{where_}: {name} keeps exactly its two viewports ({vps:?})");
        let (sheet, model) = (&vps[0], &vps[1]);
        assert_eq!(sheet.0, 1, "{where_}: {name}'s sheet viewport is id 1, first in file order");
        assert_eq!(model.0, 2, "{where_}: {name}'s model viewport is id 2");
        assert!(close(model.1.x, expected_views[i].0.x) && close(model.1.y, expected_views[i].0.y), "{where_}: {name} keeps its view centre");
        assert!(close(model.2, expected_views[i].1), "{where_}: {name} keeps its view height");
        assert!(close(model.3, 380.0) && close(model.4, 280.0), "{where_}: {name} keeps its viewport size");
    }
    let frozen = viewports(back, "East Wing")[1].5.clone();
    assert_eq!(frozen, ["Hidden", "Walls"], "{where_}: per-viewport frozen layers are kept");
    assert!(viewports(back, "West Wing")[1].5.is_empty(), "{where_}: West Wing's viewport freezes nothing");
}

fn check_header(d: &Drawing, celt_scale: f64, where_: &str) {
    assert_eq!(d.header.i64("PSLTSCALE", -1), 1, "{where_}: $PSLTSCALE");
    assert!(close(d.header.f64("CELTSCALE", -1.0), celt_scale), "{where_}: $CELTSCALE");
    assert!(close(d.header.f64("LTSCALE", -1.0), 1.0), "{where_}: $LTSCALE");
}

fn check_flags(d: &Drawing, where_: &str) {
    for (name, _, flags) in LAYOUTS {
        let l = d.layouts.iter().find(|l| l.name == name).unwrap_or_else(|| panic!("layout {name}"));
        assert_eq!(l.flags, flags, "{where_}: {name}'s LAYOUT group 70 flags (PSLTSCALE/LIMCHECK)");
    }
}

#[test]
fn dxf_roundtrip_keeps_layouts() {
    let d = synthetic();
    let back = read(&dxf(&d), "layouts.dxf").unwrap();
    check_layouts(&back, "DXF");
    check_header(&back, 1.0, "DXF");
    check_flags(&back, "DXF");
}

#[test]
fn dwg_roundtrip_keeps_layouts() {
    let d = synthetic();
    let back = read(&dwg(&d), "layouts.dwg").unwrap();
    check_layouts(&back, "DWG");
    check_header(&back, 1.0, "DWG");
    check_flags(&back, "DWG");
}

#[test]
fn non_default_celt_scale_survives() {
    let mut d = synthetic();
    d.header.set_f64("CELTSCALE", 0.5);
    let dxf_back = read(&dxf(&d), "layouts.dxf").unwrap();
    check_header(&dxf_back, 0.5, "DXF");
    let dwg_back = read(&dwg(&d), "layouts.dwg").unwrap();
    check_header(&dwg_back, 0.5, "DWG");
}

// ---------------------------------------------------------------- handle links

/// The group values of a record: every `(code, value)` after its `0` group.
fn records(text: &str) -> Vec<(String, Vec<(i32, String)>)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut current: Option<(String, Vec<(i32, String)>)> = None;
    for pair in lines.chunks(2) {
        let (Some(code), Some(value)) = (pair.first(), pair.get(1)) else { continue };
        let Ok(code) = code.trim().parse::<i32>() else { continue };
        let value = value.trim().to_string();
        if code == 0 {
            if let Some(r) = current.take() {
                out.push(r);
            }
            current = Some((value, Vec::new()));
        } else if let Some((_, tags)) = current.as_mut() {
            tags.push((code, value));
        }
    }
    if let Some(r) = current {
        out.push(r);
    }
    out
}

fn value(tags: &[(i32, String)], code: i32) -> Option<&str> {
    tags.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
}

fn last_value(tags: &[(i32, String)], code: i32) -> Option<&str> {
    tags.iter().rev().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
}

/// `(handle, name, block-record handle, tab order, flags)` of every paper LAYOUT.
fn layouts_in(text: &str) -> Vec<(String, String, String, i32, i32)> {
    let mut out = Vec::new();
    for (_, tags) in records(text).iter().filter(|(k, _)| k == "LAYOUT") {
        let Some(acdb_layout) = tags.iter().position(|(c, v)| *c == 100 && v == "AcDbLayout") else { continue };
        let tail = &tags[acdb_layout..];
        let name = value(tail, 1).unwrap_or_default().to_string();
        if name == "Model" {
            continue;
        }
        let handle = value(tags, 5).unwrap_or_default().to_string();
        let br = last_value(&tags[acdb_layout..], 330).unwrap_or_default().to_string();
        let tab = value(tail, 71).and_then(|v| v.parse().ok()).unwrap_or(0);
        let flags = value(tail, 70).and_then(|v| v.parse().ok()).unwrap_or(0);
        out.push((handle, name, br, tab, flags));
    }
    out
}

/// `(handle, name, layout handle)` of every paper-space BLOCK_RECORD.
fn paper_block_records(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (_, tags) in records(text).iter().filter(|(k, _)| k == "BLOCK_RECORD") {
        let name = value(tags, 2).unwrap_or_default().to_string();
        if !name.to_ascii_uppercase().starts_with("*PAPER_SPACE") {
            continue;
        }
        out.push((value(tags, 5).unwrap_or_default().to_string(), name, value(tags, 340).unwrap_or_default().to_string()));
    }
    out
}

/// The layout dictionary's `(name, layout handle)` pairs.
fn layout_dictionary(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (_kind, tags) in records(text).iter().filter(|(k, _)| k == "DICTIONARY") {
        let mut pending: Option<String> = None;
        for (code, v) in tags {
            match *code {
                3 => pending = Some(v.clone()),
                350 => {
                    if let Some(name) = pending.take() {
                        out.push((name, v.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn assert_links_consistent(text: &str, where_: &str) {
    let layouts = layouts_in(text);
    let blocks = paper_block_records(text);
    let dict = layout_dictionary(text);
    assert_eq!(layouts.len(), LAYOUTS.len(), "{where_}: one LAYOUT per paper layout ({layouts:?})");
    assert_eq!(blocks.len(), LAYOUTS.len(), "{where_}: one paper-space block record per layout ({blocks:?})");

    let mut block_to_layout = std::collections::HashMap::new();
    for (bh, bname, lh) in &blocks {
        assert!(!lh.is_empty() && lh != "0", "{where_}: block record {bname} links to its LAYOUT (340)");
        let prior = block_to_layout.insert(bh.clone(), lh.clone());
        assert!(prior.is_none(), "{where_}: block record {bname} has a unique handle");
    }
    let mut layout_to_block = std::collections::HashMap::new();
    for (lh, name, br, tab, _) in &layouts {
        if let Some(linked) = layout_to_block.insert(lh.clone(), br.clone()) {
            panic!("{where_}: LAYOUT {name} has a unique handle (already {linked})");
        }
        assert!(block_to_layout.contains_key(br), "{where_}: LAYOUT {name}'s 330 {br} is a paper-space block record");
        assert_eq!(block_to_layout.get(br), Some(lh), "{where_}: block record {br} points back at LAYOUT {name}");
        assert!(*tab > 0, "{where_}: LAYOUT {name} keeps its tab order");
    }
    // Each block record belongs to exactly one LAYOUT.
    let mut seen = std::collections::HashSet::new();
    for lh in block_to_layout.values() {
        assert!(seen.insert(lh.clone()), "{where_}: no two block records share one LAYOUT");
    }
    // The ACAD_LAYOUT dictionary names each layout and points at its LAYOUT object.
    for (lh, name, _, _, _) in &layouts {
        let entry = dict.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("{where_}: layout dictionary names {name} ({dict:?})"));
        assert_eq!(&entry.1, lh, "{where_}: layout dictionary's {name} points at its LAYOUT");
    }
    // The names the block records of the layouts found: *Paper_Space, *Paper_Space0, ...
    let mut names: Vec<String> = blocks.iter().map(|b| b.1.to_ascii_uppercase()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), LAYOUTS.len(), "{where_}: paper-space block records are distinct ({names:?})");
}

#[test]
fn written_dxf_links_layouts_and_block_records() {
    let d = synthetic();
    let text = String::from_utf8(dxf(&d)).unwrap();
    assert_links_consistent(&text, "CADCraft DXF");
}

#[test]
fn dwg_links_layouts_and_block_records() {
    let d = synthetic();
    let doc = acadrust::DwgReader::from_stream(Cursor::new(dwg(&d))).read().expect("read the DWG back with acadrust");
    let text = String::from_utf8(acadrust::DxfWriter::new(&doc).write_to_vec().expect("write the DXF acadrust produces")).unwrap();
    assert_links_consistent(&text, "DXF from the DWG");
    // Flags as the DWG carries them, read with acadrust's own reader.
    for (name, _, flags) in LAYOUTS {
        let layout = doc
            .objects
            .iter()
            .find_map(|(_, o)| match o {
                acadrust::objects::ObjectType::Layout(l) if l.name == name => Some(l),
                _ => None,
            })
            .unwrap_or_else(|| panic!("LAYOUT {name} in the DWG"));
        assert_eq!(layout.flags as u32, flags, "DXF from the DWG: {name}'s LAYOUT group 70 flags");
    }
}
