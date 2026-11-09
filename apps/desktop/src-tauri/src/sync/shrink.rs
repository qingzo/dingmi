//! 叮咪同步的套餐限单个图片多大（需求-1.0 第 32.1 节第 5 条，默认 5 MB）：超过的压一份小的再传。
//!
//! 只压**传上去的那一份**，这台电脑上笔记里的还是原图（engine::push_with 传之前调这里）。这样定的理由：
//! - 用户放进来的东西在自己电脑上不会变差：要看原图、另存为原图都还在；
//! - 开通叮咪同步以前就放进笔记的大图、手机发来的大图，都走同一条路，不用改已有的数据；
//! - 叮咪同步是试运营，以后换成网盘文件夹、自建服务器（不限大小）时，传过去的还是原图；
//! - 别的设备上看到的是压过的那份：5 MB 以内的照片、截图看起来几乎一样。
//!
//! 格式不变（JPEG 还是 JPEG，PNG 还是 PNG），免得文件名的扩展名和内容对不上。先降质量（JPEG），
//! 还不够就按比例缩小边长，最短边小到 [`MIN_SIDE`] 还压不下来就算压不下来（这张图只留在这台电脑上）。
//! 别的格式（GIF 动图、WebP 等）不压，超过就只留在这台电脑上。

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;

/// 最短边小于这么多还压不下来，就不压了（再小就看不清了）
pub const MIN_SIDE: u32 = 320;
/// 太大的图（像素数）不解开，免得一下占掉几个 GB 内存
const MAX_PIXELS: u64 = 120_000_000;

/// 这种图片能不能压
pub fn can_shrink(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/jpg" | "image/png")
}

/// 压到 `max` 字节以内：回压好的（同一种格式）；本来就不超的原样回；压不下来、认不出来回 None
pub fn fit(bytes: &[u8], mime: &str, max: u64) -> Option<Vec<u8>> {
    if bytes.len() as u64 <= max {
        return Some(bytes.to_vec());
    }
    if !can_shrink(mime) {
        return None;
    }
    let png = mime == "image/png";
    let img = decode(bytes, if png { ImageFormat::Png } else { ImageFormat::Jpeg })?;
    let encode = |im: &DynamicImage, quality: u8| if png { encode_png(im) } else { encode_jpeg(im, quality) };
    let mut last = bytes.len() as u64;
    // JPEG 先只降质量（边长不变）
    if !png {
        for q in [85, 75] {
            let out = encode(&img, q)?;
            if out.len() as u64 <= max {
                return Some(out);
            }
            last = out.len() as u64;
        }
    }
    // 按比例缩小：大小大致跟面积走，第一次按面积估一个比例，之后每次再小一点
    let (w, h) = (img.width(), img.height());
    let mut scale = ((max as f64 / last as f64).sqrt() * 0.95).min(0.9);
    for _ in 0..10 {
        let (nw, nh) = ((w as f64 * scale).round() as u32, (h as f64 * scale).round() as u32);
        if nw.min(nh) < MIN_SIDE {
            return None;
        }
        let small = img.resize_exact(nw, nh, FilterType::Triangle);
        let out = encode(&small, 80)?;
        if out.len() as u64 <= max {
            return Some(out);
        }
        scale *= 0.85;
    }
    None
}

/// 解开，照片里记着的方向（手机竖着拍的）先转正：重新编码以后这个记录就没了
fn decode(bytes: &[u8], format: ImageFormat) -> Option<DynamicImage> {
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format).into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Some(img)
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    // JPEG 没有透明，先变成 RGB
    img.to_rgb8().write_with_encoder(JpegEncoder::new_with_quality(&mut out, quality)).ok()?;
    Some(out)
}

fn encode_png(img: &DynamicImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut out, CompressionType::Best, PngFilter::Adaptive);
    img.write_with_encoder(enc).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// 一张压不太动的图：有渐变也有杂点（像照片）
    fn photo(w: u32, h: u32) -> RgbImage {
        let mut seed = 12345u32;
        RgbImage::from_fn(w, h, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let n = (seed >> 24) as u8;
            Rgb([(x * 255 / w) as u8 ^ (n & 0x3f), (y * 255 / h) as u8 ^ (n & 0x1f), n])
        })
    }

    fn size_of(bytes: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(bytes).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn jpeg_and_png_fit_under_the_limit() {
        let img = DynamicImage::ImageRgb8(photo(1200, 900));
        let big_jpeg = encode_jpeg(&img, 100).unwrap();
        let big_png = encode_png(&img).unwrap();
        // 本来就不超：原样
        assert_eq!(fit(&big_jpeg, "image/jpeg", big_jpeg.len() as u64).unwrap(), big_jpeg);
        // JPEG：压到一半以内，还是 JPEG，看得出来
        let max = big_jpeg.len() as u64 / 2;
        let out = fit(&big_jpeg, "image/jpeg", max).unwrap();
        assert!(out.len() as u64 <= max && out.starts_with(&[0xff, 0xd8]));
        assert!(size_of(&out).0 >= MIN_SIDE);
        // PNG：只能缩小边长，还是 PNG
        let max = big_png.len() as u64 / 3;
        let out = fit(&big_png, "image/png", max).unwrap();
        assert!(out.len() as u64 <= max && out.starts_with(b"\x89PNG"));
        let (w, h) = size_of(&out);
        assert!(w < 1200 && (w as f64 / h as f64 - 1200.0 / 900.0).abs() < 0.02, "按比例缩：{w}x{h}");
    }

    #[test]
    fn some_images_cannot_shrink() {
        let img = DynamicImage::ImageRgb8(photo(400, 340));
        let png = encode_png(&img).unwrap();
        // 要压到 1 KB：最短边小到 320 以下也压不下来
        assert!(fit(&png, "image/png", 1024).is_none());
        // 别的格式不压；坏的图片认不出来
        assert!(fit(&[1u8; 5000], "image/gif", 1000).is_none());
        assert!(fit(&[1u8; 5000], "image/jpeg", 1000).is_none());
        assert!(can_shrink("image/png") && !can_shrink("image/webp"));
    }
}
