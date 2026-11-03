//! Image conversion backed by the pure-Rust `image` crate.

use super::{failure, success};
use crate::convert::{ConvertRequest, ConvertResult};
use image::{DynamicImage, ImageEncoder, ImageFormat};
use std::io::Cursor;
use std::path::Path;
use std::time::Instant;

pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult {
    let bytes = match std::fs::read(&request.input) {
        Ok(bytes) => bytes,
        Err(err) => {
            return failure(
                format!("Cannot read {}: {err}", request.input.display()),
                started,
            );
        }
    };

    if bytes.is_empty() {
        return failure(format!("{} is empty", request.input.display()), started);
    }

    let Some(input_format) = ImageFormat::from_extension(&request.from) else {
        return failure(
            format!("No built-in decoder for '{}'", request.from),
            started,
        );
    };

    let mut image =
        match image::ImageReader::with_format(Cursor::new(&bytes), input_format).decode() {
            Ok(image) => image,
            Err(err) => {
                return failure(
                    format!("Cannot decode {}: {err}", request.input.display()),
                    started,
                );
            }
        };

    // The `image` crate ignores EXIF, so phone photos would come out sideways.
    if let Some(orientation) = exif_orientation(&bytes) {
        image = apply_orientation(image, orientation);
    }

    match encode(
        &image,
        &request.to,
        request.options.jpeg_quality,
        &request.output,
    ) {
        Ok(note) => success(
            if note.is_empty() {
                "Converted".to_string()
            } else {
                note
            },
            &request.output,
            started,
        ),
        Err(err) => failure(err, started),
    }
}

/// Writes `image` in `target` format. Returns a short note for the UI log.
pub fn encode(
    image: &DynamicImage,
    target: &str,
    jpeg_quality: u8,
    output: &Path,
) -> Result<String, String> {
    let Some(format) = ImageFormat::from_extension(target) else {
        return Err(format!("No built-in encoder for '{target}'"));
    };

    let write = |result: Result<(), image::ImageError>| {
        result.map_err(|err| format!("Cannot encode {}: {err}", output.display()))
    };

    match format {
        ImageFormat::Jpeg | ImageFormat::Pnm => {
            // Neither container carries alpha, so drop it the way ImageMagick does.
            let rgb = image.to_rgb8();
            if format == ImageFormat::Jpeg {
                let file = std::fs::File::create(output)
                    .map_err(|err| format!("Cannot create {}: {err}", output.display()))?;
                let writer = std::io::BufWriter::new(file);
                write(
                    image::codecs::jpeg::JpegEncoder::new_with_quality(
                        writer,
                        jpeg_quality.clamp(1, 100),
                    )
                    .write_image(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        image::ExtendedColorType::Rgb8,
                    ),
                )?;
                Ok(format!("Converted (JPEG quality {})", jpeg_quality))
            } else {
                write(rgb.save_with_format(output, format))?;
                Ok(String::new())
            }
        }
        ImageFormat::WebP => {
            // The encoder in `image` is lossless only; quality doubles as effort.
            write(image.save_with_format(output, format))?;
            Ok("Converted (lossless WebP)".into())
        }
        _ => {
            if (image.width() > 256 || image.height() > 256) && format == ImageFormat::Ico {
                // ICO tops out at 256×256; let another converter take over.
                return Err(format!(
                    "ICO is limited to 256×256, this image is {}×{}",
                    image.width(),
                    image.height()
                ));
            }
            write(image.save_with_format(output, format))?;
            Ok(String::new())
        }
    }
}

fn apply_orientation(image: DynamicImage, orientation: u8) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate90().flipv(),
        8 => image.rotate270(),
        _ => image,
    }
}

/// Reads the EXIF orientation tag (0x0112) without pulling in a full EXIF parser.
fn exif_orientation(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut offset = 2usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if length < 2 || offset + 2 + length > bytes.len() {
            return None;
        }
        if marker == 0xE1 {
            let segment = &bytes[offset + 4..offset + 2 + length];
            if segment.starts_with(b"Exif\0\0") {
                return parse_tiff_orientation(&segment[6..]);
            }
        }
        offset += 2 + length;
    }
    None
}

fn parse_tiff_orientation(tiff: &[u8]) -> Option<u8> {
    if tiff.len() < 8 {
        return None;
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |at: usize| -> Option<u16> {
        let slice = tiff.get(at..at + 2)?;
        Some(if little {
            u16::from_le_bytes([slice[0], slice[1]])
        } else {
            u16::from_be_bytes([slice[0], slice[1]])
        })
    };
    let read_u32 = |at: usize| -> Option<u32> {
        let slice = tiff.get(at..at + 4)?;
        Some(if little {
            u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
        } else {
            u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]])
        })
    };

    let ifd = read_u32(4)? as usize;
    let count = read_u16(ifd)? as usize;
    for index in 0..count {
        let entry = ifd + 2 + index * 12;
        if read_u16(entry)? != 0x0112 {
            continue;
        }
        let value = read_u16(entry + 8)?;
        if (1..=8).contains(&value) {
            return Some(value as u8);
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([12, 200, 40, 255]));
        let mut buffer = Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, ImageFormat::Png)
            .expect("encode png");
        buffer.into_inner()
    }

    #[test]
    fn decodes_and_encodes_between_formats() {
        let dir = std::env::temp_dir().join(format!("convertx-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.png");
        std::fs::write(&input, png_bytes(8, 4)).unwrap();

        for target in ["jpeg", "bmp", "tiff", "gif", "qoi", "webp", "tga", "pnm"] {
            let request = ConvertRequest {
                job_id: 1,
                input: input.clone(),
                from: "png".into(),
                to: target.into(),
                converter_id: "native-images".into(),
                output: dir.join(format!("out.{target}")),
                options: crate::convert::ConvertOptions::default(),
            };
            let result = convert(&request, Instant::now());
            assert!(result.ok, "png -> {target} failed: {}", result.message);
            let size = std::fs::metadata(&request.output).unwrap().len();
            assert!(size > 0, "png -> {target} produced an empty file");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_exif_orientation_from_jpeg() {
        // Minimal JPEG header: SOI + APP1 with a big-endian TIFF block carrying orientation 6.
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x22];
        bytes.extend_from_slice(b"Exif\0\0");
        bytes.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08");
        bytes.extend_from_slice(&[0x00, 0x01]);
        bytes.extend_from_slice(&[
            0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00,
        ]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(exif_orientation(&bytes), Some(6));
        assert_eq!(exif_orientation(&[0x89, b'P', b'N', b'G']), None);
    }
}
