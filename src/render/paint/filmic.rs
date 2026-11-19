//! Optional root tone curve, evaluated in linear sRGB with a precomputed per-channel LUT.

use crate::model::Filmic;

pub(in crate::render) struct FilmicLut([u8; 256]);

impl FilmicLut {
    pub(in crate::render) fn new(settings: Filmic) -> Self {
        let gain = settings.exposure.exp2();
        Self(std::array::from_fn(|code| {
            let srgb = code as f64 / 255.0;
            let linear = if srgb <= 0.04045 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            };
            let x = linear * gain;
            // The toe preserves black and fades out toward the midtones.
            let x = x / (1.0 + settings.shadows * (-x * 8.0).exp());
            // Begin rolling off above middle gray, approaching display white asymptotically.
            let knee = 1.0 - 0.75 * settings.highlights;
            let x = if settings.highlights > 0.0 && x > knee {
                let t = (x - knee) / (1.0 - knee);
                knee + (1.0 - knee) * t / (1.0 + t)
            } else {
                x
            };
            let srgb = if x <= 0.0031308 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
        }))
    }

    pub(in crate::render) fn apply(&self, pixels: &mut [u8]) {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            for channel in &mut pixel[..3] {
                *channel = self.0[*channel as usize];
            }
        }
    }
}
