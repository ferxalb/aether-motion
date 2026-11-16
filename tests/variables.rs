//! Composition variables: declarations, passed values, CSS pinning, and the `data-var-*` bindings.

mod common;

use aether_motion::model::{Composition, Content, VariableKind};
use aether_motion::parse::{parse_file_with, values_from_json, VariableValues};
use aether_motion::render::Renderer;
use common::scratch;

const DECLARATIONS: &str = r##"[
  {"id":"title","type":"string","label":"Title","default":"Launch day","maxLength":40},
  {"id":"accent","type":"color","label":"Accent","default":"#ff0000"},
  {"id":"size","type":"number","label":"Box size","default":40,"min":10,"max":90,"unit":"px"},
  {"id":"badge","type":"boolean","label":"Show badge","default":true},
  {"id":"layout","type":"enum","label":"Layout","default":"left","options":[{"value":"left","label":"Left"},"right"]},
  {"id":"logo","type":"image","label":"Logo","default":"logo-a.png"}
]"##;

const CLIP: &str = r#"class="clip" data-start="0" data-duration="2""#;

fn write(name: &str, html: &str) -> std::path::PathBuf {
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

/// A 100x100 composition declaring `DECLARATIONS` on its root.
fn page(name: &str, head: &str, body: &str) -> std::path::PathBuf {
    write(
        name,
        &format!(
            r#"<html><head><style>{head}</style></head><body><div id="root" data-composition-id="v" data-width="100" data-height="100" data-duration="2" style="background:#000000" data-composition-variables='{DECLARATIONS}'>{body}</div></body></html>"#
        ),
    )
}

fn values(json: &str) -> VariableValues {
    values_from_json(json).unwrap()
}

fn parse(path: &std::path::Path, json: &str) -> Composition {
    parse_file_with(path, &values(json)).unwrap()
}

fn error(path: &std::path::Path, json: &str) -> String {
    format!("{:#}", parse_file_with(path, &values(json)).unwrap_err())
}

fn text_of(c: &Composition, id: &str) -> String {
    let element = &c.elements[c.ids[id]];
    match &element.content {
        Content::Text(text) => text.clone(),
        other => panic!("{id} is not text: {other:?}"),
    }
}

fn pixel(c: &Composition, x: usize, y: usize) -> [u8; 3] {
    let mut renderer = Renderer::new(c, 1.0).unwrap();
    let frame = renderer.render_at(0.0).unwrap();
    let i = (y * 100 + x) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

#[test]
fn defaults_and_passed_values_reach_text_css_and_sources() {
    let body = format!(
        r#"<div id="t" {CLIP} data-var-text="title" style="color:var(--accent); font-size:10px">placeholder</div>
        <div id="box" {CLIP} style="position:absolute; left:0px; top:0px; width:var(--size); height:var(--size); background:var(--accent)"></div>
        <img id="logo" {CLIP} data-var-src="logo" src="fallback.png" style="width:10px; height:10px">"#
    );
    let path = page("vars_basic", "", &body);
    for name in ["logo-a.png", "logo-b.png", "fallback.png"] {
        image::RgbaImage::new(10, 10)
            .save(path.with_file_name(name))
            .unwrap();
    }
    let c = parse(&path, "{}");
    assert_eq!(text_of(&c, "t"), "Launch day");
    assert_eq!(pixel(&c, 39, 39), [255, 0, 0], "the default 40px box");
    assert_eq!(pixel(&c, 41, 41), [0, 0, 0]);
    let Content::Image(src) = &c.elements[c.ids["logo"]].content else {
        panic!("logo is an image");
    };
    assert!(src.ends_with("logo-a.png"));
    assert!(c.variables.declared.iter().all(|v| !v.overridden));

    let c = parse(
        &path,
        r##"{"title":"Q4 report","accent":"#00ff00","size":60,"logo":{"url":"logo-b.png"}}"##,
    );
    assert_eq!(text_of(&c, "t"), "Q4 report");
    assert_eq!(pixel(&c, 59, 59), [0, 255, 0]);
    assert_eq!(pixel(&c, 61, 61), [0, 0, 0]);
    let Content::Image(src) = &c.elements[c.ids["logo"]].content else {
        panic!("logo is an image");
    };
    assert!(src.ends_with("logo-b.png"));
    let size = c
        .variables
        .declared
        .iter()
        .find(|v| v.id == "size")
        .unwrap();
    assert_eq!(
        (size.kind, size.value.as_str(), size.default.as_str()),
        (VariableKind::Number, "60", "40")
    );
    assert!(size.overridden);
    assert_eq!(size.uses.css, 2);
    let accent = c
        .variables
        .declared
        .iter()
        .find(|v| v.id == "accent")
        .unwrap();
    assert_eq!((accent.uses.css, accent.uses.text), (2, 0));

    // An empty source keeps the authored `src` as the fallback.
    let c = parse(&path, r#"{"logo":""}"#);
    let Content::Image(src) = &c.elements[c.ids["logo"]].content else {
        panic!("logo is an image");
    };
    assert!(src.ends_with("fallback.png"));
}

#[test]
fn variables_win_on_the_root_but_descendants_may_redefine() {
    let head = ":root { --accent: #0000ff } #root { --accent: #0000ff }";
    let body = format!(
        r#"<div id="a" {CLIP} style="position:absolute; left:0px; top:0px; width:50px; height:50px; background:var(--accent)"></div>
        <div id="b" {CLIP} style="--accent:#ffffff; position:absolute; left:50px; top:0px; width:50px; height:50px; background:var(--accent)"></div>"#
    );
    let path = page("vars_pinned", head, &body);
    let c = parse(&path, r##"{"accent":"#00ff00"}"##);
    assert_eq!(
        pixel(&c, 10, 10),
        [0, 255, 0],
        "the variable beats :root and #root"
    );
    assert_eq!(
        pixel(&c, 60, 10),
        [255, 255, 255],
        "a descendant's own value wins"
    );
}

#[test]
fn conditions_remove_elements_and_stylesheets() {
    let head = ".panel { background: #ffffff }";
    let body = format!(
        r#"<style data-var-if="layout=right">#panel {{ background: #00ff00 }}</style>
        <div id="panel" class="clip panel" data-start="0" data-duration="2" style="position:absolute; left:0px; top:0px; width:50px; height:50px"></div>
        <div id="badge" {CLIP} data-var-if="badge" style="position:absolute; left:0px; top:60px; width:10px; height:10px; background:#00ff00"><span id="inner">x</span></div>
        <div id="nobadge" {CLIP} data-var-if="!badge" style="font-size:10px">none</div>
        <div id="numbered" {CLIP} data-var-if="size!=40" style="font-size:10px">resized</div>"#
    );
    let path = page("vars_if", head, &body);
    let c = parse(&path, "{}");
    assert!(c.ids.contains_key("badge") && c.ids.contains_key("inner"));
    assert!(!c.ids.contains_key("nobadge") && !c.ids.contains_key("numbered"));
    assert_eq!(c.variables.removed, 3, "the style sheet and two elements");
    assert_eq!(pixel(&c, 10, 10), [255, 255, 255]);

    let c = parse(&path, r#"{"badge":false,"layout":"right","size":40.0}"#);
    assert!(!c.ids.contains_key("badge") && !c.ids.contains_key("inner"));
    assert!(c.ids.contains_key("nobadge"));
    assert!(!c.ids.contains_key("numbered"), "40.0 equals 40");
    assert_eq!(c.variables.removed, 2, "a removed subtree counts once");
    assert_eq!(
        pixel(&c, 10, 10),
        [0, 255, 0],
        "the right-layout sheet applies"
    );
    let badge = c
        .variables
        .declared
        .iter()
        .find(|v| v.id == "badge")
        .unwrap();
    assert_eq!(badge.uses.condition, 2);
}

#[test]
fn text_binding_keeps_child_elements() {
    let body = format!(
        r#"<div id="host" {CLIP} data-var-text="title" style="font-size:10px">old <span id="child">kept</span> tail</div>
        <div id="empty" {CLIP} data-var-text="size" style="font-size:10px"><span id="only">x</span></div>"#
    );
    let path = page("vars_text", "", &body);
    let c = parse(&path, r#"{"title":"New","size":12.5}"#);
    assert_eq!(text_of(&c, "host"), "New");
    assert_eq!(text_of(&c, "child"), "kept");
    assert_eq!(text_of(&c, "empty"), "12.5");
    assert_eq!(text_of(&c, "only"), "x");
}

#[test]
fn invalid_values_and_bindings_are_explicit_errors() {
    let plain = page("vars_err_values", "", &format!(r#"<div {CLIP}></div>"#));
    for (json, needle) in [
        (r#"{"titel":"x"}"#, "does not declare"),
        (r#"{"title":3}"#, "must be a string"),
        (r#"{"size":"60"}"#, "must be a number"),
        (r#"{"size":200}"#, "outside 10..90"),
        (r#"{"accent":"blue-ish"}"#, "not a color"),
        (r#"{"badge":"yes"}"#, "true or false"),
        (r#"{"layout":"center"}"#, "not one of its options"),
        (
            r#"{"title":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            "maxLength 40",
        ),
        (r#"{"logo":{"url":"a.png","fit":"cover"}}"#, "not supported"),
    ] {
        let message = error(&plain, json);
        assert!(message.contains(needle), "{json}: {message}");
    }
    for (name, body, needle) in [
        (
            "vars_err_undeclared",
            r#"<div data-var-text="missing">x</div>"#,
            "no variable \"missing\"",
        ),
        (
            "vars_err_src_tag",
            r#"<div data-var-src="logo"></div>"#,
            "only allowed on <img>",
        ),
        (
            "vars_err_src_kind",
            r#"<img data-var-src="size" src="a.png">"#,
            "has type number",
        ),
        (
            "vars_err_text_tag",
            r#"<img data-var-text="title" src="a.png">"#,
            "cannot set text",
        ),
        (
            "vars_err_flag",
            r#"<div data-var-if="layout">x</div>"#,
            "has type enum",
        ),
        (
            "vars_err_eq_bool",
            r#"<div data-var-if="badge=true">x</div>"#,
            "has type boolean",
        ),
        (
            "vars_err_enum_typo",
            r#"<div data-var-if="layout=rigth">x</div>"#,
            "not one of its options",
        ),
        (
            "vars_err_number",
            r#"<div data-var-if="size=big">x</div>"#,
            "compare it with a number",
        ),
        (
            "vars_err_unknown",
            r#"<div data-var-color="accent">x</div>"#,
            "not a variable binding",
        ),
        (
            "vars_err_nested",
            r#"<div data-variable-values='{}'>x</div>"#,
            "not supported",
        ),
        (
            "vars_err_place",
            r#"<div data-composition-variables='[]'>x</div>"#,
            "belongs on <html>",
        ),
    ] {
        let path = page(name, "", body);
        let message = error(&path, "{}");
        assert!(message.contains(needle), "{name}: {message}");
    }
    let root_if = write(
        "vars_err_root_if",
        &format!(
            r#"<div data-composition-id="v" data-width="100" data-height="100" data-duration="2" data-var-if="badge" data-composition-variables='{DECLARATIONS}'></div>"#
        ),
    );
    assert!(error(&root_if, "{}").contains("cannot remove the composition root"));
    let twice = write(
        "vars_err_twice",
        &format!(
            r#"<html data-composition-variables='[]'><body><div data-composition-id="v" data-width="100" data-height="100" data-duration="2" data-composition-variables='{DECLARATIONS}'></div></body></html>"#
        ),
    );
    assert!(error(&twice, "{}").contains("declare the variables once"));
    let none = write(
        "vars_err_none",
        r#"<div data-composition-id="v" data-width="100" data-height="100" data-duration="2"></div>"#,
    );
    assert!(error(&none, r#"{"title":"x"}"#).contains("(declared: none)"));
}

#[test]
fn declarations_on_html_apply_and_check_reports_them() {
    let path = write(
        "vars_html",
        &format!(
            r#"<html data-composition-variables='{DECLARATIONS}'><body><div id="root" data-composition-id="v" data-width="100" data-height="100" data-duration="2"><div id="t" {CLIP} data-var-text="title" style="font-size:10px">x</div></div></body></html>"#
        ),
    );
    let report = aether_motion::agent::check_with(&path, &values(r#"{"size":12}"#));
    assert_eq!(report["ok"], true, "{report}");
    let variables = &report["variables"];
    let declared = variables["declared"].as_array().unwrap();
    assert_eq!(declared.len(), 6);
    assert_eq!(declared[0]["id"], "title");
    assert_eq!(declared[0]["uses"]["text"], 1);
    assert_eq!(declared[2]["value"], 12.0);
    assert_eq!(declared[2]["default"], 40.0);
    assert_eq!(declared[2]["overridden"], true);
    assert_eq!(declared[3]["value"], true);
    assert_eq!(
        variables["unused"],
        serde_json::json!(["accent", "size", "badge", "layout", "logo"])
    );
    let failed = aether_motion::agent::check_with(&path, &values(r#"{"nope":1}"#));
    assert_eq!(failed["ok"], false);
    assert!(failed["fix"]
        .as_str()
        .unwrap()
        .contains("data-composition-variables"));
}
