use crate::registry::{Registry, normalize_output};
use crate::tools::Toolchain;
use crate::vcf;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Scale factor used when rasterising SVG and vips output.
    pub svg_scale: f32,
    pub jpeg_quality: u8,
    pub ffmpeg_input_args: String,
    pub ffmpeg_output_args: String,
    /// Kill a converter that runs longer than this. Zero disables the watchdog.
    pub timeout_secs: u64,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            svg_scale: 1.0,
            jpeg_quality: 90,
            ffmpeg_input_args: String::new(),
            ffmpeg_output_args: String::new(),
            timeout_secs: 1800,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConvertRequest {
    pub job_id: u64,
    pub input: PathBuf,
    pub from: String,
    pub to: String,
    pub converter_id: String,
    pub output: PathBuf,
    pub options: ConvertOptions,
}

#[derive(Debug, Clone, Default)]
pub struct ConvertResult {
    pub ok: bool,
    pub message: String,
    pub output: Option<PathBuf>,
    pub log: String,
    pub duration: Duration,
    /// Converter that actually produced the file (may differ from the preferred one).
    pub converter: Option<String>,
    /// Every converter that was tried and failed, with a short reason.
    pub attempts: Vec<(String, String)>,
}

enum OutputMode {
    /// The converter writes straight to the target path.
    Direct,
    /// The converter writes to stdout; we store it ourselves.
    Stdout(PathBuf),
    /// The converter derives the file name from the input and writes into a directory.
    OutDir { dir: PathBuf, extension: String },
}

pub fn run(request: &ConvertRequest, tools: &Toolchain) -> ConvertResult {
    let started = Instant::now();
    let registry = Registry::get();

    let Some(spec) = registry.converter(&request.converter_id) else {
        return ConvertResult {
            ok: false,
            message: format!("Unknown converter '{}'", request.converter_id),
            duration: started.elapsed(),
            ..Default::default()
        };
    };

    if let Some(parent) = request.output.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return ConvertResult {
            ok: false,
            message: format!("Cannot create output folder: {err}"),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    if crate::native::handles(&spec.id) {
        return crate::native::run(&spec.id, request, started);
    }

    if spec.builtin {
        return vcf::convert(request, started);
    }

    if !tools.is_installed(&spec.id) {
        let missing = tools
            .status_of(&spec.id)
            .map(|s| s.missing.join(", "))
            .unwrap_or_default();
        return ConvertResult {
            ok: false,
            message: format!("{} is not installed (missing: {missing})", spec.name),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    let mut staging: Option<PathBuf> = None;
    let (command, mode) = match build_command(request, tools) {
        Ok(built) => built,
        Err(message) => {
            return ConvertResult {
                ok: false,
                message,
                duration: started.elapsed(),
                ..Default::default()
            };
        }
    };

    if let OutputMode::OutDir { dir, .. } = &mode {
        staging = Some(dir.clone());
    }

    let output = run_child(command, request.options.timeout_secs);
    let output = match output {
        Ok(output) => output,
        Err(err) => {
            cleanup(staging.as_deref());
            return ConvertResult {
                ok: false,
                message: format!("Could not run {}: {err}", spec.name),
                duration: started.elapsed(),
                ..Default::default()
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    let mut result = ConvertResult {
        ok: output.status.success() && !output.timed_out,
        message: String::new(),
        output: None,
        log: tail(&format!("{stdout}\n{stderr}"), 4000),
        duration: started.elapsed(),
        converter: Some(spec.id.clone()),
        attempts: Vec::new(),
    };

    if result.ok {
        let produced = match &mode {
            OutputMode::Direct => {
                if request.output.exists() {
                    Ok(request.output.clone())
                } else {
                    promote_numbered_output(&request.output)
                }
            }
            OutputMode::Stdout(path) => std::fs::write(path, &output.stdout)
                .map(|_| path.clone())
                .map_err(|err| err.to_string()),
            OutputMode::OutDir { dir, extension } => locate_output(dir, extension).map(|file| {
                if file != request.output {
                    let _ = std::fs::rename(&file, &request.output);
                }
                request.output.clone()
            }),
        };

        match produced {
            Ok(path) if path.exists() => {
                // An empty text file usually means there was no text layer to read.
                if request.to == "txt" && is_blank(&path) {
                    result.message = "No text found — nothing readable in this file".into();
                } else {
                    result.message = "Converted".into();
                }
                result.output = Some(path);
            }
            Ok(path) => {
                result.ok = false;
                result.message = format!(
                    "{} finished but did not produce {}",
                    spec.name,
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                );
            }
            Err(err) => {
                result.ok = false;
                result.message = err;
            }
        }
    } else if output.timed_out {
        result.message = format!(
            "{} was still running after {} s and was stopped",
            spec.name, request.options.timeout_secs
        );
    } else {
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or("the converter exited with an error");
        result.message = format!(
            "{} failed: {}",
            spec.name,
            crate::tools::truncate(detail, 160)
        );
    }

    cleanup(staging.as_deref());
    result
}

/// Runs the conversion, falling back to the next converter that supports the
/// route when the preferred one fails. This is what keeps a batch going when a
/// single tool chokes on a particular file.
pub fn run_chain(request: &ConvertRequest, tools: &Toolchain) -> ConvertResult {
    let started = Instant::now();
    let registry = Registry::get();
    let candidates = registry.converters_for(&request.from, &request.to);

    if candidates.is_empty() {
        return ConvertResult {
            ok: false,
            message: format!("No converter supports {} → {}", request.from, request.to),
            duration: started.elapsed(),
            ..Default::default()
        };
    }

    let installed: Vec<&crate::registry::ConverterSpec> = candidates
        .iter()
        .copied()
        .filter(|spec| tools.is_installed(&spec.id))
        .collect();

    if installed.is_empty() {
        let best = candidates[0];
        let missing = tools
            .status_of(&best.id)
            .map(|status| status.missing.join(", "))
            .unwrap_or_default();
        return ConvertResult {
            ok: false,
            message: if missing.is_empty() {
                format!("{} is not available", best.name)
            } else {
                format!("{} is not installed (missing: {missing})", best.name)
            },
            duration: started.elapsed(),
            converter: Some(best.id.clone()),
            ..Default::default()
        };
    }

    let mut attempts: Vec<(String, String)> = Vec::new();
    for spec in installed {
        if attempts.is_empty() && spec.id == request.converter_id {
            // Preferred choice: no need to reorder anything.
        }
        let mut attempt = request.clone();
        attempt.converter_id = spec.id.clone();
        let _ = std::fs::remove_file(&attempt.output);

        let mut result = run(&attempt, tools);
        result.converter = Some(spec.id.clone());

        if result.ok {
            match validate_output(&attempt) {
                Ok(()) => {
                    if !attempts.is_empty() {
                        result.attempts = attempts;
                        result.message = format!(
                            "{} ({} fell through)",
                            result.message,
                            result
                                .attempts
                                .first()
                                .map(|(id, _)| id.clone())
                                .unwrap_or_default()
                        );
                    }
                    return result;
                }
                Err(reason) => {
                    let _ = std::fs::remove_file(&attempt.output);
                    attempts.push((spec.id.clone(), reason.clone()));
                    result.ok = false;
                    result.message = reason;
                }
            }
        } else {
            attempts.push((spec.id.clone(), result.message.clone()));
        }
    }

    let summary = attempts
        .iter()
        .map(|(id, reason)| format!("{id}: {reason}"))
        .collect::<Vec<_>>()
        .join("; ");

    ConvertResult {
        ok: false,
        message: format!(
            "All converters failed — {}",
            crate::tools::truncate(&summary, 400)
        ),
        duration: started.elapsed(),
        attempts,
        ..Default::default()
    }
}

/// Catches converters that exit cleanly but leave an empty or corrupt file.
pub fn validate_output(request: &ConvertRequest) -> Result<(), String> {
    let meta = std::fs::metadata(&request.output)
        .map_err(|_| format!("No output file was written for .{}", request.to))?;
    // An empty .txt is a legitimate answer from OCR or a text extractor.
    if meta.len() == 0 && request.to != "txt" {
        return Err("The converter produced an empty file".into());
    }

    match request.to.as_str() {
        "png" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" => {
            let dimensions = image::image_dimensions(&request.output)
                .map_err(|err| format!("The output image is unreadable: {err}"))?;
            if dimensions.0 == 0 || dimensions.1 == 0 {
                return Err("The output image has no pixels".into());
            }
        }
        "pdf" => {
            let head = read_head(&request.output, 5)?;
            if !head.starts_with(b"%PDF") {
                return Err("The output is not a valid PDF".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_blank(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.trim().len() < 2)
        .unwrap_or(true)
}

fn read_head(path: &Path, length: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut buffer = vec![0u8; length];
    let mut file =
        std::fs::File::open(path).map_err(|err| format!("Cannot open the output: {err}"))?;
    let read = file
        .read(&mut buffer)
        .map_err(|err| format!("Cannot read the output: {err}"))?;
    buffer.truncate(read);
    Ok(buffer)
}

/// ImageMagick and friends write `name-0.png`, `name-1.png`… for multi-page
/// sources. Promote the first page to the requested name and leave the rest, so
/// `pdf -> png` gives page one at the expected path instead of failing.
fn promote_numbered_output(target: &Path) -> Result<PathBuf, String> {
    let Some(parent) = target.parent() else {
        return Err("No parent folder for the output".into());
    };
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = target
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut pages: Vec<(u32, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let Some(name) = name else { continue };
            let Some(rest) = name.strip_prefix(&format!("{stem}-")) else {
                continue;
            };
            let Some(number) = rest.strip_suffix(&format!(".{extension}")) else {
                continue;
            };
            if let Ok(index) = number.parse::<u32>() {
                pages.push((index, path));
            }
        }
    }

    if pages.is_empty() {
        return Err("The converter did not write any output".into());
    }
    pages.sort_by_key(|(index, _)| *index);
    let (_, first) = pages.remove(0);
    std::fs::rename(&first, target).map_err(|err| format!("Cannot rename the output: {err}"))?;
    Ok(target.to_path_buf())
}

struct ChildOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

/// Runs a converter, killing it if it hangs. LibreOffice in particular can wait
/// forever on a locked profile, which would otherwise stall the whole batch.
fn run_child(mut command: Command, timeout_secs: u64) -> std::io::Result<ChildOutput> {
    use std::io::Read;

    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn()?;

    // Pipe readers report through a channel: a killed converter can leave a
    // grandchild holding the pipe open, and joining it would block forever.
    let (tx, rx) = std::sync::mpsc::channel();
    let spawn_reader = |index: usize, mut pipe: Option<Box<dyn Read + Send>>| {
        let sender = tx.clone();
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(pipe) = pipe.as_mut() {
                let _ = pipe.read_to_end(&mut buffer);
            }
            let _ = sender.send((index, buffer));
        });
    };
    spawn_reader(
        0,
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    spawn_reader(
        1,
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    drop(tx);

    let deadline = if timeout_secs == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_secs(timeout_secs))
    };

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => return Err(err),
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            kill_tree(&mut child);
            timed_out = true;
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(40));
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let collect =
        |stream: usize, buffer: Vec<u8>| -> Option<(usize, Vec<u8>)> { Some((stream, buffer)) };
    let _ = collect;
    let grace = Instant::now() + Duration::from_millis(500);
    while Instant::now() < grace {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok((0, buffer)) => stdout = buffer,
            Ok((_, buffer)) => stderr = buffer,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(ChildOutput {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

/// Stops a converter and anything it spawned, so its pipes are released.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID"])
            .arg(child.id().to_string())
            .output();
    }
    let _ = child.kill();
}

fn cleanup(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn locate_output(dir: &Path, extension: &str) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut best: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let matches = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case(extension))
            .unwrap_or(false);
        if matches {
            best = Some(path);
            break;
        }
    }
    best.ok_or_else(|| format!("No .{extension} output was produced"))
}

fn tail(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= max {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - max;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &trimmed[start..])
}

fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("convertx-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn split_args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn build_command(
    request: &ConvertRequest,
    tools: &Toolchain,
) -> Result<(Command, OutputMode), String> {
    let input = request.input.to_string_lossy().into_owned();
    let target = request.output.to_string_lossy().into_owned();
    let from = request.from.as_str();
    let to = request.to.as_str();
    let options = &request.options;

    let built = match request.converter_id.as_str() {
        "inkscape" => {
            let mut cmd = tools.command("inkscape");
            cmd.arg(&input).arg("-o").arg(&target);
            (cmd, OutputMode::Direct)
        }
        "libjxl" => {
            let binary = if from == "jxl" { "djxl" } else { "cjxl" };
            let mut cmd = tools.command(binary);
            cmd.arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        "resvg" => {
            let mut cmd = tools.command("resvg");
            cmd.arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        "vips" => {
            let mut cmd = tools.command("vips");
            let action = if from == "pdf" { "pdfload" } else { "copy" };
            cmd.arg(action).arg(&input).arg(&target);
            if from == "svg" && (options.svg_scale - 1.0).abs() > f32::EPSILON {
                cmd.arg("--scale")
                    .arg(format!("{:.3}", options.svg_scale.max(0.01)));
            }
            (cmd, OutputMode::Direct)
        }
        "libheif" => {
            let mut cmd = tools.command("heif-convert");
            cmd.arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        "xelatex" => {
            let dir = temp_dir();
            let mut cmd = tools.command("latexmk");
            cmd.arg("-xelatex")
                .arg("-interaction=nonstopmode")
                .arg(format!("-output-directory={}", dir.to_string_lossy()))
                .arg(&input);
            (
                cmd,
                OutputMode::OutDir {
                    dir,
                    extension: normalize_output(to),
                },
            )
        }
        "djvu" => {
            let mut cmd = tools.command("ddjvu");
            cmd.arg(format!("-format={to}")).arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        "calibre" => {
            let mut cmd = tools.command("ebook-convert");
            cmd.arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        "dasel" => {
            let mut cmd = tools.command("dasel");
            cmd.arg("--var")
                .arg(format!("data={from}:file:{input}"))
                .arg("--out")
                .arg(to)
                .arg("$data");
            (cmd, OutputMode::Stdout(request.output.clone()))
        }
        "libreoffice" => {
            let dir = temp_dir();
            let (in_filter, out_filter) = libreoffice_filters(from, to);
            let mut cmd = tools.command("soffice");
            cmd.arg("--headless").arg(format!(
                "-env:UserInstallation=file://{}/profile",
                dir.to_string_lossy()
            ));
            if let Some(filter) = in_filter {
                cmd.arg(format!("--infilter={filter}"));
            }
            match out_filter {
                Some(filter) => {
                    cmd.arg("--convert-to").arg(format!("{to}:{filter}"));
                }
                None => {
                    cmd.arg("--convert-to").arg(to);
                }
            }
            cmd.arg("--outdir").arg(&dir).arg(&input);
            (
                cmd,
                OutputMode::OutDir {
                    dir,
                    extension: normalize_output(to),
                },
            )
        }
        "pandoc" => {
            let mut cmd = tools.command("pandoc");
            if matches!(to, "pdf" | "latex") {
                cmd.arg("--pdf-engine=xelatex");
            }
            cmd.arg(&input)
                .arg("-f")
                .arg(from)
                .arg("-t")
                .arg(to)
                .arg("-o")
                .arg(&target);
            (cmd, OutputMode::Direct)
        }
        "msgconvert" => {
            let mut cmd = tools.command("msgconvert");
            cmd.arg("--outfile").arg(&target).arg(&input);
            (cmd, OutputMode::Direct)
        }
        "dvisvgm" => {
            let mut cmd = tools.command("dvisvgm");
            if from == "eps" {
                cmd.arg("--eps");
            }
            if from == "pdf" {
                cmd.arg("--pdf");
            }
            if to == "svgz" {
                cmd.arg("-z");
            }
            cmd.arg(&input).arg("-o").arg(&target);
            (cmd, OutputMode::Direct)
        }
        "imagemagick" => {
            let mut cmd = tools.command("magick");
            if to == "ico" {
                cmd.arg("-define")
                    .arg("icon:auto-resize=256,128,64,48,32,16")
                    .arg("-background")
                    .arg("none");
                if from == "svg" {
                    cmd.arg("-background")
                        .arg("none")
                        .arg("-density")
                        .arg("512");
                }
            }
            if from == "pdf" {
                cmd.arg("-density").arg("300");
            }
            if from == "emf" {
                cmd.arg("-define")
                    .arg("emf:delegate=false")
                    .arg("-density")
                    .arg("300");
            }
            // HEIC/AVIF files from phones carry depth and gain maps beside the photo;
            // reading frame 0 keeps one input → one output.
            let single_frame = matches!(from, "heic" | "heif" | "avif");
            cmd.arg(if single_frame {
                format!("{input}[0]")
            } else {
                input.clone()
            });
            if from == "emf" {
                cmd.arg("-background")
                    .arg("white")
                    .arg("-alpha")
                    .arg("remove");
            }
            if to == "pdf" {
                // Uncompressed image data makes absurdly large PDFs.
                cmd.arg("-compress").arg("jpeg").arg("-quality").arg("92");
            }
            if to == "jpg" || to == "jpeg" {
                cmd.arg("-quality")
                    .arg(options.jpeg_quality.clamp(1, 100).to_string());
            }
            cmd.arg("-auto-orient").arg(&target);
            (cmd, OutputMode::Direct)
        }
        "graphicsmagick" => {
            let mut cmd = tools.command("gm");
            cmd.arg("convert")
                .arg(&input)
                .arg("-auto-orient")
                .arg(&target);
            (cmd, OutputMode::Direct)
        }
        "assimp" => {
            let mut cmd = tools.command("assimp");
            cmd.arg("export")
                .arg(&input)
                .arg(&target)
                .arg(format!("-f{to}"));
            (cmd, OutputMode::Direct)
        }
        "ffmpeg" => {
            let mut cmd = tools.command("ffmpeg");
            for arg in split_args(&options.ffmpeg_input_args) {
                cmd.arg(arg);
            }
            cmd.arg("-i").arg(&input);
            for arg in split_args(&options.ffmpeg_output_args) {
                cmd.arg(arg);
            }

            let audio_with_cover = ["m4a", "m4b", "mp3", "flac", "ogg"];
            let audio_without_cover = ["wav", "aac"];
            if audio_with_cover.contains(&to) {
                cmd.args([
                    "-map",
                    "0:a",
                    "-map",
                    "0:v:disp:attached_pic?",
                    "-c:v",
                    "copy",
                    "-disposition:v",
                    "attached_pic",
                ]);
            } else if audio_without_cover.contains(&to) {
                cmd.arg("-vn");
            }

            if to == "ico" {
                cmd.args([
                    "-filter:v",
                    "scale='min(256,iw)':min'(256,ih)':force_original_aspect_ratio=decrease",
                ]);
            }

            if let Some((codec, _container)) = to.split_once('.') {
                let encoder = match codec {
                    "av1" => Some("libaom-av1"),
                    "h264" => Some("libx264"),
                    "h265" => Some("libx265"),
                    "h266" => Some("libx266"),
                    _ => None,
                };
                if let Some(encoder) = encoder {
                    cmd.arg("-c:v").arg(encoder);
                }
            }

            cmd.arg(&target);
            (cmd, OutputMode::Direct)
        }
        "potrace" => {
            let mut cmd = tools.command("potrace");
            cmd.arg(&input).arg("-o").arg(&target).arg("-b").arg(to);
            (cmd, OutputMode::Direct)
        }
        "vtracer" => {
            let mut cmd = tools.command("vtracer");
            cmd.arg("--input").arg(&input).arg("--output").arg(&target);
            (cmd, OutputMode::Direct)
        }
        "markitdown" => {
            let mut cmd = tools.command("markitdown");
            cmd.arg(&input).arg("-o").arg(&target);
            (cmd, OutputMode::Direct)
        }
        "pdftotext" => {
            let mut cmd = tools.command("pdftotext");
            cmd.arg("-layout")
                .arg("-enc")
                .arg("UTF-8")
                .arg(&input)
                .arg(&target);
            (cmd, OutputMode::Direct)
        }
        "pdftoppm" => {
            let mut cmd = tools.command("pdftoppm");
            // -r keeps text legible without exploding the file size.
            cmd.arg("-r").arg("150");
            match to {
                "jpeg" => {
                    cmd.arg("-jpeg").arg("-jpegopt").arg("quality=90");
                }
                "tiff" => {
                    cmd.arg("-tiff");
                }
                _ => {
                    cmd.arg("-png");
                }
            }
            // Pages land next to the target as `stem-1.png`, `stem-2.png`…
            let stem = request
                .output
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "page".into());
            let prefix = request.output.with_file_name(stem);
            cmd.arg(&input).arg(prefix);
            (cmd, OutputMode::Direct)
        }
        "tesseract" => {
            // tesseract takes an output *base* and appends `.txt` itself.
            let base = request.output.with_extension("");
            let mut cmd = tools.command("tesseract");
            cmd.arg(&input)
                .arg(&base)
                .args(["-l", "eng"])
                .arg(if to == "pdf" { "pdf" } else { "txt" });
            (cmd, OutputMode::Direct)
        }
        "pdftops" => {
            let mut cmd = tools.command("pdftops");
            if to == "eps" {
                cmd.arg("-eps");
            }
            cmd.arg(&input).arg(&target);
            (cmd, OutputMode::Direct)
        }
        other => return Err(format!("No command builder for converter '{other}'")),
    };

    Ok(built)
}

fn libreoffice_filters(from: &str, to: &str) -> (Option<String>, Option<String>) {
    if to == "pdf" {
        return (None, None);
    }

    const TEXT: [(&str, &str); 37] = [
        ("602", "T602Document"),
        ("abw", "AbiWord"),
        ("doc", "MS Word 97"),
        ("docm", "MS Word 2007 XML VBA"),
        ("docx", "MS Word 2007 XML"),
        ("dot", "MS Word 97 Vorlage"),
        ("dotx", "MS Word 2007 XML Template"),
        ("dotm", "MS Word 2007 XML Template"),
        ("epub", "EPUB"),
        ("fb2", "Fictionbook 2"),
        ("fodt", "OpenDocument Text Flat XML"),
        ("htm", "HTML (StarWriter)"),
        ("html", "HTML (StarWriter)"),
        ("hwp", "writer_MIZI_Hwp_97"),
        ("mcw", "MacWrite"),
        ("mw", "MacWrite"),
        ("mwd", "Mariner_Write"),
        ("lwp", "LotusWordPro"),
        ("lrf", "BroadBand eBook"),
        ("odt", "writer8"),
        ("ott", "writer8_template"),
        ("pages", "Apple Pages"),
        ("pdf", "writer_pdf_import"),
        ("psw", "PocketWord File"),
        ("rtf", "Rich Text Format"),
        ("sdw", "StarOffice_Writer"),
        ("stw", "writer_StarOffice_XML_Writer_Template"),
        ("sxw", "StarOffice XML (Writer)"),
        ("tab", "Text"),
        ("txt", "Text"),
        ("wn", "WriteNow"),
        ("wpd", "WordPerfect"),
        ("wpt", "MS Word 97 Vorlage"),
        ("wri", "MS_Write"),
        ("xhtml", "HTML (StarWriter)"),
        ("xml", "OpenDocument Text Flat XML"),
        ("zabw", "AbiWord"),
    ];

    const CALC: [(&str, &str); 10] = [
        ("csv", "Text - txt - csv (StarCalc)"),
        ("ods", "calc8"),
        ("ots", "calc8_template"),
        ("pdf", "calc_pdf_Export"),
        ("tsv", "Text - txt - csv (StarCalc)"),
        ("xls", "MS Excel 97"),
        ("xlsx", "Calc MS Excel 2007 XML"),
        ("xlsm", "Calc MS Excel 2007 XML VBA"),
        ("xlt", "MS Excel 97 Vorlage"),
        ("xltm", "Calc MS Excel 2007 XML Template"),
    ];

    let lookup = |table: &[(&str, &str)], key: &str| {
        table
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, filter)| filter.to_string())
    };

    if let (Some(in_filter), Some(out_filter)) = (lookup(&TEXT, from), lookup(&TEXT, to)) {
        return (Some(in_filter), Some(out_filter));
    }
    if let (Some(in_filter), Some(out_filter)) = (lookup(&CALC, from), lookup(&CALC, to)) {
        return (Some(in_filter), Some(out_filter));
    }
    (None, None)
}

/// Builds a non-clobbering output path inside `dir`.
pub fn unique_output_path(dir: &Path, stem: &str, extension: &str, taken: &[PathBuf]) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.{extension}"));
    let mut counter = 1;
    while candidate.exists() || taken.contains(&candidate) {
        candidate = dir.join(format!("{stem} ({counter}).{extension}"));
        counter += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;
    use crate::tools::Toolchain;

    fn write_png(path: &std::path::Path) {
        let image = image::RgbaImage::from_fn(40, 24, |x, y| {
            image::Rgba([(x * 6) as u8, (y * 10) as u8, 128, 255])
        });
        image.save(path).expect("write png");
    }

    #[test]
    fn native_engine_handles_routes_through_the_normal_path() {
        let dir = std::env::temp_dir().join(format!("convertx-chain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("sample.png");
        write_png(&input);

        let registry = Registry::get();
        let candidates = registry.converters_for("png", "qoi");
        assert!(
            candidates.iter().any(|spec| spec.id == "native-images"),
            "the built-in engine must be a candidate for png -> qoi"
        );

        let tools = Toolchain::detect(registry.converters());
        let request = ConvertRequest {
            job_id: 1,
            input,
            from: "png".into(),
            to: "qoi".into(),
            converter_id: "native-images".into(),
            output: dir.join("sample.qoi"),
            options: ConvertOptions::default(),
        };

        let result = run(&request, &tools);
        assert!(result.ok, "{}", result.message);
        assert!(validate_output(&request).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chain_reports_every_attempt_when_all_converters_fail() {
        let dir = std::env::temp_dir().join(format!("convertx-allfail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("broken.png");
        std::fs::write(&input, b"not really a png").unwrap();

        let tools = Toolchain::detect(Registry::get().converters());
        let request = ConvertRequest {
            job_id: 1,
            input,
            from: "png".into(),
            to: "jpeg".into(),
            converter_id: String::new(),
            output: dir.join("broken.jpg"),
            options: ConvertOptions::default(),
        };

        let result = run_chain(&request, &tools);
        assert!(!result.ok);
        // Either several converters were tried, or none is installed at all.
        assert!(
            !result.attempts.is_empty() || result.message.contains("not installed"),
            "{}",
            result.message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chain_still_converts_ordinary_images() {
        let dir = std::env::temp_dir().join(format!("convertx-chain2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("sample.png");
        write_png(&input);

        let tools = Toolchain::detect(Registry::get().converters());
        let request = ConvertRequest {
            job_id: 1,
            input,
            from: "png".into(),
            to: "jpeg".into(),
            converter_id: String::new(),
            output: dir.join("sample.jpg"),
            options: ConvertOptions::default(),
        };

        let result = run_chain(&request, &tools);
        assert!(result.ok, "{}", result.message);
        assert!(result.converter.is_some());
        assert!(validate_output(&request).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command that keeps running so the watchdog has something to kill.
    fn hanging_command() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping", "-n", "30", "127.0.0.1"]);
            command
        }
    }

    fn echoing_command() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("echo");
            command.arg("done");
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args(["/C", "echo done"]);
            command
        }
    }

    #[test]
    fn watchdog_kills_a_hung_converter() {
        let command = hanging_command();
        let started = Instant::now();
        let result = run_child(command, 1).expect("spawn");
        assert!(result.timed_out, "the watchdog should have fired");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn watchdog_lets_fast_converters_finish() {
        let command = echoing_command();
        let result = run_child(command, 30).expect("spawn");
        assert!(!result.timed_out);
        assert!(result.status.success());
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "done");
    }

    #[test]
    fn validation_rejects_empty_and_corrupt_outputs() {
        let dir = std::env::temp_dir().join(format!("convertx-valid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let request = ConvertRequest {
            job_id: 1,
            input: dir.join("in.png"),
            from: "png".into(),
            to: "png".into(),
            converter_id: "native-images".into(),
            output: dir.join("empty.png"),
            options: ConvertOptions::default(),
        };
        assert!(validate_output(&request).is_err(), "empty file must fail");

        std::fs::write(&request.output, b"definitely not a png").unwrap();
        assert!(
            validate_output(&request).is_err(),
            "corrupt image must fail"
        );

        write_png(&request.output);
        assert!(validate_output(&request).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("convertx-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn output_names_do_not_collide() {
        let dir = temp_path("collisions");
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_output_path(&dir, "report", "pdf", &[]);
        assert_eq!(first.file_name().unwrap(), "report.pdf");
        let second = unique_output_path(&dir, "report", "pdf", &[first]);
        assert_eq!(second.file_name().unwrap(), "report (1).pdf");
    }

    #[test]
    fn builtin_vcf_conversion_writes_csv() {
        let input = temp_path("contact.vcf");
        let output = temp_path("contact.csv");
        std::fs::write(
            &input,
            "BEGIN:VCARD\nFN:Grace Hopper\nEMAIL:grace@example.com\nEND:VCARD\n",
        )
        .unwrap();

        let request = ConvertRequest {
            job_id: 1,
            input,
            from: "vcf".into(),
            to: "csv".into(),
            converter_id: "vcf".into(),
            output: output.clone(),
            options: ConvertOptions::default(),
        };
        let result = run(&request, &Toolchain::default());
        assert!(result.ok, "{}", result.message);
        let csv = std::fs::read_to_string(&output).unwrap();
        assert!(csv.contains("Grace Hopper"));
    }

    #[test]
    fn converts_png_to_jpeg_with_imagemagick_when_available() {
        let registry = Registry::get();
        let tools = Toolchain::detect(registry.converters());
        if !tools.is_installed("imagemagick") {
            eprintln!("ImageMagick not installed, skipping");
            return;
        }

        let input = temp_path("source.png");
        let output = temp_path("result.jpg");
        let _ = std::fs::remove_file(&output);
        let created = tools
            .command("magick")
            .args(["-size", "48x48", "xc:#3366ff"])
            .arg(&input)
            .status()
            .expect("magick should run");
        assert!(created.success());

        let request = ConvertRequest {
            job_id: 2,
            input,
            from: "png".into(),
            to: "jpeg".into(),
            converter_id: "imagemagick".into(),
            output: output.clone(),
            options: ConvertOptions::default(),
        };
        let result = run(&request, &tools);
        assert!(result.ok, "{}", result.message);
        assert!(output.exists());
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
    }
}
