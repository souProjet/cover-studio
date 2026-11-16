//! Thumbnail and caption for social posts.

use crate::error::Result;
use crate::settings::{InstrumentKind, Language};
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use std::path::Path;

const FONT_TITLE: &[u8] = include_bytes!("../assets/fonts/Lato-Black.ttf");
const FONT_SUB: &[u8] = include_bytes!("../assets/fonts/Lato-Bold.ttf");

fn text_width(font: &FontRef, scale: PxScale, text: &str) -> f32 {
    let f = font.as_scaled(scale);
    let mut w = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

fn draw_text(
    img: &mut RgbImage,
    font: &FontRef,
    scale: PxScale,
    x: f32,
    baseline: f32,
    text: &str,
    color: [u8; 3],
) {
    let f = font.as_scaled(scale);
    let mut caret = x;
    let mut prev = None;
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            caret += f.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        caret += f.h_advance(id);
        prev = Some(id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let b = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let px = b.min.x as i32 + gx as i32;
                let py = b.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= img.width() as i32 || py >= img.height() as i32 {
                    return;
                }
                let p = img.get_pixel_mut(px as u32, py as u32);
                for (dst, src) in p.0.iter_mut().zip(color) {
                    *dst = (*dst as f32 * (1.0 - cov) + src as f32 * cov) as u8;
                }
            });
        }
    }
}

/// Largest scale (down from `max_px`) at which `text` fits in `width`.
fn fit_scale(font: &FontRef, text: &str, max_px: f32, width: f32) -> PxScale {
    let mut px = max_px;
    while px > 18.0 && text_width(font, PxScale::from(px), text) > width {
        px -= 2.0;
    }
    PxScale::from(px)
}

/// Warm dark gradient used when there is no video frame.
pub fn gradient_background(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        let t = y as f32 / height as f32;
        let u = x as f32 / width as f32;
        let r = 40.0 + 90.0 * (1.0 - t) * (0.6 + 0.4 * u);
        let g = 22.0 + 35.0 * (1.0 - t);
        let b = 30.0 + 50.0 * t;
        Rgb([r as u8, g as u8, b as u8])
    })
}

/// Draws a darkened lower band with the title and subtitle, then saves a JPEG.
pub fn compose_thumbnail(mut img: RgbImage, title: &str, subtitle: &str, out: &Path) -> Result<()> {
    let (w, h) = img.dimensions();
    let band_top = (h as f32 * 0.68) as u32;
    for y in band_top..h {
        // Smooth ramp so the band does not look pasted on.
        let t = ((y - band_top) as f32 / (h as f32 * 0.12)).min(1.0);
        let dim = 1.0 - 0.6 * t;
        for x in 0..w {
            let p = img.get_pixel_mut(x, y);
            for c in p.0.iter_mut() {
                *c = (*c as f32 * dim) as u8;
            }
        }
    }
    let title_font = FontRef::try_from_slice(FONT_TITLE).expect("embedded font");
    let sub_font = FontRef::try_from_slice(FONT_SUB).expect("embedded font");
    let margin = w as f32 * 0.07;
    let usable = w as f32 - 2.0 * margin;
    let unit = w.min(h) as f32;
    let ts = fit_scale(&title_font, title, unit * 0.1, usable);
    let ss = fit_scale(&sub_font, subtitle, unit * 0.05, usable);
    let title_base = band_top as f32 + (h - band_top) as f32 * 0.45;
    draw_text(
        &mut img,
        &title_font,
        ts,
        margin,
        title_base,
        title,
        [255, 246, 236],
    );
    draw_text(
        &mut img,
        &sub_font,
        ss,
        margin,
        title_base + ss.y * 1.5,
        subtitle,
        [232, 212, 190],
    );
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(out)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 92).encode_image(&img)?;
    Ok(())
}

fn hashtag(s: &str) -> Option<String> {
    let t: String = s.chars().filter(|c| c.is_alphanumeric()).collect();
    (!t.is_empty()).then(|| format!("#{}", t.to_lowercase()))
}

fn instrument_words(
    kind: InstrumentKind,
    lang: Language,
) -> (&'static str, &'static [&'static str]) {
    match (lang, kind) {
        (Language::Fr, InstrumentKind::AcousticGuitar) => {
            ("guitare-voix", &["#guitare", "#acoustique", "#guitarcover"])
        }
        (Language::Fr, InstrumentKind::ClassicalGuitar) => (
            "guitare classique et voix",
            &["#guitareclassique", "#acoustique"],
        ),
        (Language::Fr, InstrumentKind::ElectricGuitar) => (
            "guitare électrique et voix",
            &["#guitareelectrique", "#guitarcover"],
        ),
        (Language::Fr, InstrumentKind::Piano) => ("piano-voix", &["#piano", "#pianocover"]),
        (Language::Fr, InstrumentKind::Ukulele) => {
            ("ukulélé et voix", &["#ukulele", "#ukulelecover"])
        }
        (Language::Fr, InstrumentKind::FullBand) => ("version groupe", &["#groupe", "#live"]),
        (Language::Fr, InstrumentKind::Other) => ("version perso", &["#musique"]),
        (Language::En, InstrumentKind::AcousticGuitar) => {
            ("acoustic guitar and vocals", &["#acoustic", "#guitarcover"])
        }
        (Language::En, InstrumentKind::ClassicalGuitar) => (
            "classical guitar and vocals",
            &["#classicalguitar", "#acoustic"],
        ),
        (Language::En, InstrumentKind::ElectricGuitar) => (
            "electric guitar and vocals",
            &["#electricguitar", "#guitarcover"],
        ),
        (Language::En, InstrumentKind::Piano) => ("piano and vocals", &["#piano", "#pianocover"]),
        (Language::En, InstrumentKind::Ukulele) => {
            ("ukulele and vocals", &["#ukulele", "#ukulelecover"])
        }
        (Language::En, InstrumentKind::FullBand) => ("band version", &["#band", "#live"]),
        (Language::En, InstrumentKind::Other) => ("my take", &["#music"]),
    }
}

pub fn caption(
    title: &str,
    artist: &str,
    kind: InstrumentKind,
    lang: Language,
    extra: &[String],
) -> String {
    let (instrument, tags) = instrument_words(kind, lang);
    let who = if artist.trim().is_empty() {
        String::new()
    } else {
        format!(" ({})", artist.trim())
    };
    let mut hashtags: Vec<String> = vec!["#cover".into()];
    hashtags.extend(hashtag(title));
    hashtags.extend(hashtag(artist));
    hashtags.extend(tags.iter().map(|t| t.to_string()));
    match lang {
        Language::Fr => hashtags.extend(["#reprise", "#chant", "#homestudio"].map(String::from)),
        Language::En => hashtags.extend(["#singing", "#homestudio", "#musician"].map(String::from)),
    }
    hashtags.extend(
        extra
            .iter()
            .filter_map(|t| hashtag(t.trim_start_matches('#'))),
    );
    hashtags.dedup();
    let body = match lang {
        Language::Fr => format!(
            "{title}{who} - reprise {instrument}.\n\nEnregistré à la maison, en une prise.\nDites-moi ce que vous en pensez en commentaire !"
        ),
        Language::En => format!(
            "{title}{who} - {instrument} cover.\n\nRecorded at home in one take.\nLet me know what you think in the comments!"
        ),
    };
    format!("{body}\n\n{}\n", hashtags.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_contains_tags() {
        let c = caption(
            "Mockingbird",
            "Eminem",
            InstrumentKind::AcousticGuitar,
            Language::Fr,
            &["#live".into()],
        );
        assert!(c.contains("#mockingbird"));
        assert!(c.contains("#eminem"));
        assert!(c.contains("#live"));
        assert!(c.starts_with("Mockingbird (Eminem)"));
    }

    #[test]
    fn thumbnail_renders() {
        let dir = std::env::temp_dir().join("cover-engine-thumb");
        let out = dir.join("t.jpg");
        compose_thumbnail(
            gradient_background(540, 960),
            "Nothing Else Matters",
            "Cover guitare-voix",
            &out,
        )
        .unwrap();
        assert!(out.metadata().unwrap().len() > 5_000);
        std::fs::remove_dir_all(dir).ok();
    }
}
