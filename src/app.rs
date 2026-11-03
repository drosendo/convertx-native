use crate::convert::{self, ConvertRequest, ConvertResult};
use crate::history::{self, HistoryEntry};
use crate::optimize::{self, OptimizeOptions, OptimizeOutcome};
use crate::registry::{ConverterSpec, Registry, human_size, normalize_input, normalize_output};
use crate::settings::Settings;
use crate::theme;
use crate::tools::{self, PackageManager, Toolchain};
use crate::update::{self, ReleaseInfo};
use eframe::egui;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, RichText, Sense, Stroke, StrokeKind,
};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Convert,
    Optimize,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Convert => "Convert",
            Mode::Optimize => "Optimize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobStatus {
    Ready,
    Running,
    Done,
    Failed,
}

impl JobStatus {
    fn label(self) -> &'static str {
        match self {
            JobStatus::Ready => "Ready",
            JobStatus::Running => "Converting",
            JobStatus::Done => "Done",
            JobStatus::Failed => "Failed",
        }
    }

    fn color(self) -> Color32 {
        match self {
            JobStatus::Ready => theme::MUTED,
            JobStatus::Running => theme::ACCENT,
            JobStatus::Done => theme::SUCCESS,
            JobStatus::Failed => theme::ERROR,
        }
    }
}

struct Job {
    id: u64,
    path: PathBuf,
    name: String,
    size: u64,
    raw_ext: Option<String>,
    from: Option<String>,
    target: Option<String>,
    status: JobStatus,
    message: String,
    converter: Option<String>,
    output: Option<PathBuf>,
    duration: Option<Duration>,
    log: String,
    new_size: Option<u64>,
}

enum Event {
    Tools(Box<Toolchain>),
    JobStarted(u64),
    JobOptimized {
        id: u64,
        outcome: OptimizeOutcome,
    },
    UpdateChecked(Box<Result<Option<ReleaseInfo>, String>>),
    JobFinished {
        id: u64,
        converter: String,
        from: String,
        to: String,
        result: ConvertResult,
    },
    InstallLine(String),
    InstallDone {
        converter: String,
        ok: bool,
        message: String,
    },
}

struct InstallPrompt {
    converter: String,
    converter_name: String,
    command: Vec<String>,
    manager: PackageManager,
    note: Option<String>,
    manual: Option<String>,
    apply_target: Option<String>,
}

struct Toast {
    text: String,
    color: Color32,
    at: Instant,
}

struct ConverterRow {
    id: String,
    name: String,
    description: String,
    installed: bool,
    version: Option<String>,
    missing: Vec<String>,
    builtin: bool,
    binaries: Vec<String>,
}

struct Badge {
    label: String,
    color: Color32,
}

pub struct ConvertXApp {
    registry: &'static Registry,
    settings: Settings,
    tools: Toolchain,
    detecting: bool,
    jobs: Vec<Job>,
    next_id: u64,
    global_target: String,
    target_filter: String,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    log: Vec<String>,
    running: bool,
    batch_total: usize,
    batch_done: usize,
    pending_target: Option<String>,
    install_prompt: Option<InstallPrompt>,
    installing: Option<String>,
    install_log: Vec<String>,
    show_settings: bool,
    show_history: bool,
    history: Vec<HistoryEntry>,
    toast: Option<Toast>,
    last_output_dir: Option<PathBuf>,
    drag_hover: bool,
    mode: Mode,
    checking_updates: bool,
    update: Option<ReleaseInfo>,
    update_status: String,
    optimize: OptimizeOptions,
    optimize_in_place: bool,
    optimize_backup: bool,
}

impl ConvertXApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let settings = Settings::load();
        let mut app = Self::build(settings);
        let initial: Vec<PathBuf> = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .collect();
        if !initial.is_empty() {
            app.add_paths(initial);
        }
        app.detect_tools(&cc.egui_ctx);
        if app.settings.check_updates {
            app.check_updates(&cc.egui_ctx);
        }
        app
    }

    fn check_updates(&mut self, ctx: &egui::Context) {
        if self.checking_updates {
            return;
        }
        self.checking_updates = true;
        self.update_status = "Checking for updates…".into();
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = update::check();
            let _ = tx.send(Event::UpdateChecked(Box::new(result)));
            ctx.request_repaint();
        });
    }

    fn build(settings: Settings) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let registry = Registry::get();
        tools::set_overrides(&settings.custom_paths);
        let global_target = settings.last_target.clone();

        Self {
            registry,
            settings,
            tools: Toolchain::default(),
            detecting: true,
            jobs: Vec::new(),
            next_id: 1,
            global_target,
            target_filter: String::new(),
            events_tx,
            events_rx,
            log: Vec::new(),
            running: false,
            batch_total: 0,
            batch_done: 0,
            pending_target: None,
            install_prompt: None,
            installing: None,
            install_log: Vec::new(),
            show_settings: false,
            show_history: false,
            history: history::load(),
            toast: None,
            last_output_dir: None,
            drag_hover: false,
            mode: Mode::Convert,
            checking_updates: false,
            update: None,
            update_status: String::new(),
            optimize: OptimizeOptions::default(),
            optimize_in_place: false,
            optimize_backup: false,
        }
    }

    fn detect_tools(&mut self, ctx: &egui::Context) {
        self.detecting = true;
        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let chain = Toolchain::detect(Registry::get().converters());
            let _ = tx.send(Event::Tools(Box::new(chain)));
            ctx.request_repaint();
        });
    }

    fn toast(&mut self, text: impl Into<String>, color: Color32) {
        self.toast = Some(Toast {
            text: text.into(),
            color,
            at: Instant::now(),
        });
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 400 {
            self.log.remove(0);
        }
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let mut added = 0usize;
        for path in paths {
            if path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    let files: Vec<PathBuf> = entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_file())
                        .take(500)
                        .collect();
                    added += self.add_files(files);
                }
                continue;
            }
            added += self.add_files(vec![path]);
        }

        if added > 0 {
            self.toast(format!("Added {added} file(s)"), theme::SUCCESS);
        }
    }

    fn add_files(&mut self, paths: Vec<PathBuf>) -> usize {
        let mut count = 0;
        for path in paths {
            if self.jobs.iter().any(|job| job.path == path) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let raw_ext = self.registry.detect_extension(&path);
            let from = raw_ext.as_ref().map(|ext| normalize_input(ext));
            let from = from.filter(|ext| self.registry.is_known_input(ext));

            let mut job = Job {
                id: self.next_id,
                path,
                name,
                size: meta.len(),
                raw_ext,
                from: from.clone(),
                target: None,
                status: JobStatus::Ready,
                message: String::new(),
                converter: None,
                output: None,
                duration: None,
                log: String::new(),
                new_size: None,
            };

            if from.is_none() {
                job.status = JobStatus::Failed;
                job.message = match &job.raw_ext {
                    Some(ext) => format!("No converter reads .{ext}"),
                    None => "Unknown file type".into(),
                };
            } else if !self.global_target.is_empty() {
                let supports = self
                    .registry
                    .converters_for(from.as_deref().unwrap_or(""), &self.global_target);
                if !supports.is_empty() {
                    job.target = Some(self.global_target.clone());
                }
            }

            self.next_id += 1;
            self.jobs.push(job);
            count += 1;
        }
        count
    }

    fn remove_job(&mut self, id: u64) {
        self.jobs
            .retain(|job| job.id != id || job.status == JobStatus::Running);
    }

    fn clear_finished(&mut self) {
        self.jobs
            .retain(|job| matches!(job.status, JobStatus::Running | JobStatus::Ready));
    }

    fn is_installed(&self, converter_id: &str) -> bool {
        self.tools.is_installed(converter_id)
    }

    /// Output formats available for the current selection, with the number of
    /// files in the queue that support each of them.
    fn target_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for job in &self.jobs {
            let Some(from) = job.from.as_deref() else {
                continue;
            };
            for target in self.registry.targets_for(from) {
                match counts.iter_mut().find(|(t, _)| t == target) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((target.to_string(), 1)),
                }
            }
        }
        counts
    }

    fn category_order(&self) -> Vec<&'static str> {
        vec![
            "images", "image", "object", "document", "text", "calc", "ocr", "email", "contacts",
            "jxl", "muxer",
        ]
    }

    fn apply_target(&mut self, target: &str) {
        self.global_target = target.to_string();
        for job in &mut self.jobs {
            let Some(from) = job.from.as_deref() else {
                continue;
            };
            if self.registry.converters_for(from, target).is_empty() {
                continue;
            }
            job.target = Some(target.to_string());
            if job.status == JobStatus::Failed {
                job.status = JobStatus::Ready;
                job.message.clear();
            }
        }
        self.settings.last_target = target.to_string();
        self.settings.save();
    }

    fn start_batch(&mut self, ctx: &egui::Context) {
        if self.running {
            return;
        }

        let out_dir = self.settings.output_path();
        if let Err(err) = std::fs::create_dir_all(&out_dir) {
            self.toast(format!("Cannot create output folder: {err}"), theme::ERROR);
            return;
        }

        let options = self.settings.convert_options();
        let mut requests: Vec<ConvertRequest> = Vec::new();
        let mut missing: Vec<(String, String)> = Vec::new();
        let mut taken: Vec<PathBuf> = Vec::new();
        let mut skipped_unknown = 0usize;

        let registry = self.registry;
        let tools = &self.tools;
        let mut queued_log: Vec<String> = Vec::new();

        for job in &mut self.jobs {
            if job.status == JobStatus::Running {
                continue;
            }
            let (Some(from), Some(to)) = (job.from.clone(), job.target.clone()) else {
                if job.from.is_some() {
                    skipped_unknown += 1;
                }
                continue;
            };

            let Some(spec) = registry.best_converter(&from, &to, |id| tools.is_installed(id))
            else {
                job.status = JobStatus::Failed;
                job.message = format!("No converter supports {from} → {to}");
                continue;
            };

            if !tools.is_installed(&spec.id) {
                job.status = JobStatus::Ready;
                job.message = format!("Needs {}", spec.name);
                missing.push((spec.id.clone(), to.clone()));
                continue;
            }

            let extension = normalize_output(&to);
            let stem = job
                .name
                .trim_end_matches(&format!(".{}", job.raw_ext.clone().unwrap_or_default()))
                .to_string();
            let stem = if stem.is_empty() {
                job.name.clone()
            } else {
                stem
            };
            let output = convert::unique_output_path(&out_dir, &stem, &extension, &taken);
            taken.push(output.clone());

            job.status = JobStatus::Running;
            job.converter = Some(spec.id.clone());
            job.message = "Waiting for worker…".into();
            job.output = None;
            job.duration = None;

            queued_log.push(format!("{}: {} → {} via {}", job.name, from, to, spec.name));
            requests.push(ConvertRequest {
                job_id: job.id,
                input: job.path.clone(),
                from,
                to,
                converter_id: spec.id.clone(),
                output,
                options: options.clone(),
            });
        }

        for line in queued_log {
            self.push_log(line);
        }

        if requests.is_empty() {
            if let Some((converter, target)) = missing.first() {
                self.offer_install(converter, Some(target.clone()));
            } else if skipped_unknown > 0 {
                self.toast("Choose an output format first", theme::WARN);
            }
            return;
        }

        self.running = true;
        self.batch_total = requests.len();
        self.batch_done = 0;
        self.last_output_dir = Some(out_dir.clone());
        self.push_log(format!(
            "Starting {} conversion(s) with {} worker(s)",
            requests.len(),
            self.settings.parallel_jobs.clamp(1, 8)
        ));

        let queue = Arc::new(requests);
        let cursor = Arc::new(AtomicUsize::new(0));
        let chain = Arc::new(self.tools.clone());
        let workers = self
            .settings
            .parallel_jobs
            .clamp(1, 8)
            .min(queue.len())
            .max(1);

        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let cursor = Arc::clone(&cursor);
            let chain = Arc::clone(&chain);
            let tx = self.events_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let index = cursor.fetch_add(1, Ordering::SeqCst);
                    let Some(request) = queue.get(index).cloned() else {
                        break;
                    };
                    let _ = tx.send(Event::JobStarted(request.job_id));
                    ctx.request_repaint();

                    let result = convert::run_chain(&request, &chain);
                    let converter = result
                        .converter
                        .clone()
                        .unwrap_or_else(|| request.converter_id.clone());
                    let _ = tx.send(Event::JobFinished {
                        id: request.job_id,
                        converter,
                        from: request.from.clone(),
                        to: request.to.clone(),
                        result,
                    });
                    ctx.request_repaint();
                }
            });
        }

        if let Some((converter, target)) = missing.first() {
            self.toast(
                format!("Some files need an extra tool: {}", self.name_of(converter)),
                theme::WARN,
            );
            self.offer_install(converter, Some(target.clone()));
        }
    }

    fn finish_batch(&mut self) {
        self.running = false;
        let failures = self
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Failed)
            .count();
        if failures == 0 {
            self.toast("Finished — everything worked", theme::SUCCESS);
        } else {
            self.toast(
                format!("Finished with {failures} failed file(s)"),
                theme::WARN,
            );
        }
        if self.settings.open_output_after
            && let Some(dir) = self.last_output_dir.clone()
        {
            crate::open_in_file_manager(&dir);
        }
        self.batch_total = 0;
        self.batch_done = 0;
    }

    /// Images that the optimizer can handle and that are not already running.
    fn optimizable_jobs(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.status != JobStatus::Running)
            .filter(|job| job.raw_ext.as_deref().is_some_and(optimize::is_optimizable))
            .count()
    }

    fn start_optimize(&mut self, ctx: &egui::Context) {
        if self.running {
            return;
        }

        let out_dir = self.settings.output_path();
        if !self.optimize_in_place
            && let Err(err) = std::fs::create_dir_all(&out_dir)
        {
            self.toast(format!("Cannot create output folder: {err}"), theme::ERROR);
            return;
        }

        let options = self.optimize.clone();
        let in_place = self.optimize_in_place;
        let backup = self.optimize_backup;
        let mut tasks: Vec<(u64, PathBuf, PathBuf)> = Vec::new();
        let mut taken: Vec<PathBuf> = Vec::new();
        let mut skipped = 0usize;

        for job in &mut self.jobs {
            if job.status == JobStatus::Running {
                continue;
            }
            let Some(extension) = job.raw_ext.clone() else {
                skipped += 1;
                continue;
            };
            if !optimize::is_optimizable(&extension) {
                skipped += 1;
                continue;
            }

            let output = if in_place {
                job.path.clone()
            } else {
                let stem = job
                    .name
                    .trim_end_matches(&format!(".{extension}"))
                    .to_string();
                let stem = if stem.is_empty() {
                    job.name.clone()
                } else {
                    format!("{stem}-min")
                };
                let path = convert::unique_output_path(&out_dir, &stem, &extension, &taken);
                taken.push(path.clone());
                path
            };

            job.status = JobStatus::Running;
            job.message = "Optimizing…".into();
            job.output = None;
            job.new_size = None;
            tasks.push((job.id, job.path.clone(), output));
        }

        if tasks.is_empty() {
            self.toast(
                if skipped == 0 {
                    "Add images to optimize".to_string()
                } else {
                    "Only PNG, JPEG, GIF, WebP, SVG, TIFF, AVIF and HEIC can be optimized"
                        .to_string()
                },
                theme::WARN,
            );
            return;
        }

        self.running = true;
        self.batch_total = tasks.len();
        self.batch_done = 0;
        self.last_output_dir = Some(out_dir.clone());
        self.push_log(format!(
            "Optimizing {} image(s){}",
            tasks.len(),
            if options.lossless {
                " losslessly".to_string()
            } else {
                format!(" at quality {}", options.quality)
            }
        ));

        let queue = Arc::new(tasks);
        let cursor = Arc::new(AtomicUsize::new(0));
        let workers = self
            .settings
            .parallel_jobs
            .clamp(1, 8)
            .min(queue.len())
            .max(1);

        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let cursor = Arc::clone(&cursor);
            let options = options.clone();
            let tx = self.events_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let index = cursor.fetch_add(1, Ordering::SeqCst);
                    let Some((id, input, output)) = queue.get(index).cloned() else {
                        break;
                    };
                    let _ = tx.send(Event::JobStarted(id));
                    ctx.request_repaint();

                    let mut outcome = if in_place {
                        optimize::optimize_in_place(&input, &options, backup)
                    } else {
                        optimize::optimize(&input, &output, &options)
                    };
                    if outcome.ok && !in_place {
                        outcome.message = "Optimized".into();
                    }

                    let _ = tx.send(Event::JobOptimized { id, outcome });
                    ctx.request_repaint();
                }
            });
        }
    }

    fn name_of(&self, converter_id: &str) -> String {
        self.registry
            .converter(converter_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| converter_id.to_string())
    }

    fn offer_install(&mut self, converter_id: &str, apply_target: Option<String>) {
        let Some(spec) = self.registry.converter(converter_id) else {
            return;
        };
        let installers = tools::available_installers(spec);
        let Some((manager, argv)) = installers.into_iter().next() else {
            self.toast(
                format!(
                    "No package manager found for {}. {}",
                    spec.name,
                    spec.manual.clone().unwrap_or_default()
                ),
                theme::WARN,
            );
            return;
        };
        let (program, args, note) = tools::elevated(&argv, manager);
        let mut command = vec![program];
        command.extend(args);

        self.install_prompt = Some(InstallPrompt {
            converter: spec.id.clone(),
            converter_name: spec.name.clone(),
            command,
            manager,
            note,
            manual: spec.manual.clone(),
            apply_target,
        });
    }

    fn start_install(&mut self, ctx: &egui::Context, prompt: InstallPrompt) {
        self.installing = Some(prompt.converter.clone());
        self.install_log.clear();
        self.pending_target = prompt.apply_target.clone();
        self.push_log(format!(
            "Installing {} via {}: {}",
            prompt.converter_name,
            prompt.manager.label(),
            prompt.command.join(" ")
        ));

        let tx = self.events_tx.clone();
        let ctx = ctx.clone();
        let path_env = self.tools.path.clone();
        let converter = prompt.converter.clone();
        let command = prompt.command.clone();

        std::thread::spawn(move || {
            let program = command.first().cloned().unwrap_or_default();
            let args: Vec<String> = command.iter().skip(1).cloned().collect();
            let mut process = match Command::new(&program)
                .args(&args)
                .env("PATH", &path_env)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => child,
                Err(err) => {
                    let _ = tx.send(Event::InstallDone {
                        converter,
                        ok: false,
                        message: format!("Could not start installer: {err}"),
                    });
                    ctx.request_repaint();
                    return;
                }
            };

            let stderr = process.stderr.take();
            let stderr_tx = tx.clone();
            let stderr_ctx = ctx.clone();
            let stderr_thread = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    for line in BufReader::new(err).lines().map_while(Result::ok) {
                        let _ = stderr_tx.send(Event::InstallLine(line));
                        stderr_ctx.request_repaint();
                    }
                }
            });

            if let Some(out) = process.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(Event::InstallLine(line));
                    ctx.request_repaint();
                }
            }

            let status = process.wait();
            let _ = stderr_thread.join();
            let ok = status.map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(Event::InstallDone {
                converter,
                ok,
                message: if ok {
                    "Installed".into()
                } else {
                    "Installer exited with an error".into()
                },
            });
            ctx.request_repaint();
        });
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                Event::Tools(chain) => {
                    let ready = chain.status.values().filter(|s| s.available).count();
                    self.tools = *chain;
                    self.detecting = false;
                    self.push_log(format!(
                        "{ready} of {} converters ready",
                        self.registry.converters().len()
                    ));
                    if let Some(target) = self.pending_target.take() {
                        self.apply_target(&target);
                    }
                }
                Event::JobStarted(id) => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.status = JobStatus::Running;
                        job.message = match self.mode {
                            Mode::Convert => "Converting…",
                            Mode::Optimize => "Optimizing…",
                        }
                        .into();
                    }
                }
                Event::JobFinished {
                    id,
                    converter,
                    from,
                    to,
                    result,
                } => {
                    let mut output_for_history = None;
                    let mut log_line = None;
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.status = if result.ok {
                            JobStatus::Done
                        } else {
                            JobStatus::Failed
                        };
                        job.message = result.message.clone();
                        job.output = result.output.clone();
                        job.duration = Some(result.duration);
                        job.log = result.log.clone();
                        output_for_history =
                            Some((job.name.clone(), job.path.clone(), job.output.clone()));
                        log_line = Some(format!(
                            "{}: {} → {} ({})",
                            job.name, from, to, result.message
                        ));
                    }

                    if let Some(line) = log_line {
                        self.push_log(line);
                    }

                    if self.settings.record_history
                        && let Some((name, input, output)) = output_for_history
                    {
                        history::push(
                            &mut self.history,
                            HistoryEntry {
                                timestamp: chrono::Local::now().timestamp(),
                                input: input.to_string_lossy().into_owned(),
                                output: output
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| name.clone()),
                                converter,
                                from,
                                to,
                                ok: result.ok,
                                message: result.message.clone(),
                                duration_ms: result.duration.as_millis() as u64,
                            },
                        );
                    }

                    self.batch_done += 1;
                    if self.batch_done >= self.batch_total {
                        self.finish_batch();
                    }
                }
                Event::JobOptimized { id, outcome } => {
                    let mut log_line = None;
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        job.status = if outcome.ok {
                            JobStatus::Done
                        } else {
                            JobStatus::Failed
                        };
                        job.message = if outcome.ok {
                            outcome.summary()
                        } else {
                            outcome.message.clone()
                        };
                        job.new_size = Some(outcome.after);
                        log_line = Some(format!("{}: {}", job.name, job.message));
                    }

                    if let Some(line) = log_line {
                        self.push_log(line);
                    }

                    self.batch_done += 1;
                    if self.batch_done >= self.batch_total {
                        self.finish_batch();
                    }
                }
                Event::UpdateChecked(result) => {
                    self.checking_updates = false;
                    match *result {
                        Ok(Some(release)) => {
                            self.update_status =
                                format!("Version {} is available", release.version);
                            self.push_log(format!(
                                "Update available: {} (running {})",
                                release.version,
                                update::CURRENT_VERSION
                            ));
                            self.update = Some(release);
                        }
                        Ok(None) => {
                            self.update_status = format!(
                                "You are on the latest version ({})",
                                update::CURRENT_VERSION
                            );
                            self.update = None;
                        }
                        Err(err) => {
                            self.update_status = err;
                            self.update = None;
                        }
                    }
                }
                Event::InstallLine(line) => {
                    self.install_log.push(line);
                    if self.install_log.len() > 500 {
                        self.install_log.remove(0);
                    }
                }
                Event::InstallDone {
                    converter,
                    ok,
                    message,
                } => {
                    self.installing = None;
                    let name = self.name_of(&converter);
                    if ok {
                        self.toast(format!("{name} installed"), theme::SUCCESS);
                        self.detect_tools(ctx);
                    } else {
                        self.toast(format!("Could not install {name}: {message}"), theme::ERROR);
                        self.pending_target = None;
                    }
                }
            }
        }
    }

    fn handle_dropped(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) = ctx.input(|i| {
            (
                i.raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>(),
                !i.raw.hovered_files.is_empty(),
            )
        });
        self.drag_hover = hovering;
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }
    }

    // ---------------------------------------------------------------- UI bits

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(9),
                theme::ACCENT,
                Stroke::NONE,
                StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "CX",
                FontId::proportional(15.0),
                Color32::WHITE,
            );

            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("ConvertX").size(18.5).strong());
                ui.label(
                    RichText::new("Native file conversion")
                        .size(11.5)
                        .color(theme::MUTED),
                );
            });

            ui.add_space(16.0);
            for mode in [Mode::Convert, Mode::Optimize] {
                let selected = self.mode == mode;
                let button =
                    egui::Button::new(RichText::new(mode.label()).size(12.5).color(if selected {
                        Color32::WHITE
                    } else {
                        theme::MUTED
                    }))
                    .fill(if selected { theme::ACCENT } else { theme::CARD })
                    .stroke(Stroke::new(1.0, theme::BORDER));
                if ui
                    .add(button)
                    .on_hover_text(match mode {
                        Mode::Convert => "Change a file from one format to another",
                        Mode::Optimize => "Shrink images and strip metadata without losing quality",
                    })
                    .clicked()
                {
                    self.mode = mode;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.show_settings = true;
                }
                if ui.button("History").clicked() {
                    self.show_history = true;
                }

                let ready = self.tools.status.values().filter(|s| s.available).count();
                let total = self.registry.converters().len();
                let color = if ready == 0 {
                    theme::ERROR
                } else if ready < total / 2 {
                    theme::WARN
                } else {
                    theme::SUCCESS
                };
                let label = if self.detecting {
                    "Scanning tools…".to_string()
                } else {
                    format!("{ready}/{total} converters ready")
                };
                let response = ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("● {label}")).size(12.0).color(color),
                        )
                        .fill(theme::CARD)
                        .stroke(Stroke::new(1.0, theme::BORDER)),
                    )
                    .on_hover_text("Open the converter manager");
                if response.clicked() {
                    self.show_settings = true;
                }
            });
        });
    }

    fn ui_empty_state(&mut self, ui: &mut egui::Ui) {
        let border = if self.drag_hover {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        let fill = if self.drag_hover {
            theme::ACCENT_SOFT
        } else {
            theme::CARD
        };

        egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.5, border))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(egui::Margin::same(28))
            .show(ui, |ui| {
                ui.set_min_height(300.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(48.0);
                    ui.label(
                        RichText::new("Drop files here")
                            .size(22.0)
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} input · {} output formats · {} converters",
                            self.registry.input_formats().count(),
                            self.registry.all_targets().len(),
                            self.registry.converters().len()
                        ))
                        .size(12.5)
                        .color(theme::MUTED),
                    );
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        let width = 150.0;
                        let total = width * 2.0 + 8.0;
                        ui.add_space((ui.available_width() - total).max(0.0) / 2.0);
                        if ui
                            .add_sized(
                                [width, 34.0],
                                egui::Button::new(RichText::new("Browse files").strong())
                                    .fill(theme::ACCENT)
                                    .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            self.pick_files();
                        }
                        if ui
                            .add_sized([width, 34.0], egui::Button::new("Add folder"))
                            .clicked()
                        {
                            self.pick_folder();
                        }
                    });
                });
            });
    }

    fn pick_files(&mut self) {
        if let Some(files) = crate::dialogs::pick_files() {
            self.add_paths(files);
        }
    }

    fn pick_folder(&mut self) {
        if let Some(dir) = crate::dialogs::pick_folder() {
            self.add_paths(vec![dir]);
        }
    }

    fn ui_files(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Files").size(15.0).strong());
            ui.label(
                RichText::new(format!("{}", self.jobs.len()))
                    .size(11.0)
                    .color(theme::MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.jobs.is_empty() && ui.small_button("Clear").clicked() {
                    self.clear_finished();
                }
                if ui.small_button("+ Add files").clicked() {
                    self.pick_files();
                }
            });
        });
        ui.add_space(4.0);

        if self.jobs.is_empty() {
            ScrollAreaHelper::show(ui, |ui| self.ui_empty_state(ui));
            return;
        }

        let mut remove: Option<u64> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let ids: Vec<u64> = self.jobs.iter().map(|j| j.id).collect();
                for id in ids {
                    let Some(index) = self.jobs.iter().position(|j| j.id == id) else {
                        continue;
                    };
                    if self.ui_job_row(ui, index) {
                        remove = Some(id);
                    }
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
            });

        if let Some(id) = remove {
            self.remove_job(id);
        }
    }

    fn ui_job_row(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        struct JobView {
            id: u64,
            name: String,
            size: u64,
            raw_ext: Option<String>,
            from: Option<String>,
            target: Option<String>,
            status: JobStatus,
            message: String,
            output: Option<PathBuf>,
            new_size: Option<u64>,
        }

        let view = {
            let job = &self.jobs[index];
            JobView {
                id: job.id,
                name: job.name.clone(),
                size: job.size,
                raw_ext: job.raw_ext.clone(),
                from: job.from.clone(),
                target: job.target.clone(),
                status: job.status,
                message: job.message.clone(),
                output: job.output.clone(),
                new_size: job.new_size,
            }
        };

        let category = view
            .from
            .as_deref()
            .and_then(|ext| self.registry.targets_for(ext).first().copied())
            .map(|target| self.registry.category_of(target))
            .unwrap_or("other");
        let badge = category_badge(category);

        let mut remove = false;
        let mut chosen_target: Option<String> = None;
        let mut reveal: Option<PathBuf> = None;
        let registry = self.registry;
        let tools = &self.tools;

        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(8),
                    badge.color.gamma_multiply(0.18),
                    Stroke::new(1.0, badge.color.gamma_multiply(0.5)),
                    StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    badge.label,
                    FontId::proportional(10.5),
                    badge.color,
                );

                let info_width = (ui.available_width() - 210.0).max(120.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(info_width, 44.0),
                    Layout::top_down(Align::LEFT),
                    |ui| {
                        ui.add_space(2.0);
                        ui.label(RichText::new(&view.name).size(13.5).strong());
                        ui.horizontal(|ui| {
                            let from = view
                                .raw_ext
                                .as_deref()
                                .map(|e| e.to_uppercase())
                                .unwrap_or_else(|| "?".into());
                            ui.label(
                                RichText::new(from)
                                    .size(10.5)
                                    .color(theme::MUTED)
                                    .monospace(),
                            );
                            ui.label(RichText::new("→").size(10.5).color(theme::MUTED));
                            let to = match (self.mode, view.target.as_deref()) {
                                (Mode::Optimize, _) => "OPTIMIZED".to_string(),
                                (_, Some(target)) => normalize_output(target).to_uppercase(),
                                (_, None) => "—".into(),
                            };
                            ui.label(
                                RichText::new(to)
                                    .size(10.5)
                                    .color(theme::ACCENT)
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(human_size(view.size))
                                    .size(10.5)
                                    .color(theme::MUTED),
                            );
                        });
                        if !view.message.is_empty() {
                            ui.label(
                                RichText::new(tools::truncate(&view.message, 62))
                                    .size(10.5)
                                    .color(view.status.color()),
                            );
                        }
                    },
                );

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Remove").clicked() {
                        remove = true;
                    }

                    match view.status {
                        JobStatus::Done => {
                            if let Some(output) = &view.output
                                && ui.small_button("Reveal").clicked()
                            {
                                reveal = Some(output.clone());
                            }
                        }
                        JobStatus::Failed | JobStatus::Ready if self.mode == Mode::Optimize => {}
                        JobStatus::Failed | JobStatus::Ready => {
                            let label = view
                                .target
                                .as_deref()
                                .map(|t| t.to_uppercase())
                                .unwrap_or_else(|| "Select format".into());
                            egui::ComboBox::from_id_salt(("target", view.id))
                                .selected_text(RichText::new(label).size(12.0))
                                .width(130.0)
                                .show_ui(ui, |ui| {
                                    if let Some(from) = view.from.as_deref() {
                                        let mut targets = registry.targets_for(from);
                                        targets.sort_unstable();
                                        for target in targets {
                                            let selected = view.target.as_deref() == Some(target);
                                            if ui
                                                .selectable_label(
                                                    selected,
                                                    RichText::new(target).size(12.0),
                                                )
                                                .clicked()
                                            {
                                                chosen_target = Some(target.to_string());
                                            }
                                        }
                                    }
                                });
                        }
                        JobStatus::Running => {
                            ui.spinner();
                        }
                    }

                    let installed_any = view
                        .from
                        .as_deref()
                        .zip(view.target.as_deref())
                        .map(|(from, to)| {
                            registry
                                .converters_for(from, to)
                                .iter()
                                .any(|c| tools.is_installed(&c.id))
                        })
                        .unwrap_or(false);
                    if self.mode == Mode::Optimize
                        && let Some(new_size) = view.new_size
                    {
                        let saved = view.size.saturating_sub(new_size);
                        if saved > 0 {
                            let percent =
                                (saved as f64 * 100.0 / view.size.max(1) as f64).round() as i64;
                            ui.label(
                                RichText::new(format!("−{percent}%"))
                                    .size(10.5)
                                    .strong()
                                    .color(theme::SUCCESS),
                            );
                        }
                    }

                    if view.status == JobStatus::Ready && view.target.is_some() && !installed_any {
                        ui.label(RichText::new("needs tool").size(10.0).color(theme::WARN));
                    }

                    ui.label(
                        RichText::new(view.status.label())
                            .size(10.5)
                            .color(view.status.color()),
                    );
                });
            });
        });

        if let Some(target) = chosen_target {
            self.jobs[index].target = Some(target);
            self.jobs[index].message.clear();
        }
        if let Some(path) = reveal {
            crate::open_in_file_manager(&path);
        }
        remove
    }

    fn ui_formats(&mut self, ui: &mut egui::Ui) {
        if self.mode == Mode::Optimize {
            self.ui_optimize_panel(ui);
            return;
        }
        ui.label(RichText::new("Convert to").size(15.0).strong());
        ui.label(
            RichText::new("Pick a format for every compatible file in the queue")
                .size(11.0)
                .color(theme::MUTED),
        );
        ui.add_space(6.0);

        ui.add(
            egui::TextEdit::singleline(&mut self.target_filter)
                .hint_text("Search formats…")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);

        let counts = self.target_counts();
        if counts.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Add files to see available formats")
                        .size(12.5)
                        .color(theme::MUTED),
                );
            });
            return;
        }

        let filter = self.target_filter.trim().to_lowercase();
        let mut grouped: Vec<(&str, Vec<(String, usize)>)> = Vec::new();
        for (target, count) in counts {
            if !filter.is_empty() && !target.to_lowercase().contains(&filter) {
                continue;
            }
            let category = self.registry.category_of(&target);
            match grouped.iter_mut().find(|(name, _)| *name == category) {
                Some((_, items)) => items.push((target, count)),
                None => grouped.push((category, vec![(target, count)])),
            }
        }

        let order = self.category_order();
        grouped
            .sort_by_key(|(name, _)| order.iter().position(|c| c == name).unwrap_or(order.len()));

        if grouped.is_empty() {
            ui.add_space(16.0);
            ui.label(
                RichText::new("No format matches your search")
                    .size(12.0)
                    .color(theme::MUTED),
            );
            return;
        }

        let mut chosen: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (category, mut items) in grouped {
                    items.sort_by(|a, b| a.0.cmp(&b.0));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(category.to_uppercase())
                            .size(10.0)
                            .color(theme::MUTED)
                            .strong(),
                    );
                    ui.add_space(2.0);
                    for (target, count) in items {
                        let converters = self
                            .jobs
                            .iter()
                            .filter_map(|job| job.from.as_deref())
                            .filter(|from| !self.registry.converters_for(from, &target).is_empty())
                            .flat_map(|from| self.registry.converters_for(from, &target))
                            .collect::<Vec<&ConverterSpec>>();
                        let installed = converters.iter().any(|c| self.is_installed(&c.id));
                        let names = {
                            let mut unique: Vec<&str> = Vec::new();
                            for converter in &converters {
                                if !unique.contains(&converter.name.as_str()) {
                                    unique.push(&converter.name);
                                }
                            }
                            tools::truncate(&unique.join(", "), 26)
                        };
                        let meta = if !installed {
                            format!("needs {names}")
                        } else if count > 1 {
                            format!("{count} files")
                        } else {
                            names
                        };
                        let selected = self.global_target.eq_ignore_ascii_case(&target);
                        let response =
                            format_row(ui, &target.to_uppercase(), &meta, selected, installed);
                        if response.clicked() {
                            if installed {
                                chosen = Some(target.clone());
                            } else if let Some(converter) =
                                converters.iter().find(|c| !self.is_installed(&c.id))
                            {
                                let id = converter.id.clone();
                                self.offer_install(&id, Some(target.clone()));
                            }
                        }
                    }
                }
            });

        if let Some(target) = chosen {
            self.apply_target(&target);
        }
    }

    fn ui_optimize_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Optimize images").size(15.0).strong());
        ui.label(
            RichText::new(
                "Strips bloated metadata and recompresses at the smallest size that keeps \
                 the picture identical. Nothing is resized or re-encoded unless you ask.",
            )
            .size(11.0)
            .color(theme::MUTED),
        );
        ui.add_space(10.0);

        ui.label(RichText::new("Mode").size(12.0).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let presets: [(bool, &str, &str); 3] = [
                (
                    true,
                    "Lossless",
                    "Pixel-identical. Metadata removed, compression improved.",
                ),
                (
                    false,
                    "Balanced",
                    "Re-encodes at quality 82 — visually identical for most photos.",
                ),
                (
                    false,
                    "Smallest",
                    "Quality 70. Noticeable only on close inspection.",
                ),
            ];
            for (lossless, label, hint) in presets {
                let selected = self.optimize.lossless == lossless
                    && match label {
                        "Lossless" => true,
                        "Balanced" => !lossless && self.optimize.quality >= 80,
                        _ => !lossless && self.optimize.quality < 80,
                    };
                let button =
                    egui::Button::new(RichText::new(label).size(12.0).color(if selected {
                        Color32::WHITE
                    } else {
                        theme::MUTED
                    }))
                    .fill(if selected { theme::ACCENT } else { theme::CARD })
                    .stroke(Stroke::new(1.0, theme::BORDER));
                if ui.add(button).on_hover_text(hint).clicked() {
                    self.optimize.lossless = lossless;
                    self.optimize.quality = match label {
                        "Lossless" => 100,
                        "Balanced" => 82,
                        _ => 70,
                    };
                }
            }
        });

        ui.add_space(10.0);
        ui.add_enabled_ui(!self.optimize.lossless, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("JPEG/WebP quality")
                        .size(11.5)
                        .color(theme::MUTED),
                );
                ui.add(egui::Slider::new(&mut self.optimize.quality, 40..=95));
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut resize = self.optimize.max_dimension.is_some();
            if ui
                .checkbox(&mut resize, "Shrink the long edge to")
                .changed()
            {
                self.optimize.max_dimension = if resize { Some(2048) } else { None };
            }
            if let Some(max) = self.optimize.max_dimension.as_mut() {
                ui.add(egui::DragValue::new(max).range(64..=16384).suffix(" px"));
            }
        });
        if !self.optimize.lossless {
            ui.label(
                RichText::new(
                    "Resizing always loses detail, so it only applies outside lossless mode.",
                )
                .size(10.5)
                .color(theme::WARN),
            );
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        ui.checkbox(
            &mut self.optimize.strip_metadata,
            "Remove metadata (EXIF, GPS, XMP, comments, thumbnails)",
        );
        ui.checkbox(
            &mut self.optimize_in_place,
            "Overwrite the originals instead of writing copies",
        );
        ui.add_enabled_ui(self.optimize_in_place, |ui| {
            ui.checkbox(
                &mut self.optimize_backup,
                "Keep a .orig backup when overwriting",
            );
        });

        ui.add_space(8.0);
        ui.add(
            egui::Slider::new(&mut self.optimize.effort, 0..=6)
                .text("PNG effort")
                .trailing_fill(true),
        )
        .on_hover_text("Higher effort searches harder for a smaller file, but takes longer");

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);

        let count = self.optimizable_jobs();
        let bytes: u64 = self
            .jobs
            .iter()
            .filter(|job| job.raw_ext.as_deref().is_some_and(optimize::is_optimizable))
            .map(|job| job.size)
            .sum();

        if count == 0 {
            ui.label(
                RichText::new("Drop PNG, JPEG, GIF, WebP, SVG, TIFF, AVIF or HEIC files to begin")
                    .size(12.0)
                    .color(theme::MUTED),
            );
        } else {
            ui.label(
                RichText::new(format!("{count} image(s), {} in total", human_size(bytes)))
                    .size(12.0)
                    .strong(),
            );
            ui.label(
                RichText::new(if self.optimize_in_place {
                    "The originals will be replaced"
                } else {
                    "Copies will be written next to your output folder"
                })
                .size(10.5)
                .color(theme::MUTED),
            );
        }

        ui.add_space(10.0);
        let helpers: Vec<&str> = ["jpegtran", "gifsicle", "cwebp", "svgo", "pngquant"]
            .into_iter()
            .filter(|name| tools::find_binary(name).is_some())
            .collect();
        ui.label(
            RichText::new(if helpers.is_empty() {
                "Using only the built-in Rust optimizers".to_string()
            } else {
                format!("Extra optimizers detected: {}", helpers.join(", "))
            })
            .size(10.5)
            .color(theme::MUTED),
        );
    }

    fn ui_footer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Output").size(11.0).color(theme::MUTED));
            let dir = self.settings.output_path();
            let label = tools::truncate(&dir.to_string_lossy(), 44);
            if ui
                .add(
                    egui::Button::new(RichText::new(label).size(11.5).monospace())
                        .fill(theme::BG)
                        .stroke(Stroke::new(1.0, theme::BORDER)),
                )
                .on_hover_text(dir.to_string_lossy().to_string())
                .clicked()
                && let Some(folder) = crate::dialogs::pick_folder()
            {
                self.settings.output_dir = folder.to_string_lossy().into_owned();
                self.settings.save();
            }
            if ui.small_button("Open").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                crate::open_in_file_manager(&dir);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let optimizing = self.mode == Mode::Optimize;
                let ready_jobs = if optimizing {
                    self.optimizable_jobs()
                } else {
                    self.jobs
                        .iter()
                        .filter(|j| j.target.is_some() && j.status != JobStatus::Running)
                        .count()
                };
                let enabled = ready_jobs > 0 && !self.running;
                let label = if self.running {
                    format!(
                        "{} {}/{}…",
                        if optimizing {
                            "Optimizing"
                        } else {
                            "Converting"
                        },
                        self.batch_done,
                        self.batch_total
                    )
                } else if ready_jobs == 0 {
                    if optimizing {
                        "Optimize".to_string()
                    } else {
                        "Convert".to_string()
                    }
                } else if optimizing {
                    format!("Optimize {ready_jobs} image(s)")
                } else {
                    format!("Convert {ready_jobs} file(s)")
                };
                let button = egui::Button::new(RichText::new(label).size(14.0).strong())
                    .fill(if enabled { theme::ACCENT } else { theme::CARD })
                    .stroke(Stroke::NONE)
                    .min_size(egui::vec2(190.0, 36.0));
                if ui.add_enabled(enabled, button).clicked() {
                    let ctx = ui.ctx().clone();
                    if optimizing {
                        self.start_optimize(&ctx);
                    } else {
                        self.start_batch(&ctx);
                    }
                }

                if self.running {
                    let fraction = if self.batch_total == 0 {
                        0.0
                    } else {
                        self.batch_done as f32 / self.batch_total as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(160.0)
                            .show_percentage(),
                    );
                }
            });
        });

        if !self.log.is_empty() {
            ui.add_space(2.0);
            ui.collapsing(RichText::new("Log").size(11.0).color(theme::MUTED), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(110.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.label(
                                RichText::new(line)
                                    .size(10.5)
                                    .monospace()
                                    .color(theme::MUTED),
                            );
                        }
                    });
            });
        }
        ui.add_space(2.0);
    }

    fn ui_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(680.0)
            .default_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.label(RichText::new("Converters").size(15.0).strong());
                    ui.label(
                        RichText::new(
                            "ConvertX uses tools you already have installed (found automatically, or \
                             pick one with Locate…). Install only the ones you need.",
                        )
                        .size(11.5)
                        .color(theme::MUTED),
                    );
                    ui.add_space(6.0);

                    let mut install: Option<String> = None;
                    let mut locate: Option<String> = None;
                    let mut reset: Option<Vec<String>> = None;
                    let specs: Vec<ConverterRow> = self
                        .registry
                        .converters()
                        .iter()
                        .map(|spec| {
                            let status = self.tools.status_of(&spec.id);
                            ConverterRow {
                                id: spec.id.clone(),
                                name: spec.name.clone(),
                                description: spec.description.clone(),
                                installed: status.map(|s| s.available).unwrap_or(false),
                                version: status.and_then(|s| s.version.clone()),
                                missing: status.map(|s| s.missing.clone()).unwrap_or_default(),
                                builtin: spec.builtin,
                                binaries: spec.binaries.clone(),
                            }
                        })
                        .collect();

                    for ConverterRow {
                        id,
                        name,
                        description,
                        installed,
                        version,
                        missing,
                        builtin,
                        binaries,
                    } in specs
                    {
                        let location = self
                            .tools
                            .status_of(&id)
                            .and_then(|status| status.locations.values().next())
                            .map(|path| path.display().to_string())
                            .unwrap_or_default();
                        egui::Frame::new()
                            .fill(theme::CARD)
                            .stroke(Stroke::new(1.0, theme::BORDER))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let color = if installed { theme::SUCCESS } else { theme::MUTED };
                                    let dot = ui.label(RichText::new("●").color(color).size(12.0));
                                    if !location.is_empty() {
                                        dot.on_hover_text(location.clone());
                                    }
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(&name).size(13.0).strong());
                                        let detail = if builtin {
                                            "Built in — no external tool needed".to_string()
                                        } else if installed {
                                            version
                                                .clone()
                                                .unwrap_or_else(|| "Installed".to_string())
                                        } else {
                                            format!("Missing: {}", missing.join(", "))
                                        };
                                        ui.label(
                                            RichText::new(tools::truncate(&detail, 78))
                                                .size(10.5)
                                                .color(theme::MUTED),
                                        );
                                        ui.label(
                                            RichText::new(description)
                                                .size(10.5)
                                                .color(theme::MUTED),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if !installed && !builtin {
                                            if self.installing.as_deref() == Some(id.as_str()) {
                                                ui.spinner();
                                            } else {
                                                if ui.small_button("Install").clicked() {
                                                    install = Some(id.clone());
                                                }
                                                if ui
                                                    .small_button("Locate…")
                                                    .on_hover_text(
                                                        "Already installed somewhere unusual? Pick the program.",
                                                    )
                                                    .clicked()
                                                {
                                                    locate = missing.first().cloned();
                                                }
                                            }
                                        }
                                        let overridden: Vec<String> = binaries
                                            .iter()
                                            .filter(|b| self.settings.custom_paths.contains_key(*b))
                                            .cloned()
                                            .collect();
                                        if !overridden.is_empty()
                                            && ui
                                                .small_button("Reset path")
                                                .on_hover_text("Go back to automatic detection")
                                                .clicked()
                                        {
                                            reset = Some(overridden);
                                        }
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Re-scan tools").clicked() {
                            self.detect_tools(ctx);
                        }
                        if self.detecting {
                            ui.spinner();
                        }
                        let managers = PackageManager::detect_all()
                            .iter()
                            .map(|m| m.label())
                            .collect::<Vec<_>>();
                        ui.label(
                            RichText::new(if managers.is_empty() {
                                "No package manager detected".to_string()
                            } else {
                                format!("Installer: {}", managers.join(", "))
                            })
                            .size(10.5)
                            .color(theme::MUTED),
                        );
                    });

                    ui.add_space(14.0);
                    ui.separator();
                    ui.label(RichText::new("Preferences").size(15.0).strong());
                    ui.add_space(4.0);

                    egui::Grid::new("preferences")
                        .num_columns(2)
                        .spacing([14.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("Parallel conversions");
                            ui.add(egui::Slider::new(&mut self.settings.parallel_jobs, 1..=8));
                            ui.end_row();

                            ui.label("Open output folder when finished");
                            ui.checkbox(&mut self.settings.open_output_after, "");
                            ui.end_row();

                            ui.label("Keep conversion history");
                            ui.checkbox(&mut self.settings.record_history, "");
                            ui.end_row();
                        });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(RichText::new("Advanced").size(15.0).strong());
                    ui.label(
                        RichText::new(
                            "Extra arguments are passed to FFmpeg around the input and output files.",
                        )
                        .size(10.5)
                        .color(theme::MUTED),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("advanced")
                        .num_columns(2)
                        .spacing([14.0, 10.0])
                        .show(ui, |ui| {
                            ui.label("FFmpeg input args");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.ffmpeg_input_args)
                                    .hint_text("-hwaccel vaapi")
                                    .desired_width(320.0),
                            );
                            ui.end_row();

                            ui.label("FFmpeg output args");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.ffmpeg_output_args)
                                    .hint_text("-preset veryfast")
                                    .desired_width(320.0),
                            );
                            ui.end_row();

                            ui.label("SVG render scale");
                            ui.add(
                                egui::DragValue::new(&mut self.settings.svg_scale)
                                    .speed(0.05)
                                    .range(0.1..=8.0),
                            )
                            .on_hover_text("2.0 renders SVG files at twice their nominal size");
                            ui.end_row();

                            ui.label("JPEG quality");
                            ui.add(egui::Slider::new(&mut self.settings.jpeg_quality, 50..=100))
                                .on_hover_text("Used when writing JPEG files");
                            ui.end_row();

                            ui.label("Stop a converter after");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(
                                        &mut self.settings.convert_timeout_secs,
                                    )
                                    .speed(30.0)
                                    .range(0..=86_400),
                                );
                                ui.label(
                                    RichText::new("seconds (0 = never)").size(10.5).color(theme::MUTED),
                                );
                            })
                            .response
                            .on_hover_text("A hung converter is killed so the batch keeps going");
                            ui.end_row();
                        });

                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(RichText::new("Updates").size(15.0).strong());
                    ui.label(
                        RichText::new(format!("You are running version {}", update::CURRENT_VERSION))
                            .size(11.0)
                            .color(theme::MUTED),
                    );
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        if ui.small_button("Check now").clicked() {
                            self.check_updates(ctx);
                        }
                        ui.checkbox(
                            &mut self.settings.check_updates,
                            "Check automatically when the app starts",
                        );
                        if self.checking_updates {
                            ui.spinner();
                        }
                    });

                    if !self.update_status.is_empty() {
                        ui.label(
                            RichText::new(&self.update_status)
                                .size(10.5)
                                .color(theme::MUTED),
                        );
                    }

                    if let Some(release) = self.update.clone() {
                        ui.add_space(4.0);
                        let label = if release.action_label() == "Install and restart" {
                            "Install and restart".to_string()
                        } else {
                            "Open download page".to_string()
                        };
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(label).size(12.0).color(Color32::WHITE),
                                    )
                                    .fill(theme::SUCCESS)
                                    .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                if update::can_self_update() {
                                    match update::install(&release) {
                                        Ok(()) => {
                                            self.settings.save();
                                            // The helper waits for this process to exit.
                                            std::process::exit(0);
                                        }
                                        Err(err) => {
                                            self.update_status = format!("{err} — opening the page instead");
                                            update::open_page(&release.page_url);
                                        }
                                    }
                                } else {
                                    update::open_page(&release.page_url);
                                }
                            }
                            if ui.small_button("What changed").clicked() {
                                update::open_page(&release.page_url);
                            }
                        });
                        if !release.notes.trim().is_empty() {
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical()
                                .max_height(96.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(tools::truncate(&release.notes, 900))
                                            .size(10.5)
                                            .color(theme::MUTED),
                                    );
                                });
                        }
                    }

                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "Settings are stored in your user configuration folder and applied immediately.",
                        )
                        .size(10.5)
                        .color(theme::MUTED),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("A native rewrite of")
                                .size(10.5)
                                .color(theme::MUTED),
                        );
                        ui.hyperlink_to(
                            RichText::new("ConvertX by C4illin").size(10.5),
                            "https://github.com/c4illin/ConvertX",
                        );
                        ui.label(RichText::new("· AGPL-3.0").size(10.5).color(theme::MUTED));
                    });

                    if let Some(binary) = locate
                        && let Some(path) =
                            crate::dialogs::pick_file(&format!("Locate {binary}"))
                        {
                            self.settings
                                .custom_paths
                                .insert(binary, path.to_string_lossy().into_owned());
                            tools::set_overrides(&self.settings.custom_paths);
                            self.settings.save();
                            self.detect_tools(ctx);
                        }
                    if let Some(binaries) = reset {
                        for binary in binaries {
                            self.settings.custom_paths.remove(&binary);
                        }
                        tools::set_overrides(&self.settings.custom_paths);
                        self.settings.save();
                        self.detect_tools(ctx);
                    }
                    if let Some(id) = install {
                        self.offer_install(&id, None);
                    }
                });
            });
        self.show_settings = open;
        if !open {
            self.settings.save();
        }
    }

    fn ui_history(&mut self, ctx: &egui::Context) {
        let mut open = self.show_history;
        egui::Window::new("History")
            .open(&mut open)
            .default_width(760.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} conversions recorded", self.history.len()))
                            .size(11.5)
                            .color(theme::MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Clear").clicked() {
                            history::clear(&mut self.history);
                        }
                    });
                });
                ui.add_space(6.0);

                if self.history.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Nothing converted yet")
                                .size(12.5)
                                .color(theme::MUTED),
                        );
                    });
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &self.history {
                            let color = if entry.ok {
                                theme::SUCCESS
                            } else {
                                theme::ERROR
                            };
                            egui::Frame::new()
                                .fill(theme::CARD)
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("●").color(color).size(10.0));
                                        ui.label(
                                            RichText::new(
                                                entry
                                                    .input
                                                    .rsplit('/')
                                                    .next()
                                                    .unwrap_or(&entry.input),
                                            )
                                            .size(12.0)
                                            .strong(),
                                        );
                                        ui.label(
                                            RichText::new(format!("{} → {}", entry.from, entry.to))
                                                .size(10.5)
                                                .color(theme::ACCENT)
                                                .monospace(),
                                        );
                                        ui.label(
                                            RichText::new(&entry.converter)
                                                .size(10.5)
                                                .color(theme::MUTED),
                                        );
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(entry.when())
                                                        .size(10.0)
                                                        .color(theme::MUTED),
                                                );
                                            },
                                        );
                                    });
                                    if !entry.ok {
                                        ui.label(
                                            RichText::new(tools::truncate(&entry.message, 90))
                                                .size(10.0)
                                                .color(theme::ERROR),
                                        );
                                    }
                                });
                            ui.add_space(3.0);
                        }
                    });
            });
        self.show_history = open;
    }

    fn ui_install_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.install_prompt.take() else {
            return;
        };

        let mut close = false;
        let mut confirmed = false;
        let response = egui::Modal::new(egui::Id::unique("install-prompt")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(
                RichText::new(format!("Install {}", prompt.converter_name))
                    .size(17.0)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "This converter is not installed yet. ConvertX can install it for you with the command below.",
                )
                .size(12.0)
                .color(theme::MUTED),
            );
            ui.add_space(8.0);

            egui::Frame::new()
                .fill(theme::BG)
                .stroke(Stroke::new(1.0, theme::BORDER))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(prompt.command.join(" "))
                            .monospace()
                            .size(11.5)
                            .color(theme::ACCENT),
                    );
                });

            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "Package manager: {}. Nothing runs until you press Install.",
                    prompt.manager.label()
                ))
                .size(10.5)
                .color(theme::MUTED),
            );
            if let Some(note) = &prompt.note {
                ui.label(RichText::new(note).size(10.5).color(theme::WARN));
            }
            if let Some(manual) = &prompt.manual {
                ui.label(RichText::new(manual).size(10.5).color(theme::MUTED));
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                let install = egui::Button::new(RichText::new("Install").strong())
                    .fill(theme::ACCENT)
                    .stroke(Stroke::NONE);
                if ui.add(install).clicked() {
                    confirmed = true;
                    close = true;
                }
            });
        });

        if response.backdrop_response.clicked() {
            close = true;
        }

        if confirmed {
            self.start_install(ctx, prompt);
        } else if !close {
            self.install_prompt = Some(prompt);
        }
    }

    fn ui_install_progress(&mut self, ctx: &egui::Context) {
        if self.installing.is_none() {
            return;
        }
        let name = self
            .installing
            .as_deref()
            .map(|id| self.name_of(id))
            .unwrap_or_default();

        egui::Window::new("Installing")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Installing {name}…")).strong());
                });
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(theme::BG)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for line in &self.install_log {
                                    ui.label(
                                        RichText::new(line)
                                            .monospace()
                                            .size(10.5)
                                            .color(theme::MUTED),
                                    );
                                }
                            });
                    });
            });
    }

    fn ui_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        if toast.at.elapsed() > Duration::from_secs(5) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(1.0, toast.color.gamma_multiply(0.6)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&toast.text).size(12.0).color(toast.color));
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

struct ScrollAreaHelper;

impl ScrollAreaHelper {
    fn show(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, add);
    }
}

fn format_row(
    ui: &mut egui::Ui,
    label: &str,
    meta: &str,
    selected: bool,
    installed: bool,
) -> egui::Response {
    let height = 33.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if selected {
            theme::ACCENT_SOFT
        } else if response.hovered() {
            theme::CARD_HOVER
        } else {
            theme::CARD
        };
        let stroke = if selected {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        ui.painter().rect(
            rect.shrink(1.0),
            CornerRadius::same(8),
            bg,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 11.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(12.5),
            if installed { theme::TEXT } else { theme::MUTED },
        );
        ui.painter().text(
            egui::pos2(rect.right() - 11.0, rect.center().y),
            Align2::RIGHT_CENTER,
            meta,
            FontId::proportional(10.5),
            if installed { theme::MUTED } else { theme::WARN },
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn category_badge(category: &str) -> Badge {
    let (label, color) = match category {
        "images" | "image" => ("IMG", Color32::from_rgb(0x7c, 0xb3, 0xff)),
        "muxer" => ("MEDIA", Color32::from_rgb(0xff, 0x9f, 0x6b)),
        "object" => ("3D", Color32::from_rgb(0xb2, 0x8c, 0xff)),
        "text" => ("TEXT", Color32::from_rgb(0x7d, 0xd8, 0xa8)),
        "calc" => ("SHEET", Color32::from_rgb(0x7d, 0xd8, 0xa8)),
        "ocr" => ("OCR", Color32::from_rgb(0xf0, 0xa8, 0xdd)),
        "document" => ("DOC", Color32::from_rgb(0x6b, 0xd4, 0xd0)),
        "email" => ("MAIL", Color32::from_rgb(0xf2, 0xc2, 0x6b)),
        "contacts" => ("CARD", Color32::from_rgb(0xf2, 0xa5, 0x9b)),
        "jxl" => ("JXL", Color32::from_rgb(0xb8, 0xe0, 0x86)),
        _ => ("FILE", Color32::from_rgb(0x9a, 0xa3, 0xb8)),
    };
    Badge {
        label: label.to_string(),
        color,
    }
}

impl eframe::App for ConvertXApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped(&ctx);
        self.poll_events(&ctx);

        // Keyboard shortcuts: ⌘/Ctrl+O adds files, ⌘/Ctrl+Enter starts conversion.
        let (open_files, start) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.modifiers.command && i.key_pressed(egui::Key::Enter),
            )
        });
        if open_files {
            self.pick_files();
        }
        let (convert_mode, optimize_mode) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::Num1),
                i.modifiers.command && i.key_pressed(egui::Key::Num2),
            )
        });
        if convert_mode {
            self.mode = Mode::Convert;
        }
        if optimize_mode {
            self.mode = Mode::Optimize;
        }

        if start && !self.running {
            if self.mode == Mode::Optimize {
                self.start_optimize(&ctx);
            } else {
                self.start_batch(&ctx);
            }
        }

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.ui_header(ui));

        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.ui_footer(ui));

        egui::Panel::left("files")
            .default_size(620.0)
            .min_size(380.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| self.ui_files(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| self.ui_formats(ui));

        self.ui_settings(&ctx);
        self.ui_history(&ctx);
        self.ui_install_prompt(&ctx);
        self.ui_install_progress(&ctx);
        self.ui_toast(&ctx);

        if self.running || self.detecting || self.installing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }
}

#[allow(dead_code)]
fn short_path(path: &Path, max: usize) -> String {
    let text = path.to_string_lossy();
    tools::truncate(&text, max)
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    fn sample_files() -> Vec<PathBuf> {
        let dir = std::env::temp_dir().join(format!("convertx-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("sample.png");
        let svg = dir.join("logo.svg");
        let docx = dir.join("report.docx");

        let image = image::RgbaImage::from_pixel(32, 32, image::Rgba([40, 120, 220, 255]));
        image.save(&png).unwrap();
        std::fs::write(
            &svg,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#123456"/></svg>"##,
        )
        .unwrap();
        std::fs::write(&docx, b"PK\x03\x04").unwrap();

        vec![png, svg, docx]
    }

    fn run_pass(ctx: &egui::Context, app: &mut ConvertXApp, frame: u32) {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0))),
            ..Default::default()
        };
        input.time = Some(frame as f64 * 0.016);
        let mut output = ctx.run_ui(input, |ui| {
            egui::Panel::top("test-header").show(ui, |ui| app.ui_header(ui));
            egui::Panel::bottom("test-footer").show(ui, |ui| app.ui_footer(ui));
            egui::Panel::left("test-files").show(ui, |ui| app.ui_files(ui));
            app.ui_formats(ui);
        });
        output.textures_delta.clear();
        let ctx = ctx.clone();
        app.ui_settings(&ctx);
        app.ui_history(&ctx);
        app.ui_install_prompt(&ctx);
        app.ui_toast(&ctx);
    }

    #[test]
    fn every_panel_renders_in_both_modes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);

        let mut app = ConvertXApp::build(Settings::default());
        app.tools = Toolchain::detect(Registry::get().converters());
        app.detecting = false;
        app.add_paths(sample_files());

        for mode in [Mode::Convert, Mode::Optimize] {
            app.mode = mode;
            for frame in 0..3 {
                run_pass(&ctx, &mut app, frame);
            }
        }

        assert_eq!(app.jobs.len(), 3);
    }

    #[test]
    fn optimize_panel_reacts_to_mode_changes() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);

        let mut app = ConvertXApp::build(Settings::default());
        app.mode = Mode::Optimize;
        app.optimize.lossless = false;
        app.optimize.quality = 70;
        app.optimize.max_dimension = Some(1024);
        app.optimize_in_place = true;
        app.optimize_backup = true;
        app.add_paths(sample_files());

        run_pass(&ctx, &mut app, 0);
        run_pass(&ctx, &mut app, 1);
        assert_eq!(app.optimizable_jobs(), 2);
    }
}
