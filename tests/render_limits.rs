//! Rendering rejects extreme layer coordinates and shadow allocations with controlled errors.

mod common;

use aether_motion::{parse::parse_file, render::Renderer};

#[test]
fn extreme_layer_and_shadow_sizes_return_errors() {
    for (name, style, expected) in [
        (
            "huge_rotation",
            "width:1000000000px;height:1000000000px;transform:rotate(1deg)",
            "128 MiB",
        ),
        (
            "far_rotation",
            "left:3000000000px;width:16px;height:16px;transform:rotate(1deg)",
            "pixel coordinate",
        ),
        (
            "far_filter",
            "left:3000000000px;width:16px;height:16px;filter:blur(8px)",
            "pixel coordinate",
        ),
        (
            "huge_shadow",
            "width:16px;height:16px;box-shadow:0 0 1000000000px white",
            "profile exceeds",
        ),
        (
            "huge_inset",
            "width:16px;height:16px;box-shadow:inset 0 0 1000000000px white",
            "profile exceeds",
        ),
    ] {
        let path = common::scratch(name).join("scene.html");
        std::fs::write(&path,format!(r#"<main data-composition-id="limit" data-width="32" data-height="32" data-duration="1"><div style="{style};background:white"></div></main>"#)).unwrap();
        let composition = parse_file(&path).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        let error = renderer.render_at(0.0).unwrap_err();
        assert!(error.to_string().contains(expected), "{name}: {error}");
    }
}
