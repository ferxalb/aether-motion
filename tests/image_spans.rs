//! Exact image compositing across transparent margins, opaque rows, and frame clipping.

mod common;

use aether_motion::{parse::parse_file, render::Renderer};

#[test]
fn mixed_alpha_rows_match_original_pixel_formula() {
    let dir = common::scratch("image_spans");
    let image = image::RgbaImage::from_fn(16, 8, |x, y| {
        let a = if !(3..13).contains(&x) || y == 0 {
            0
        } else if y < 4 {
            255
        } else {
            137
        };
        image::Rgba([x as u8 * 13, y as u8 * 29, 73, a])
    });
    image.save(dir.join("mixed.png")).unwrap();
    for alpha in [0.0_f32, 0.37, 1.0] {
        let path = dir.join("scene.html");
        std::fs::write(&path, format!(r#"<main data-composition-id="test" data-width="12" data-height="8" data-duration="1" style="background:#14283c"><img src="mixed.png" style="left:-2px;top:-1px;width:16px;height:8px;opacity:{alpha}"></main>"#)).unwrap();
        let composition = parse_file(&path).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        let actual = renderer.render_at(0.0).unwrap();
        for y in 0..8 {
            for x in 0..12 {
                let mut expected = [20, 40, 60, 255];
                if y < 7 {
                    let source = image.get_pixel(x + 2, y + 1).0;
                    if source[3] != 0 {
                        let a = alpha * f32::from(source[3]) / 255.0;
                        for c in 0..3 {
                            expected[c] = (f32::from(source[c]) * a
                                + f32::from(expected[c]) * (1.0 - a))
                                .round() as u8;
                        }
                    }
                }
                let i = (y as usize * 12 + x as usize) * 4;
                assert_eq!(&actual[i..i + 4], expected, "alpha {alpha}, pixel {x},{y}");
            }
        }
    }
}
