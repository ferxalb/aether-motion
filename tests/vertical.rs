//! Vertical 9:16 output: size presets pick portrait dimensions and layout scales to fill the frame.

mod common;

use aether_hyperframes::parse::parse_file;
use aether_hyperframes::render::{OutputSize, Renderer};

fn vertical_composition(name: &str) -> aether_hyperframes::model::Composition {
    let html = r#"<html><head><style>
        .bar { left: 0px; top: 0px; width: 540px; height: 96px; background: #f00 }
      </style></head><body>
      <main data-composition-id="v" data-width="540" data-height="960" data-duration="1" style="background:#000">
        <div class="clip bar" data-start="0" data-duration="1"></div>
      </main></body></html>"#;
    let path = common::scratch(name).join("v.html");
    std::fs::write(&path, html).unwrap();
    parse_file(&path).unwrap()
}

#[test]
fn size_presets_render_portrait_frames_for_9_16_compositions() {
    let composition = vertical_composition("vertical_1080");
    let mut renderer = Renderer::with_output_size(&composition, OutputSize::FullHd).unwrap();
    assert_eq!(renderer.dimensions(), (1080, 1920));
    let pixels = renderer.render_at(0.0).unwrap().to_vec();
    let at = |x: usize, y: usize| {
        let i = (y * 1080 + x) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    // The 96px bar becomes 192px tall and spans the full width.
    assert_eq!(at(1079, 191), [255, 0, 0]);
    assert_eq!(at(540, 193), [0, 0, 0]);
    assert_eq!(at(540, 1900), [0, 0, 0]);
}

#[test]
fn the_4k_preset_is_2160_by_3840_in_portrait() {
    let composition = vertical_composition("vertical_4k");
    let renderer = Renderer::with_output_size(&composition, OutputSize::Uhd).unwrap();
    assert_eq!(renderer.dimensions(), (2160, 3840));
}
