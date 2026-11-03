//! Converters implemented in Rust, so they work on any machine without extra installs.

pub mod docs;
pub mod images;
pub mod sheets;
pub mod svg;

use crate::convert::{ConvertRequest, ConvertResult};
use std::time::Instant;

pub const IMAGES: &str = "native-images";
pub const SVG: &str = "native-svg";
pub const SHEETS: &str = "native-sheets";
pub const DOCS: &str = "native-docs";

pub fn handles(converter_id: &str) -> bool {
    matches!(converter_id, IMAGES | SVG | SHEETS | DOCS)
}

pub fn run(converter_id: &str, request: &ConvertRequest, started: Instant) -> ConvertResult {
    match converter_id {
        IMAGES => images::convert(request, started),
        SVG => svg::convert(request, started),
        SHEETS => sheets::convert(request, started),
        DOCS => docs::convert(request, started),
        other => ConvertResult {
            ok: false,
            message: format!("Unknown built-in converter '{other}'"),
            ..Default::default()
        },
    }
}

/// Turns any error into the standard failed result, keeping the elapsed time.
pub(crate) fn failure(message: impl Into<String>, started: Instant) -> ConvertResult {
    ConvertResult {
        ok: false,
        message: message.into(),
        duration: started.elapsed(),
        ..Default::default()
    }
}

pub(crate) fn success(
    message: impl Into<String>,
    output: &std::path::Path,
    started: Instant,
) -> ConvertResult {
    ConvertResult {
        ok: true,
        message: message.into(),
        output: Some(output.to_path_buf()),
        duration: started.elapsed(),
        ..Default::default()
    }
}
