//! Bounded local cover preparation; no network or model dependency.
//!
//! The fingerprint (`cover_hash`) ports Pillow's greyscale conversion and
//! LANCZOS resampling, so a cover decoded to the same pixels hashes the same.
//! Re-encoded covers cannot match Pillow's encoder bytes.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};
use sha2::{Digest, Sha256};

use crate::errors::{Error, Result};
use crate::pyerr::{self, Call};
use crate::pyjson::Map;

pub const COVER_EDGE: u32 = 640;
const MAX_EDGE: u32 = 8192;
const MAX_PIXELS: u64 = 20_000_000;

/// A decoded cover and what Pillow would report about it.
pub struct Opened {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    data: Vec<u8>,
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Png => "PNG",
        ImageFormat::WebP => "WEBP",
        _ => "",
    }
}

fn orientation_number(orientation: Orientation) -> u8 {
    match orientation {
        Orientation::NoTransforms => 1,
        Orientation::FlipHorizontal => 2,
        Orientation::Rotate180 => 3,
        Orientation::FlipVertical => 4,
        Orientation::Rotate90FlipH => 5,
        Orientation::Rotate90 => 6,
        Orientation::Rotate270FlipH => 7,
        Orientation::Rotate270 => 8,
    }
}

/// `Image.open(path)`: identify the format and read the header.
pub fn open(path: &Path) -> Result<Opened> {
    let data = pyerr::read_bytes(path).map_err(|f| Error::Os(f.message))?;
    let unidentified = || {
        Error::Os(format!(
            "cannot identify image file {}",
            crate::pystr::repr(&pyerr::path_str(path))
        ))
    };
    let format = image::guess_format(&data).map_err(|_| unidentified())?;
    if format_name(format).is_empty() {
        return Ok(Opened {
            format,
            width: 0,
            height: 0,
            orientation: 1,
            data,
        });
    }
    let (width, height, orientation) = {
        let reader = ImageReader::with_format(Cursor::new(&data), format);
        let mut decoder = reader.into_decoder().map_err(|_| unidentified())?;
        let (width, height) = decoder.dimensions();
        let orientation = decoder.orientation().map(orientation_number).unwrap_or(1);
        (width, height, orientation)
    };
    Ok(Opened {
        format,
        width,
        height,
        orientation,
        data,
    })
}

impl Opened {
    pub fn supported(&self) -> bool {
        !format_name(self.format).is_empty()
    }

    pub fn format_name(&self) -> &'static str {
        format_name(self.format)
    }

    fn too_large(&self) -> bool {
        self.width.max(self.height) > MAX_EDGE
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
    }

    /// Fully decode, as `Image.load()` does.
    pub fn decode(&self) -> std::result::Result<DynamicImage, String> {
        let mut reader = ImageReader::with_format(Cursor::new(&self.data), self.format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_EDGE);
        limits.max_image_height = Some(MAX_EDGE);
        reader.limits(limits);
        reader.decode().map_err(|e| e.to_string())
    }
}

/// Pillow's `L24`: ITU-R 601-2 luma in 16-bit fixed point.
fn l24(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 19595 + u32::from(g) * 38470 + u32::from(b) * 7471 + 0x8000) >> 16) as u8
}

/// `image.convert("L")` with Pillow's rules for each source mode.
fn to_luma(image: &DynamicImage) -> (u32, u32, Vec<u8>) {
    let (w, h) = (image.width(), image.height());
    let pixels = match image {
        DynamicImage::ImageLuma8(img) => img.as_raw().clone(),
        DynamicImage::ImageLuma16(img) => img.as_raw().iter().map(|&v| v.min(255) as u8).collect(),
        DynamicImage::ImageLumaA8(img) => img.pixels().map(|p| p.0[0]).collect(),
        DynamicImage::ImageLumaA16(img) => img.pixels().map(|p| (p.0[0] >> 8) as u8).collect(),
        DynamicImage::ImageRgb8(img) => img.pixels().map(|p| l24(p.0[0], p.0[1], p.0[2])).collect(),
        DynamicImage::ImageRgba8(img) => {
            img.pixels().map(|p| l24(p.0[0], p.0[1], p.0[2])).collect()
        }
        DynamicImage::ImageRgb16(img) => img
            .pixels()
            .map(|p| {
                l24(
                    (p.0[0] >> 8) as u8,
                    (p.0[1] >> 8) as u8,
                    (p.0[2] >> 8) as u8,
                )
            })
            .collect(),
        DynamicImage::ImageRgba16(img) => img
            .pixels()
            .map(|p| {
                l24(
                    (p.0[0] >> 8) as u8,
                    (p.0[1] >> 8) as u8,
                    (p.0[2] >> 8) as u8,
                )
            })
            .collect(),
        other => {
            let rgb = other.to_rgb8();
            rgb.pixels().map(|p| l24(p.0[0], p.0[1], p.0[2])).collect()
        }
    };
    (w, h, pixels)
}

/// Apply an EXIF orientation to a greyscale buffer, as `exif_transpose`.
fn orient_luma(w: u32, h: u32, pixels: Vec<u8>, orientation: u8) -> (u32, u32, Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize| pixels[y * w + x];
    let transposed = matches!(orientation, 5..=8);
    let (ow, oh) = if transposed { (h, w) } else { (w, h) };
    if !(2..=8).contains(&orientation) {
        return (w as u32, h as u32, pixels);
    }
    let mut out = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        for x in 0..ow {
            let value = match orientation {
                2 => at(w - 1 - x, y),
                3 => at(w - 1 - x, h - 1 - y),
                4 => at(x, h - 1 - y),
                5 => at(y, x),
                6 => at(y, h - 1 - x),
                7 => at(w - 1 - y, h - 1 - x),
                _ => at(w - 1 - y, x),
            };
            out.push(value);
        }
    }
    (ow as u32, oh as u32, out)
}

const PRECISION_BITS: u32 = 32 - 8 - 2;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let x = x * std::f64::consts::PI;
    x.sin() / x
}

fn lanczos(x: f64) -> f64 {
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// Pillow's `precompute_coeffs` plus `normalize_coeffs_8bpc`.
fn coefficients(in_size: usize, out_size: usize) -> (usize, Vec<(usize, usize)>, Vec<i32>) {
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 3.0 * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let mut bounds = Vec::with_capacity(out_size);
    let mut kk = vec![0i32; out_size * ksize];
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let ss = 1.0 / filterscale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize;
        let count = xmax.saturating_sub(xmin);
        let mut weights = vec![0f64; ksize];
        let mut ww = 0.0;
        for (x, w) in weights.iter_mut().enumerate().take(count) {
            *w = lanczos((x as f64 + xmin as f64 - center + 0.5) * ss);
            ww += *w;
        }
        for (x, w) in weights.iter().enumerate() {
            let normalised = if x < count && ww != 0.0 { w / ww } else { *w };
            let fixed = normalised * f64::from(1u32 << PRECISION_BITS);
            kk[xx * ksize + x] = if normalised < 0.0 {
                (-0.5 + fixed) as i32
            } else {
                (0.5 + fixed) as i32
            };
        }
        bounds.push((xmin, count));
    }
    (ksize, bounds, kk)
}

fn clip8(value: i64) -> u8 {
    if value >= (1i64 << PRECISION_BITS << 8) {
        255
    } else if value <= 0 {
        0
    } else {
        (value >> PRECISION_BITS) as u8
    }
}

/// `image.resize((out_w, out_h), Image.Resampling.LANCZOS)` for mode "L".
fn resize_luma(w: usize, h: usize, pixels: &[u8], out_w: usize, out_h: usize) -> Vec<u8> {
    if w == out_w && h == out_h {
        return pixels.to_vec();
    }
    let (ksize_h, bounds_h, kk_h) = coefficients(w, out_w);
    let (ksize_v, mut bounds_v, kk_v) = coefficients(h, out_h);
    let first = bounds_v[0].0;
    let last = bounds_v[out_h - 1].0 + bounds_v[out_h - 1].1;
    let mut source = pixels.to_vec();
    let mut src_w = w;
    if out_w != w {
        for b in &mut bounds_v {
            b.0 -= first;
        }
        let rows = last - first;
        let mut temp = vec![0u8; out_w * rows];
        for yy in 0..rows {
            for xx in 0..out_w {
                let (xmin, count) = bounds_h[xx];
                let k = &kk_h[xx * ksize_h..];
                let mut ss: i64 = 1 << (PRECISION_BITS - 1);
                for x in 0..count {
                    ss += i64::from(source[(yy + first) * w + x + xmin]) * i64::from(k[x]);
                }
                temp[yy * out_w + xx] = clip8(ss);
            }
        }
        source = temp;
        src_w = out_w;
    }
    if out_h == h && out_w != w {
        return source;
    }
    let mut out = vec![0u8; src_w * out_h];
    for yy in 0..out_h {
        let (ymin, count) = bounds_v[yy];
        let k = &kk_v[yy * ksize_v..];
        for xx in 0..src_w {
            let mut ss: i64 = 1 << (PRECISION_BITS - 1);
            for y in 0..count {
                ss += i64::from(source[(y + ymin) * src_w + xx]) * i64::from(k[y]);
            }
            out[yy * src_w + xx] = clip8(ss);
        }
    }
    out
}

/// thumb_version (content digest) and cover_hash (difference hash).
pub fn cover_facts(path: &Path) -> Map {
    let mut result = Map::new();
    let Ok(meta) = std::fs::metadata(path) else {
        return result;
    };
    if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
        return result;
    }
    let Ok(data) = std::fs::read(path) else {
        return result;
    };
    let digest = Sha256::digest(&data);
    let version: String = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..24]
        .to_string();
    result.insert("thumb_version".into(), version.into());
    result.insert("cover_hash".into(), "".into());
    let Ok(opened) = open(path) else {
        return result;
    };
    if !opened.supported() || opened.too_large() {
        return result;
    }
    let Ok(image) = opened.decode() else {
        return result;
    };
    let (w, h, luma) = to_luma(&image);
    let (w, h, luma) = orient_luma(w, h, luma, opened.orientation);
    let pixels = resize_luma(w as usize, h as usize, &luma, 9, 8);
    let mut bits: u64 = 0;
    for row in 0..8 {
        for col in 0..8 {
            bits = (bits << 1) | u64::from(pixels[row * 9 + col] > pixels[row * 9 + col + 1]);
        }
    }
    result.insert("cover_hash".into(), format!("{bits:016x}").into());
    result
}

/// `Image.thumbnail` target size: fit within the box, keep the aspect ratio.
fn thumbnail_size(width: u32, height: u32, edge: u32) -> Option<(u32, u32)> {
    let (x, y) = (edge, edge);
    if x >= width && y >= height {
        return None;
    }
    let aspect = f64::from(width) / f64::from(height);
    let round_aspect = |number: f64, key: &dyn Fn(f64) -> f64| -> u32 {
        let (floor, ceil) = (number.floor(), number.ceil());
        let best = if key(ceil) < key(floor) { ceil } else { floor };
        (best as u32).max(1)
    };
    if f64::from(x) / f64::from(y) >= aspect {
        let nx = round_aspect(f64::from(y) * aspect, &|n| {
            (aspect - n / f64::from(y)).abs()
        });
        Some((nx, y))
    } else {
        let ny = round_aspect(f64::from(x) / aspect, &|n| {
            if n == 0.0 {
                0.0
            } else {
                (aspect - f64::from(x) / n).abs()
            }
        });
        Some((x, ny))
    }
}

/// Orient and shrink a verified image in place, keeping its format and
/// transparency. False when it is already small and upright.
pub fn resize_cover(path: &Path) -> Result<bool> {
    let opened = open(path)?;
    if !opened.supported() {
        return Err(Error::Value("unsupported cover format".into()));
    }
    if opened.too_large() {
        return Err(Error::Value("cover dimensions exceed limits".into()));
    }
    if opened.width.max(opened.height) <= COVER_EDGE && opened.orientation == 1 {
        return Ok(false);
    }
    let mut image = opened.decode().map_err(Error::Os)?;
    if let Some(orientation) = Orientation::from_exif(opened.orientation)
        && opened.orientation != 1
    {
        image.apply_orientation(orientation);
    }
    if let Some((w, h)) = thumbnail_size(image.width(), image.height(), COVER_EDGE) {
        image = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    }
    if opened.format == ImageFormat::Jpeg
        && !matches!(
            image,
            DynamicImage::ImageRgb8(_) | DynamicImage::ImageLuma8(_)
        )
    {
        image = DynamicImage::ImageRgb8(image.to_rgb8());
    }
    if opened.format == ImageFormat::WebP
        && !matches!(
            image,
            DynamicImage::ImageRgb8(_) | DynamicImage::ImageRgba8(_)
        )
    {
        image = DynamicImage::ImageRgba8(image.to_rgba8());
    }
    let mut encoded = Vec::new();
    let write_result = match opened.format {
        ImageFormat::Jpeg => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 85);
            image.write_with_encoder(encoder)
        }
        ImageFormat::Png => {
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                &mut encoded,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            image.write_with_encoder(encoder)
        }
        // shortcut: the image crate encodes WebP losslessly only; Pillow
        // writes lossy quality 85. Covers stay WebP, at a larger size.
        // Use a lossy encoder if cover storage size becomes a problem.
        _ => image.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)),
    };
    write_result.map_err(|e| Error::Os(e.to_string()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let (temp, file) = crate::storage::create_temp(dir, "tmp", ".tmp")?;
    drop(file);
    let result = std::fs::write(&temp, &encoded)
        .map_err(|e| Error::Os(pyerr::os_error(&e, Call::Open, &temp)))
        .and_then(|()| {
            std::fs::rename(&temp, path)
                .map_err(|e| Error::Os(pyerr::os_error2(&e, Call::Os, &temp, path)))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map(|()| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_sizes_follow_pillow() {
        assert_eq!(thumbnail_size(800, 1600, 640), Some((320, 640)));
        assert_eq!(thumbnail_size(1000, 800, 640), Some((640, 512)));
        assert_eq!(thumbnail_size(640, 100, 640), None);
    }
}
