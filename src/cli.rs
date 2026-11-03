//! Headless mode: `convertx --to pdf *.docx`, `convertx --optimize *.png`.
//! Without arguments the GUI starts as usual.

use crate::convert::{self, ConvertOptions, ConvertRequest};
use crate::optimize::{self, OptimizeOptions};
use crate::registry::{canonical_format, human_size, normalize_input, normalize_output, Registry};
use crate::settings::Settings;
use crate::tools::Toolchain;
use std::path::{Path, PathBuf};

const HELP: &str = "\
ConvertX — native file conversion

USAGE
  convertx                          open the window
  convertx <files…>                 open the window with files queued
  convertx --to <format> <files…>   convert without opening the window
  convertx --optimize <files…>      shrink images without opening the window

OPTIONS
  --to <format>      target format, e.g. pdf, jpg, mp3, csv
  --out <dir>        output folder (defaults to the one in settings)
  --optimize         optimize images instead of converting
  --quality <1-100>  use lossy re-encoding (default is lossless)
  --max <px>         shrink the long edge when optimizing
  --in-place         replace the original files when optimizing
  --backup           keep a .orig copy when optimizing in place
  --list             show converters and whether they are installed
  --help             this message

EXAMPLES
  convertx --to jpg ~/Pictures/*.heic
  convertx --to pdf report.docx notes.odt
  convertx --optimize --in-place ~/site/assets/*.png
";

#[derive(Debug, Default)]
struct Args {
    to: Option<String>,
    out: Option<PathBuf>,
    optimize: bool,
    quality: Option<u8>,
    max: Option<u32>,
    in_place: bool,
    backup: bool,
    list: bool,
    help: bool,
    files: Vec<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--to" | "-t" => {
                index += 1;
                parsed.to = Some(
                    args.get(index)
                        .ok_or("--to needs a format, e.g. --to jpg")?
                        .to_lowercase(),
                );
            }
            "--out" | "-o" => {
                index += 1;
                parsed.out =
                    Some(PathBuf::from(args.get(index).ok_or("--out needs a folder")?));
            }
            "--optimize" | "-O" => parsed.optimize = true,
            "--quality" | "-q" => {
                index += 1;
                parsed.quality = Some(
                    args.get(index)
                        .ok_or("--quality needs a number")?
                        .parse::<u8>()
                        .map_err(|_| "--quality must be a number between 1 and 100")?,
                );
            }
            "--max" => {
                index += 1;
                parsed.max = Some(
                    args.get(index)
                        .ok_or("--max needs a pixel value")?
                        .parse::<u32>()
                        .map_err(|_| "--max must be a number of pixels")?,
                );
            }
            "--in-place" | "-i" => parsed.in_place = true,
            "--backup" => parsed.backup = true,
            "--list" | "-l" => parsed.list = true,
            "--help" | "-h" => parsed.help = true,
            other if other.starts_with('-') => {
                return Err(format!("Unknown option '{other}' — try --help"));
            }
            other => parsed.files.push(PathBuf::from(other)),
        }
        index += 1;
    }
    Ok(parsed)
}

/// Returns `Some(exit_code)` when the CLI handled the request.
pub fn maybe_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has_flag = args.iter().any(|arg| arg.starts_with('-'));
    if args.is_empty() || !has_flag {
        // Plain file list (or nothing): hand over to the GUI.
        return None;
    }

    let parsed = match parse(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("error: {err}");
            return Some(2);
        }
    };

    if parsed.help {
        println!("{HELP}");
        return Some(0);
    }

    let registry = Registry::get();
    let tools = Toolchain::detect(registry.converters());

    if parsed.list {
        list(&tools);
        return Some(0);
    }

    if parsed.files.is_empty() {
        eprintln!("error: no input files — try --help");
        return Some(2);
    }

    let settings = Settings::load();
    if parsed.optimize {
        Some(optimize_files(&parsed, &settings))
    } else {
        let Some(target) = parsed.to.clone() else {
            eprintln!("error: --to <format> is required unless you pass --optimize");
            return Some(2);
        };
        Some(convert_files(&parsed, &target, &tools, &settings))
    }
}

fn output_dir(parsed: &Args, settings: &Settings) -> PathBuf {
    parsed.out.clone().unwrap_or_else(|| settings.output_path())
}

fn convert_files(parsed: &Args, target: &str, tools: &Toolchain, settings: &Settings) -> i32 {
    let registry = Registry::get();
    let target = &canonical_format(target);
    let dir = output_dir(parsed, settings);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {err}", dir.display());
        return 1;
    }

    let options = ConvertOptions {
        svg_scale: settings.svg_scale,
        jpeg_quality: parsed.quality.unwrap_or(settings.jpeg_quality),
        ffmpeg_input_args: settings.ffmpeg_input_args.clone(),
        ffmpeg_output_args: settings.ffmpeg_output_args.clone(),
        timeout_secs: settings.convert_timeout_secs,
    };

    let extension = normalize_output(target);
    let mut failures = 0usize;
    let mut taken: Vec<PathBuf> = Vec::new();

    for input in &parsed.files {
        let Some(raw) = registry.detect_extension(input) else {
            eprintln!("✗ {} — unknown file type", input.display());
            failures += 1;
            continue;
        };
        let from = normalize_input(&raw);
        if !registry.is_known_input(&from) {
            eprintln!("✗ {} — nothing can read .{raw}", input.display());
            failures += 1;
            continue;
        }
        if registry.converters_for(&from, target).is_empty() {
            eprintln!("✗ {} — cannot convert {from} to {target}", input.display());
            failures += 1;
            continue;
        }

        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output".into());
        let output = convert::unique_output_path(&dir, &stem, &extension, &taken);
        taken.push(output.clone());

        let request = ConvertRequest {
            job_id: 0,
            input: input.clone(),
            from,
            to: target.to_string(),
            converter_id: String::new(),
            output: output.clone(),
            options: options.clone(),
        };

        let result = convert::run_chain(&request, tools);
        if result.ok {
            let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
            println!(
                "✓ {} → {} ({}, via {})",
                file_name(input),
                output
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                human_size(size),
                result.converter.as_deref().unwrap_or("converter")
            );
        } else {
            eprintln!("✗ {} — {}", file_name(input), result.message);
            failures += 1;
        }
    }

    if failures == 0 {
        0
    } else {
        1
    }
}

fn optimize_files(parsed: &Args, settings: &Settings) -> i32 {
    let dir = output_dir(parsed, settings);
    if !parsed.in_place {
        if let Err(err) = std::fs::create_dir_all(&dir) {
            eprintln!("error: cannot create {}: {err}", dir.display());
            return 1;
        }
    }

    let options = OptimizeOptions {
        lossless: parsed.quality.is_none(),
        quality: parsed.quality.unwrap_or(82),
        strip_metadata: true,
        max_dimension: parsed.max,
        effort: 4,
    };

    let mut failures = 0usize;
    let mut saved_total: u64 = 0;
    let mut taken: Vec<PathBuf> = Vec::new();

    for input in &parsed.files {
        let extension = input
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        if !optimize::is_optimizable(&extension) {
            eprintln!("✗ {} — .{extension} cannot be optimized", file_name(input));
            failures += 1;
            continue;
        }

        let outcome = if parsed.in_place {
            optimize::optimize_in_place(input, &options, parsed.backup)
        } else {
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".into());
            let output = convert::unique_output_path(&dir, &format!("{stem}-min"), &extension, &taken);
            taken.push(output.clone());
            optimize::optimize(input, &output, &options)
        };

        if outcome.ok {
            saved_total += outcome.saved();
            println!("✓ {} — {}", file_name(input), outcome.summary());
            for note in &outcome.notes {
                println!("    {note}");
            }
        } else {
            eprintln!("✗ {} — {}", file_name(input), outcome.message);
            failures += 1;
        }
    }

    if saved_total > 0 {
        println!("\nSaved {} in total", human_size(saved_total));
    }
    if failures == 0 {
        0
    } else {
        1
    }
}

fn list(tools: &Toolchain) {
    let registry = Registry::get();
    let mut ready = 0usize;
    for spec in registry.converters() {
        let installed = tools.is_installed(&spec.id);
        if installed {
            ready += 1;
        }
        let detail = if spec.builtin {
            "built in".to_string()
        } else if installed {
            tools
                .status_of(&spec.id)
                .and_then(|s| s.version.clone())
                .unwrap_or_else(|| "installed".into())
        } else {
            format!(
                "missing: {}",
                tools
                    .status_of(&spec.id)
                    .map(|s| s.missing.join(", "))
                    .unwrap_or_default()
            )
        };
        println!(
            "{} {:24} {}",
            if installed { "●" } else { "○" },
            spec.id,
            detail
        );
    }
    println!(
        "\n{ready} of {} converters ready — {} input formats, {} targets",
        registry.converters().len(),
        registry.input_formats().count(),
        registry.all_targets().len()
    );
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
