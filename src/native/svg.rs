//! SVG rasterisation through `resvg`, so SVG works without Inkscape or librsvg.

use super::{failure, success};
use crate::convert::{ConvertRequest, ConvertResult};
use resvg::{tiny_skia, usvg};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// System fonts are expensive to enumerate, so build the database only once.
fn system_fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            database.load_system_fonts();
            Arc::new(database)
        })
        .clone()
}

pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult {
    let data = match std::fs::read(&request.input) {
        Ok(data) => data,
        Err(err) => {
            return failure(
                format!("Cannot read {}: {err}", request.input.display()),
                started,
            );
        }
    };

    let options = usvg::Options {
        resources_dir: request.input.parent().map(|dir| dir.to_path_buf()),
        // Without a font database, <text> elements silently disappear.
        fontdb: system_fonts(),
        ..Default::default()
    };

    let tree = match usvg::Tree::from_data(&data, &options) {
        Ok(tree) => tree,
        Err(err) => {
            return failure(
                format!("Invalid SVG in {}: {err}", request.input.display()),
                started,
            );
        }
    };

    let scale = request.options.svg_scale.clamp(0.01, 50.0);
    let width = (tree.size().width() * scale).ceil().max(1.0) as u32;
    let height = (tree.size().height() * scale).ceil().max(1.0) as u32;
    if width == 0 || height == 0 || width > 30_000 || height > 30_000 {
        return failure(format!("Unsupported SVG size {width}x{height}"), started);
    }

    let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
        return failure("Cannot allocate the render target", started);
    };
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let png = match pixmap.encode_png() {
        Ok(png) => png,
        Err(err) => return failure(format!("Cannot build the raster: {err}"), started),
    };

    if request.to == "png" {
        return match std::fs::write(&request.output, &png) {
            Ok(_) => success("Converted", &request.output, started),
            Err(err) => failure(
                format!("Cannot write {}: {err}", request.output.display()),
                started,
            ),
        };
    }

    let image = match image::load_from_memory_with_format(&png, image::ImageFormat::Png) {
        Ok(image) => image,
        Err(err) => return failure(format!("Cannot read the raster back: {err}"), started),
    };

    match super::images::encode(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::ConvertOptions;

    #[test]
    fn rasterises_svg() {
        let dir = std::env::temp_dir().join(format!("convertx-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("logo.svg");
        std::fs::write(
            &input,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32">
                  <rect width="64" height="32" fill="#3b82f6"/>
                </svg>"##,
        )
        .unwrap();

        for target in ["png", "jpeg", "webp"] {
            let request = ConvertRequest {
                job_id: 1,
                input: input.clone(),
                from: "svg".into(),
                to: target.into(),
                converter_id: "native-svg".into(),
                output: dir.join(format!("logo.{target}")),
                options: ConvertOptions::default(),
            };
            let result = convert(&request, Instant::now());
            assert!(result.ok, "svg -> {target}: {}", result.message);
            assert!(std::fs::metadata(&request.output).unwrap().len() > 0);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
