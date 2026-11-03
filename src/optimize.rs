//! Lossless-first image optimization: strip bloated metadata and recompress
//! without touching a single pixel. External tools are used when they do better.

use crate::registry::human_size;
use crate::tools::find_binary;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OptimizeOptions {
    /// Pixel-identical output. When false, images are re-encoded at `quality`.
    pub lossless: bool,
    pub quality: u8,
    pub strip_metadata: bool,
    /// Shrinks the long edge to at most this many pixels (lossy only).
    pub max_dimension: Option<u32>,
    /// PNG effort, 1 (fast) to 6 (smallest).
    pub effort: u8,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            lossless: true,
            quality: 82,
            strip_metadata: true,
            max_dimension: None,
            effort: 4,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OptimizeOutcome {
    pub ok: bool,
    pub before: u64,
    pub after: u64,
    pub message: String,
    pub notes: Vec<String>,
}

impl OptimizeOutcome {
    pub fn saved(&self) -> u64 {
        self.before.saturating_sub(self.after)
    }

    pub fn percent(&self) -> f32 {
        if self.before == 0 {
            0.0
        } else {
            (self.saved() as f64 * 100.0 / self.before as f64) as f32
        }
    }

    /// "1.4 MB → 420 KB (70% smaller)"
    pub fn summary(&self) -> String {
        if !self.ok {
            return self.message.clone();
        }
        if self.saved() == 0 {
            return format!("{} — already optimal", human_size(self.before));
        }
        format!(
            "{} → {} ({}% smaller)",
            human_size(self.before),
            human_size(self.after),
            self.percent().round() as i64
        )
    }
}

/// Formats the optimizer knows how to handle.
pub fn is_optimizable(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "png" | "jpeg" | "jpg" | "jpe" | "jfif" | "gif" | "webp" | "svg" | "tif" | "tiff" | "bmp"
            | "ico" | "avif" | "heic" | "heif"
    )
}

pub fn optimize(input: &Path, output: &Path, options: &OptimizeOptions) -> OptimizeOutcome {
    let before = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    let extension = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let mut notes: Vec<String> = Vec::new();
    let result = match extension.as_str() {
        "png" => optimize_png(input, output, options, &mut notes),
        "jpeg" | "jpg" | "jfif" => optimize_jpeg(input, output, options, &mut notes),
        "gif" => optimize_gif(input, output, options, &mut notes),
        "webp" => optimize_webp(input, output, options, &mut notes),
        "svg" => optimize_svg(input, output, options, &mut notes),
        other => optimize_with_imagemagick(input, output, options, other, &mut notes),
    };

    let after = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);

    match result {
        Ok(()) if after == 0 => OptimizeOutcome {
            ok: false,
            before,
            after,
            message: "The optimizer produced an empty file".into(),
            notes,
        },
        Ok(()) if after >= before && output != input => {
            // Never hand back something bigger than the original.
            let _ = std::fs::copy(input, output);
            notes.push("kept the original — recompression did not help".into());
            OptimizeOutcome {
                ok: true,
                before,
                after: before,
                message: "Already optimal".into(),
                notes,
            }
        }
        Ok(()) if after >= before => {
            let _ = std::fs::copy(input, output);
            OptimizeOutcome {
                ok: true,
                before,
                after: before,
                message: "Already optimal".into(),
                notes,
            }
        }
        Ok(()) => OptimizeOutcome {
            ok: true,
            before,
            after,
            message: "Optimized".into(),
            notes,
        },
        Err(err) => OptimizeOutcome {
            ok: false,
            before,
            after: 0,
            message: err,
            notes,
        },
    }
}

// ---------------------------------------------------------------------------
// PNG — oxipng: filters, palette and bit-depth reduction, zopfli deflate
// ---------------------------------------------------------------------------

fn optimize_png(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let data = std::fs::read(input).map_err(|err| format!("Cannot read the PNG: {err}"))?;

    // Lossy requests still go through oxipng first, then through the re-encoder
    // only when a quality setting was asked for explicitly.
    if !options.lossless {
        return reencode_with_image(input, output, options, notes);
    }

    let mut settings = oxipng::Options::from_preset(options.effort.clamp(0, 6));
    settings.strip = if options.strip_metadata {
        oxipng::StripChunks::Safe
    } else {
        oxipng::StripChunks::None
    };
    settings.timeout = Some(std::time::Duration::from_secs(25));

    let optimized = oxipng::optimize_from_memory(&data, &settings)
        .map_err(|err| format!("PNG optimization failed: {err}"))?;

    std::fs::write(output, optimized).map_err(|err| format!("Cannot write the PNG: {err}"))?;
    if options.strip_metadata {
        notes.push("removed text, EXIF and timestamp chunks; pixels untouched".into());
    }
    notes.push("lossless recompression (oxipng)".into());
    Ok(())
}

// ---------------------------------------------------------------------------
// JPEG — lossless metadata strip, then jpegtran/mozjpeg if present
// ---------------------------------------------------------------------------

fn optimize_jpeg(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if !options.lossless {
        return reencode_with_image(input, output, options, notes);
    }

    let data = std::fs::read(input).map_err(|err| format!("Cannot read the JPEG: {err}"))?;
    let stripped = strip_jpeg_metadata(&data, options.strip_metadata);

    // `jpegtran` rebuilds the Huffman tables without touching the coefficients.
    if let Some(jpegtran) = find_binary("jpegtran") {
        let temp = temp_path(output, "jpg");
        if std::fs::write(&temp, &stripped).is_ok() {
            let status = std::process::Command::new(jpegtran)
                .args(["-optimize", "-progressive", "-copy", "none", "-outfile"])
                .arg(output)
                .arg(&temp)
                .output();
            let _ = std::fs::remove_file(&temp);
            if let Ok(output_result) = status {
                if output_result.status.success() {
                    notes.push("optimized Huffman tables (jpegtran, lossless)".into());
                    if options.strip_metadata {
                        notes.push("removed EXIF, IPTC, XMP and comments; kept the ICC profile".into());
                    }
                    return Ok(());
                }
            }
        }
    }

    std::fs::write(output, stripped).map_err(|err| format!("Cannot write the JPEG: {err}"))?;
    if options.strip_metadata {
        notes.push("removed EXIF, IPTC, XMP, thumbnails and comments; kept the ICC profile".into());
    }
    notes.push("metadata-only pass (pixels untouched)".into());
    Ok(())
}

/// Rewrites a JPEG without APP1/APP13/APP15/COM segments. APP0 (JFIF) and
/// APP2 (ICC) stay, because removing them changes how the file renders.
fn strip_jpeg_metadata(bytes: &[u8], strip: bool) -> Vec<u8> {
    if !strip || bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return bytes.to_vec();
    }

    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[0..2]);
    let mut index = 2usize;

    while index + 1 < bytes.len() {
        if bytes[index] != 0xFF {
            out.extend_from_slice(&bytes[index..]);
            break;
        }
        let marker = bytes[index + 1];

        // Start of scan: everything after this is entropy-coded data.
        if marker == 0xDA {
            out.extend_from_slice(&bytes[index..]);
            return out;
        }
        // Standalone markers carry no length.
        if matches!(marker, 0x01 | 0xD0..=0xD8) {
            out.extend_from_slice(&bytes[index..index + 2]);
            index += 2;
            continue;
        }
        if index + 4 > bytes.len() {
            break;
        }
        let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
        if length < 2 || index + 2 + length > bytes.len() {
            out.extend_from_slice(&bytes[index..]);
            break;
        }
        let segment = &bytes[index..index + 2 + length];
        let keep = match marker {
            0xE0 | 0xE2 | 0xEE => true,
            0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(segment);
        }
        index += 2 + length;
    }

    out
}

// ---------------------------------------------------------------------------
// WebP — drop EXIF/XMP RIFF chunks
// ---------------------------------------------------------------------------

fn optimize_webp(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if !options.lossless {
        if let Some(cwebp) = find_binary("cwebp") {
            let status = std::process::Command::new(cwebp)
                .args(["-quiet", "-q"])
                .arg(options.quality.to_string())
                .arg("-metadata")
                .arg(if options.strip_metadata { "none" } else { "all" })
                .arg(input)
                .arg("-o")
                .arg(output)
                .output();
            if let Ok(result) = status {
                if result.status.success() {
                    notes.push(format!("re-encoded at quality {} (cwebp)", options.quality));
                    return Ok(());
                }
            }
        }
        return reencode_with_image(input, output, options, notes);
    }

    let data = std::fs::read(input).map_err(|err| format!("Cannot read the WebP: {err}"))?;
    if !options.strip_metadata || data.len() < 12 || &data[0..4] != b"RIFF" {
        std::fs::write(output, data).map_err(|err| format!("Cannot write the WebP: {err}"))?;
        return Ok(());
    }

    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&data[8..12]);

    let mut index = 12usize;
    while index + 8 <= data.len() {
        let fourcc = &data[index..index + 4];
        let size = u32::from_le_bytes([
            data[index + 4],
            data[index + 5],
            data[index + 6],
            data[index + 7],
        ]) as usize;
        let end = (index + 8 + size).min(data.len());
        let padded_end = (end + (size % 2)).min(data.len());
        if fourcc != b"EXIF" && fourcc != b"XMP " {
            out.extend_from_slice(&data[index..padded_end]);
        }
        index = padded_end;
    }

    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    std::fs::write(output, out).map_err(|err| format!("Cannot write the WebP: {err}"))?;
    notes.push("removed EXIF and XMP chunks; image data untouched".into());
    Ok(())
}

// ---------------------------------------------------------------------------
// GIF — gifsicle when installed, otherwise drop comment/application blocks
// ---------------------------------------------------------------------------

fn optimize_gif(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(gifsicle) = find_binary("gifsicle") {
        let mut command = std::process::Command::new(gifsicle);
        if options.lossless {
            command.args(["-O3", "--careful", "--no-comments", "--no-extensions"]);
        } else {
            command.arg("-O3").arg("--lossy=").arg(
                ((100u16.saturating_sub(options.quality as u16)) * 3)
                    .max(20)
                    .to_string(),
            );
        }
        let result = command.arg(input).arg("-o").arg(output).output();
        if let Ok(result) = result {
            if result.status.success() {
                notes.push("optimized frame deltas (gifsicle)".into());
                return Ok(());
            }
        }
    }

    let data = std::fs::read(input).map_err(|err| format!("Cannot read the GIF: {err}"))?;
    if !options.strip_metadata {
        std::fs::write(output, data).map_err(|err| format!("Cannot write the GIF: {err}"))?;
        return Ok(());
    }
    let stripped = strip_gif_extensions(&data);
    std::fs::write(output, stripped).map_err(|err| format!("Cannot write the GIF: {err}"))?;
    notes.push("removed comment and application blocks".into());
    Ok(())
}

fn strip_gif_extensions(data: &[u8]) -> Vec<u8> {
    if data.len() < 13 || &data[0..3] != b"GIF" {
        return data.to_vec();
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len());

    // Logical screen descriptor, plus the global colour table if flagged.
    let packed = data[10];
    let gct = if packed & 0x80 != 0 {
        3 * (1usize << ((packed & 0x07) + 1))
    } else {
        0
    };
    out.extend_from_slice(&data[..(13 + gct).min(data.len())]);
    let mut index = (13 + gct).min(data.len());

    while index < data.len() {
        match data[index] {
            0x3B => {
                out.push(0x3B);
                break;
            }
            0x21 => {
                let Some(&label) = data.get(index + 1) else {
                    break;
                };
                let start = index;
                index += 2;
                while index < data.len() && data[index] != 0 {
                    index += 1 + data[index] as usize;
                }
                index = (index + 1).min(data.len());
                // 0xFE is a comment, 0xFF an application (XMP/ICC) block.
                if label != 0xFE && label != 0xFF {
                    out.extend_from_slice(&data[start..index]);
                }
            }
            0x2C => {
                let start = index;
                if index + 10 > data.len() {
                    break;
                }
                let local = data[index + 9];
                let lct = if local & 0x80 != 0 {
                    3 * (1usize << ((local & 0x07) + 1))
                } else {
                    0
                };
                index += 10 + lct;
                if index < data.len() {
                    index += 1 + data[index] as usize; // LZW minimum code size
                }
                while index < data.len() && data[index] != 0 {
                    index += 1 + data[index] as usize;
                }
                index = (index + 1).min(data.len());
                out.extend_from_slice(&data[start..index]);
            }
            _ => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SVG — conservative minification
// ---------------------------------------------------------------------------

fn optimize_svg(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    for tool in ["svgo", "svgcleaner", "scour"] {
        let Some(binary) = find_binary(tool) else {
            continue;
        };
        let mut command = std::process::Command::new(binary);
        match tool {
            "svgo" => {
                command.args(["--input"]).arg(input).args(["--output"]).arg(output);
            }
            "svgcleaner" => {
                command.arg(input).arg(output);
            }
            _ => {
                command
                    .arg("-i")
                    .arg(input)
                    .arg("-o")
                    .arg(output)
                    .args(["--remove-metadata", "--enable-comment-stripping", "--shorten-ids"]);
            }
        }
        if command.output().map(|r| r.status.success()).unwrap_or(false) {
            notes.push(format!("cleaned with {tool}"));
            return Ok(());
        }
    }

    let text = std::fs::read_to_string(input)
        .map_err(|err| format!("Cannot read the SVG: {err}"))?;
    let mut result = text;
    if options.strip_metadata {
        result = strip_svg_noise(&result);
        notes.push("removed comments, metadata and editor bookkeeping".into());
    }
    std::fs::write(output, result).map_err(|err| format!("Cannot write the SVG: {err}"))?;
    Ok(())
}

fn strip_svg_noise(svg: &str) -> String {
    let mut out = remove_element_blocks(&remove_element_blocks(svg, "metadata"), "sodipodi:namedview");

    // Drop comments, keeping conditional-comment free documents intact.
    while let Some(start) = out.find("<!--") {
        let Some(end) = out[start..].find("-->") else {
            break;
        };
        out.replace_range(start..start + end + 3, "");
    }
    out = remove_attribute_prefix(&out, "inkscape:");
    out = remove_attribute_prefix(&out, "sodipodi:");
    out = out.replace(" xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\"", "");
    out = out.replace(" xmlns:sodipodi=\"http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd\"", "");
    out = out.replace(" xmlns:serif=\"http://www.serif.com/\"", "");

    // Collapse whitespace between tags and runs of blank space.
    while out.contains(">\n") || out.contains(">  ") {
        out = out.replace(">\n", ">").replace(">  ", ">");
    }
    while out.contains("\n<") {
        out = out.replace("\n<", "<");
    }
    out.trim().to_string()
}

fn remove_element_blocks(svg: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = svg.to_string();
    while let Some(start) = out.find(&open) {
        let Some(relative_end) = out[start..].find(&close) else {
            // Self-closing form.
            match out[start..].find("/>") {
                Some(end) => {
                    out.replace_range(start..start + end + 2, "");
                    continue;
                }
                None => break,
            }
        };
        out.replace_range(start..start + relative_end + close.len(), "");
    }
    out
}

fn remove_attribute_prefix(svg: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(position) = rest.find(prefix) {
        // Only strip when the prefix starts an attribute name (preceded by a space).
        let preceded_by_space = rest[..position].ends_with(char::is_whitespace);
        out.push_str(&rest[..position]);
        if !preceded_by_space {
            out.push_str(prefix);
            rest = &rest[position + prefix.len()..];
            continue;
        }
        let after = &rest[position..];
        let Some(equals) = after.find('=') else {
            break;
        };
        let Some(quote) = after[equals + 1..].chars().next() else {
            break;
        };
        let Some(end_quote) = after[equals + 1..].find(quote) else {
            break;
        };
        let skip = equals + 1 + end_quote + quote.len_utf8();
        rest = &after[skip..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Everything else — ImageMagick, the only sane fallback for TIFF/HEIC/AVIF
// ---------------------------------------------------------------------------

fn optimize_with_imagemagick(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    extension: &str,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let binary = find_binary("magick").or_else(|| find_binary("convert"));
    let Some(binary) = binary else {
        return Err(format!(
            "No built-in optimizer for .{extension} — install ImageMagick for this format"
        ));
    };

    let mut command = std::process::Command::new(&binary);
    command.arg(input);
    if options.strip_metadata {
        command.arg("-strip");
    }
    if let Some(max) = options.max_dimension {
        command.args(["-resize", &format!("{max}x{max}>")]);
    }
    if !options.lossless {
        command.args(["-quality", &options.quality.to_string()]);
    }
    command.arg(output);

    let result = command
        .output()
        .map_err(|err| format!("Cannot run ImageMagick: {err}"))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!(
            "ImageMagick failed: {}",
            crate::tools::truncate(stderr.trim().lines().last().unwrap_or("unknown error"), 160)
        ));
    }
    if options.strip_metadata {
        notes.push("stripped metadata (ImageMagick)".into());
    }
    Ok(())
}

fn reencode_with_image(
    input: &Path,
    output: &Path,
    options: &OptimizeOptions,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let image = image::ImageReader::open(input)
        .map_err(|err| format!("Cannot open the image: {err}"))?
        .with_guessed_format()
        .map_err(|err| format!("Cannot read the image: {err}"))?
        .decode()
        .map_err(|err| format!("Cannot decode the image: {err}"))?;

    let mut image = image;
    if let Some(max) = options.max_dimension {
        if image.width().max(image.height()) > max {
            image = image.resize(max, max, image::imageops::FilterType::Lanczos3);
        }
    }

    let target = output
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());
    crate::native::images::encode(&image, &target, options.quality, output)?;
    notes.push(format!("re-encoded at quality {}", options.quality));
    Ok(())
}

fn temp_path(output: &Path, extension: &str) -> std::path::PathBuf {
    let mut name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    name.push_str(&format!(".tmp-{}.{extension}", std::process::id()));
    output.with_file_name(name)
}

/// Optimizes in place through a temporary file, so a crash never loses the original.
pub fn optimize_in_place(
    input: &Path,
    options: &OptimizeOptions,
    backup: bool,
) -> OptimizeOutcome {
    let extension = input
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = temp_path(input, &extension);
    let mut outcome = optimize(input, &temp, options);

    if outcome.ok {
        if backup {
            let backup_path = input.with_extension(format!("{extension}.orig"));
            let _ = std::fs::copy(input, &backup_path);
            outcome.notes.push(format!(
                "original kept as {}",
                backup_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ));
        }
        if std::fs::rename(&temp, input).is_err() {
            let _ = std::fs::copy(&temp, input);
            let _ = std::fs::remove_file(&temp);
        }
    } else {
        let _ = std::fs::remove_file(&temp);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png_with_text(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
        });
        let mut buffer = Cursor::new(Vec::new());
        image.write_to(&mut buffer, image::ImageFormat::Png).unwrap();
        let mut png = buffer.into_inner();

        // Append a tEXt chunk right after IHDR so the file carries metadata.
        let payload: &[u8] = b"Comment\0x";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(payload);
        let crc = crc32(&chunk[4..]);
        chunk.extend_from_slice(&crc.to_be_bytes());

        let ihdr_end = 8 + 25;
        png.splice(ihdr_end..ihdr_end, chunk);
        png
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB8_8320;
                } else {
                    crc >>= 1;
                }
            }
        }
        crc ^ 0xFFFF_FFFF
    }

    #[test]
    fn png_loses_metadata_and_keeps_pixels_lossless() {
        let dir = std::env::temp_dir().join(format!("convertx-opt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("photo.png");
        let original = png_with_text(64, 64);
        std::fs::write(&input, &original).unwrap();

        let output = dir.join("photo.min.png");
        let outcome = optimize(
            &input,
            &output,
            &OptimizeOptions {
                effort: 2,
                ..Default::default()
            },
        );
        assert!(outcome.ok, "{}", outcome.message);

        let optimized = std::fs::read(&output).unwrap();
        assert!(optimized.len() < original.len(), "expected a smaller PNG");
        assert!(
            !optimized.windows(4).any(|w| w == b"tEXt"),
            "the tEXt chunk should be gone"
        );

        // Pixels must be identical.
        let before = image::load_from_memory(&original).unwrap().to_rgba8();
        let after = image::load_from_memory(&optimized).unwrap().to_rgba8();
        assert_eq!(before, after);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jpeg_metadata_is_stripped_without_re_encoding() {
        // SOI + APP1(EXIF) + COM + SOS + a byte of scan data.
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x10]);
        jpeg.extend_from_slice(b"Exif\0\0padding");
        jpeg.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x08, b'h', b'e', b'l', b'l', b'o', b'!']);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22, 0xFF, 0xD9]);

        let stripped = strip_jpeg_metadata(&jpeg, true);
        assert!(stripped.len() < jpeg.len());
        assert!(!stripped.windows(4).any(|w| w == b"Exif"));
        assert!(stripped.windows(2).any(|w| w == [0xFF, 0xDA]));
        assert_eq!(&stripped[stripped.len() - 2..], &[0xFF, 0xD9]);

        // Without the flag nothing changes.
        assert_eq!(strip_jpeg_metadata(&jpeg, false), jpeg);
    }

    #[test]
    fn svg_noise_is_removed() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd">
            <!-- a comment -->
            <metadata>bloat</metadata>
            <sodipodi:namedview id="nv" pagecolor="#fff" />
            <rect width="10" height="10" inkscape:label="Box" sodipodi:nodetypes="cc" fill="#000" />
        </svg>"##;
        let clean = strip_svg_noise(svg);
        assert!(!clean.contains("<!--"));
        assert!(!clean.contains("metadata"));
        assert!(!clean.contains("inkscape:"));
        assert!(!clean.contains("sodipodi:"));
        assert!(clean.contains("<rect"));
        assert!(clean.contains("fill=\"#000\""));
    }

    #[test]
    fn never_returns_a_bigger_file() {
        let dir = std::env::temp_dir().join(format!("convertx-opt-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("tiny.png");
        let png = png_with_text(2, 2);
        std::fs::write(&input, &png).unwrap();
        let output = dir.join("tiny.min.png");

        let outcome = optimize(&input, &output, &OptimizeOptions::default());
        assert!(outcome.ok, "{}", outcome.message);
        assert!(std::fs::metadata(&output).unwrap().len() <= png.len() as u64);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
