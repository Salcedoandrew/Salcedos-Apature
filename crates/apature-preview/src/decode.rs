//! JPEG decoding with DCT-domain downscaling, plus orientation and thumbnailing.

use image::{imageops, RgbImage};

/// A decoded 8-bit RGB image, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgb {
    pub fn long_side(&self) -> u32 {
        self.width.max(self.height)
    }
    pub fn into_image(self) -> Option<RgbImage> {
        RgbImage::from_raw(self.width, self.height, self.data)
    }
    pub fn from_image(img: RgbImage) -> Self {
        Rgb {
            width: img.width(),
            height: img.height(),
            data: img.into_raw(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("jpeg: {0}")]
    Jpeg(#[from] jpeg_decoder::Error),
    #[error("unsupported pixel format {0:?}")]
    PixelFormat(jpeg_decoder::PixelFormat),
    #[error("decoded buffer size mismatch")]
    SizeMismatch,
}

/// Decode a JPEG. If `max_long_side` is given, the decoder picks the largest
/// DCT scale (1/1, 1/2, 1/4, 1/8) that still yields at least that many pixels
/// on the long side, so a 6000px preview requested at 320px decodes at 750px
/// in roughly one eighth the time of a full decode.
pub fn decode_jpeg(bytes: &[u8], max_long_side: Option<u32>) -> Result<Rgb, DecodeError> {
    let mut d = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    d.read_info()?;
    let info = d
        .info()
        .ok_or(jpeg_decoder::Error::Format("no header".into()))?;
    let (mut w, mut h) = (info.width as u32, info.height as u32);
    // A header can claim any size up to 65535×65535 (about 12.9 GB of RGB),
    // and the decoder allocates that before a single pixel is read. No
    // single-shot camera sensor is anywhere near 250 megapixels, so a header
    // past that is a lie or a corrupt file: refuse it before decoding.
    // Stitched panoramas come through the scaled path, whose target size
    // bounds the decode; this check bounds the header claim itself.
    const MAX_PIXELS: u64 = 250_000_000;
    let claimed = w as u64 * h as u64;
    if w == 0 || h == 0 || claimed > MAX_PIXELS {
        return Err(DecodeError::Jpeg(jpeg_decoder::Error::Format(format!(
            "refusing absurd JPEG dimensions {w}×{h} (over {MAX_PIXELS} pixels)"
        ))));
    }
    if let Some(t) = max_long_side {
        let long = w.max(h);
        if long > t && long > 0 {
            let s = t as f32 / long as f32;
            let rw = ((w as f32) * s).ceil().max(1.0) as u16;
            let rh = ((h as f32) * s).ceil().max(1.0) as u16;
            let (sw, sh) = d.scale(rw, rh)?;
            w = sw as u32;
            h = sh as u32;
        }
    }
    let pixels = d.decode()?;
    let info = d
        .info()
        .ok_or(jpeg_decoder::Error::Format("no header".into()))?;
    // After scaling, info reports the output size.
    if info.width as u32 != w || info.height as u32 != h {
        w = info.width as u32;
        h = info.height as u32;
    }
    let n = (w * h) as usize;
    use jpeg_decoder::PixelFormat::*;
    let data = match info.pixel_format {
        RGB24 => {
            if pixels.len() != n * 3 {
                return Err(DecodeError::SizeMismatch);
            }
            pixels
        }
        L8 => {
            if pixels.len() != n {
                return Err(DecodeError::SizeMismatch);
            }
            pixels.iter().flat_map(|&g| [g, g, g]).collect()
        }
        L16 => {
            if pixels.len() != n * 2 {
                return Err(DecodeError::SizeMismatch);
            }
            pixels.chunks(2).flat_map(|c| [c[1], c[1], c[1]]).collect()
        }
        CMYK32 => {
            if pixels.len() != n * 4 {
                return Err(DecodeError::SizeMismatch);
            }
            pixels
                .chunks(4)
                .flat_map(|c| {
                    let k = c[3] as u32;
                    let f = |x: u8| ((x as u32) * k / 255) as u8;
                    [f(c[0]), f(c[1]), f(c[2])]
                })
                .collect()
        }
    };
    Ok(Rgb {
        width: w,
        height: h,
        data,
    })
}

/// Apply an EXIF orientation (1..=8) so the image displays upright.
pub fn rotate(img: Rgb, orientation: u32) -> Rgb {
    if orientation <= 1 || orientation > 8 {
        return img;
    }
    let Some(im) = img.clone().into_image() else {
        return img;
    };
    let out = match orientation {
        2 => imageops::flip_horizontal(&im),
        3 => imageops::rotate180(&im),
        4 => imageops::flip_vertical(&im),
        5 => imageops::flip_horizontal(&imageops::rotate90(&im)),
        6 => imageops::rotate90(&im),
        7 => imageops::flip_horizontal(&imageops::rotate270(&im)),
        8 => imageops::rotate270(&im),
        _ => im,
    };
    Rgb::from_image(out)
}

/// Box-filter downscale so the long side is at most `max_side`.
pub fn thumbnail(img: &Rgb, max_side: u32) -> Rgb {
    let long = img.long_side();
    if long <= max_side || long == 0 {
        return img.clone();
    }
    let s = max_side as f32 / long as f32;
    let nw = ((img.width as f32) * s).round().max(1.0) as u32;
    let nh = ((img.height as f32) * s).round().max(1.0) as u32;
    let Some(im) = img.clone().into_image() else {
        return img.clone();
    };
    Rgb::from_image(imageops::thumbnail(&im, nw, nh))
}

/// Encode as JPEG (used by the disk cache).
pub fn encode_jpeg(img: &Rgb, quality: u8) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    use image::ImageEncoder;
    enc.write_image(
        &img.data,
        img.width,
        img.height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Rgb {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[
                    (x * 255 / w.max(1)) as u8,
                    (y * 255 / h.max(1)) as u8,
                    128,
                ]);
            }
        }
        Rgb {
            width: w,
            height: h,
            data,
        }
    }

    #[test]
    fn encode_decode_round_trip_and_scale() {
        let src = gradient(1600, 1200);
        let bytes = encode_jpeg(&src, 90).unwrap();
        let full = decode_jpeg(&bytes, None).unwrap();
        assert_eq!((full.width, full.height), (1600, 1200));
        let small = decode_jpeg(&bytes, Some(320)).unwrap();
        // 1/4 scale is the largest that still gives >= 320 on the long side.
        assert_eq!((small.width, small.height), (400, 300));
        let t = thumbnail(&small, 160);
        assert_eq!((t.width, t.height), (160, 120));
    }

    #[test]
    fn rotation_swaps_axes() {
        let src = gradient(40, 20);
        let r = rotate(src.clone(), 6);
        assert_eq!((r.width, r.height), (20, 40));
        assert_eq!(rotate(src.clone(), 1), src);
        assert_eq!(rotate(src.clone(), 3).data.len(), src.data.len());
    }

    #[test]
    fn absurd_header_dimensions_are_refused_without_allocating() {
        // Finding #9b: a header claiming 65535×65535 would make the decoder
        // allocate about 12.9 GB before reading a pixel. The cap refuses it
        // as an error instead. These bytes are only a header (SOI, APP0,
        // SOF0) — there is no scan data, and the decode must fail before
        // any of it is needed.
        let bytes: Vec<u8> = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x10, // APP0, length 16
            b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
            0x00, // 14 bytes of JFIF payload
            0xFF, 0xC0, 0x00, 0x11, // SOF0, length 17
            0x08, // precision
            0xFF, 0xFF, // height 65535
            0xFF, 0xFF, // width 65535
            0x03, // 3 components
            0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01,
        ];
        let err = decode_jpeg(&bytes, None).unwrap_err();
        assert!(
            err.to_string().contains("absurd"),
            "expected the dimension cap, got: {err}"
        );
    }
}
