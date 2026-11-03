use crate::registry::ConverterSpec;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{OnceLock, RwLock};

/// Directories that GUI launches on macOS do not inherit in `PATH`, but that host
/// most of our converter binaries. They are searched explicitly and also added to
/// the `PATH` of every child process we spawn.
fn extra_binary_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/opt/local/bin",
        "/Library/TeX/texbin",
        "/usr/local/texlive/2025/bin/universal-darwin",
        "/usr/local/texlive/2024/bin/universal-darwin",
        "/usr/bin",
        "/snap/bin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("bin"));
        dirs.push(home.join("Library/Python/3.12/bin"));
        dirs.push(home.join("Library/Python/3.13/bin"));
        dirs.push(home.join("Library/Python/3.14/bin"));
    }
    dirs
}

/// Some tools only ship inside macOS application bundles.
fn bundle_candidates(name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let bundle = match name {
        "soffice" => Some(("LibreOffice.app", "soffice")),
        "inkscape" => Some(("Inkscape.app", "inkscape")),
        "ebook-convert" => Some(("calibre.app", "ebook-convert")),
        _ => None,
    };
    if let Some((app, exe)) = bundle {
        let mut roots = vec![PathBuf::from("/Applications")];
        roots.push(crate::settings::home_dir().join("Applications"));
        for root in roots {
            found.push(root.join(app).join("Contents/MacOS").join(exe));
        }
    }
    if cfg!(target_os = "windows") {
        let windows_dirs: &[(&str, &str)] = match name {
            "soffice" => &[("LibreOffice", "program")],
            "inkscape" => &[("Inkscape", "bin")],
            "ebook-convert" => &[("Calibre2", "")],
            "magick" | "convert" => &[("ImageMagick-7.1.1-Q16-HDRI", "")],
            _ => &[],
        };
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(var) {
                for (dir, sub) in windows_dirs {
                    found.push(
                        PathBuf::from(&root)
                            .join(dir)
                            .join(sub)
                            .join(format!("{name}.exe")),
                    );
                }
            }
        }
    }
    if name == "xelatex" || name == "latexmk" {
        found.push(PathBuf::from("/Library/TeX/texbin").join(name));
    }
    found
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn overrides() -> &'static RwLock<HashMap<String, PathBuf>> {
    static OVERRIDES: OnceLock<RwLock<HashMap<String, PathBuf>>> = OnceLock::new();
    OVERRIDES.get_or_init(Default::default)
}

/// Binary paths the user picked by hand; they win over automatic detection.
pub fn set_overrides(map: &std::collections::BTreeMap<String, String>) {
    if let Ok(mut guard) = overrides().write() {
        *guard = map
            .iter()
            .map(|(k, v)| (k.clone(), PathBuf::from(v)))
            .collect();
    }
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(guard) = overrides().read()
        && let Some(path) = guard.get(name)
        && is_executable(path)
    {
        return Some(path.clone());
    }
    if let Ok(path) = which::which(name) {
        return Some(path);
    }
    for dir in extra_binary_dirs() {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    bundle_candidates(name)
        .into_iter()
        .find(|candidate| is_executable(candidate))
}

fn augmented_path() -> String {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for part in std::env::split_paths(&path) {
            if !part.as_os_str().is_empty() {
                seen.insert(part);
            }
        }
    }
    for dir in extra_binary_dirs() {
        if dir.is_dir() {
            seen.insert(dir);
        }
    }
    let joined: Vec<PathBuf> = seen.into_iter().collect();
    std::env::join_paths(joined)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct ToolStatus {
    pub available: bool,
    pub version: Option<String>,
    pub missing: Vec<String>,
    pub locations: HashMap<String, PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Toolchain {
    pub binaries: HashMap<String, PathBuf>,
    pub status: HashMap<String, ToolStatus>,
    pub path: String,
}

impl Toolchain {
    pub fn detect(converters: &[ConverterSpec]) -> Toolchain {
        let mut binaries = HashMap::new();
        let mut status = HashMap::new();

        for converter in converters {
            if converter.builtin {
                status.insert(
                    converter.id.clone(),
                    ToolStatus {
                        available: true,
                        version: Some("built in".into()),
                        ..Default::default()
                    },
                );
                continue;
            }

            let mut missing = Vec::new();
            let mut locations = HashMap::new();
            for binary in &converter.binaries {
                match find_binary(binary) {
                    Some(path) => {
                        binaries
                            .entry(binary.clone())
                            .or_insert_with(|| path.clone());
                        locations.insert(binary.clone(), path);
                    }
                    None => missing.push(binary.clone()),
                }
            }

            let version = if missing.is_empty() {
                probe_version(
                    locations.values().next().map(PathBuf::as_path),
                    &converter.version_args,
                )
            } else {
                None
            };

            status.insert(
                converter.id.clone(),
                ToolStatus {
                    available: missing.is_empty(),
                    version,
                    missing,
                    locations,
                },
            );
        }

        Toolchain {
            binaries,
            status,
            path: augmented_path(),
        }
    }

    pub fn is_installed(&self, converter_id: &str) -> bool {
        self.status.get(converter_id).is_some_and(|s| s.available)
    }

    pub fn status_of(&self, converter_id: &str) -> Option<&ToolStatus> {
        self.status.get(converter_id)
    }

    pub fn path_of(&self, binary: &str) -> Option<&Path> {
        self.binaries.get(binary).map(PathBuf::as_path)
    }

    /// Builds a `Command` using the resolved absolute path when available, so
    /// conversions work even when the app is launched from Finder.
    pub fn command(&self, binary: &str) -> Command {
        let program = self
            .path_of(binary)
            .map(|p| p.as_os_str().to_os_string())
            .unwrap_or_else(|| binary.into());
        let mut command = Command::new(program);
        command.env("PATH", &self.path);
        command
    }
}

fn probe_version(binary: Option<&Path>, args: &[String]) -> Option<String> {
    let binary = binary?;
    if args.is_empty() {
        return None;
    }
    let output = Command::new(binary)
        .args(args)
        .env("PATH", augmented_path())
        .output()
        .ok()?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Copyright"))
        .unwrap_or("");
    if line.is_empty() {
        return None;
    }
    Some(truncate(line, 72))
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Brew,
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Apk,
    Winget,
    Choco,
    Scoop,
    Cargo,
    Pipx,
    Pip,
    Cpan,
    Go,
}

impl PackageManager {
    /// Ordered by preference for the current OS.
    pub fn detect_all() -> Vec<PackageManager> {
        let mut managers = Vec::new();
        let candidates = if cfg!(target_os = "macos") {
            vec![
                PackageManager::Brew,
                PackageManager::Pipx,
                PackageManager::Pip,
                PackageManager::Cargo,
                PackageManager::Cpan,
                PackageManager::Go,
            ]
        } else if cfg!(target_os = "windows") {
            vec![
                PackageManager::Winget,
                PackageManager::Choco,
                PackageManager::Scoop,
                PackageManager::Pipx,
                PackageManager::Pip,
                PackageManager::Cargo,
                PackageManager::Cpan,
                PackageManager::Go,
            ]
        } else {
            vec![
                PackageManager::Apt,
                PackageManager::Dnf,
                PackageManager::Pacman,
                PackageManager::Zypper,
                PackageManager::Apk,
                PackageManager::Pipx,
                PackageManager::Pip,
                PackageManager::Cargo,
                PackageManager::Cpan,
                PackageManager::Go,
            ]
        };
        for manager in candidates {
            if find_binary(manager.program()).is_some() {
                managers.push(manager);
            }
        }
        managers
    }

    pub fn key(&self) -> &'static str {
        match self {
            PackageManager::Brew => "brew",
            PackageManager::Apt => "apt",
            PackageManager::Dnf => "dnf",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
            PackageManager::Apk => "apk",
            PackageManager::Winget => "winget",
            PackageManager::Choco => "choco",
            PackageManager::Scoop => "scoop",
            PackageManager::Cargo => "cargo",
            PackageManager::Pipx => "pipx",
            PackageManager::Pip => "pip",
            PackageManager::Cpan => "cpan",
            PackageManager::Go => "go",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PackageManager::Brew => "Homebrew",
            PackageManager::Apt => "APT",
            PackageManager::Dnf => "DNF",
            PackageManager::Pacman => "Pacman",
            PackageManager::Zypper => "Zypper",
            PackageManager::Apk => "APK",
            PackageManager::Winget => "winget",
            PackageManager::Choco => "Chocolatey",
            PackageManager::Scoop => "Scoop",
            PackageManager::Cargo => "Cargo",
            PackageManager::Pipx => "pipx",
            PackageManager::Pip => "pip",
            PackageManager::Cpan => "CPAN",
            PackageManager::Go => "Go",
        }
    }

    pub fn program(&self) -> &'static str {
        match self {
            PackageManager::Brew => "brew",
            PackageManager::Apt => "apt-get",
            PackageManager::Dnf => "dnf",
            PackageManager::Pacman => "pacman",
            PackageManager::Zypper => "zypper",
            PackageManager::Apk => "apk",
            PackageManager::Winget => "winget",
            PackageManager::Choco => "choco",
            PackageManager::Scoop => "scoop",
            PackageManager::Cargo => "cargo",
            PackageManager::Pipx => "pipx",
            PackageManager::Pip => "python3",
            PackageManager::Cpan => "cpan",
            PackageManager::Go => "go",
        }
    }

    pub fn needs_elevation(&self) -> bool {
        matches!(
            self,
            PackageManager::Apt
                | PackageManager::Dnf
                | PackageManager::Pacman
                | PackageManager::Zypper
                | PackageManager::Apk
        )
    }
}

/// The full command line that installs `spec` with `manager`, if we know one.
pub fn install_argv(spec: &ConverterSpec, manager: PackageManager) -> Option<Vec<String>> {
    let args = spec.install.get(manager.key())?;
    let mut argv: Vec<String> = Vec::new();

    match manager {
        PackageManager::Pip => {
            argv.push("python3".into());
            argv.push("-m".into());
            argv.push("pip".into());
        }
        PackageManager::Pipx => {
            find_binary("pipx")?;
            argv.push("pipx".into());
        }
        other => argv.push(other.program().to_string()),
    }

    argv.extend(args.iter().cloned());
    Some(argv)
}

/// Same as [`install_argv`] but with elevation handled for system package managers.
pub fn elevated(argv: &[String], manager: PackageManager) -> (String, Vec<String>, Option<String>) {
    if manager.needs_elevation() && !is_root() {
        if find_binary("pkexec").is_some() {
            let mut args = argv.to_vec();
            return ("pkexec".into(), args.split_off(1), None);
        }
        if find_binary("sudo").is_some() {
            let mut args = vec!["-n".to_string()];
            args.extend(argv.iter().cloned());
            return (
                "sudo".into(),
                args,
                Some("This installer needs administrator rights. Run the command in a terminal if it fails.".into()),
            );
        }
    }
    let mut argv = argv.to_vec();
    let program = argv.remove(0);
    (program, argv, None)
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc_geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: forwarded from the caller, geteuid is always safe to call.
    unsafe { geteuid() }
}

/// Best-effort list of package managers that can install a given converter.
pub fn available_installers(spec: &ConverterSpec) -> Vec<(PackageManager, Vec<String>)> {
    let mut out = Vec::new();
    for manager in PackageManager::detect_all() {
        if let Some(argv) = install_argv(spec, manager) {
            out.push((manager, argv));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    #[test]
    fn detection_covers_every_converter() {
        let registry = Registry::get();
        let chain = Toolchain::detect(registry.converters());
        assert_eq!(chain.status.len(), registry.converters().len());
        // The built-in VCF converter never needs an external tool.
        assert!(chain.is_installed("vcf"));
    }

    #[test]
    fn install_argv_is_derived_per_manager() {
        let registry = Registry::get();
        let ffmpeg = registry.converter("ffmpeg").unwrap();
        let argv = install_argv(ffmpeg, PackageManager::Brew).unwrap();
        assert_eq!(argv, vec!["brew", "install", "ffmpeg"]);

        let vcf = registry.converter("vcf").unwrap();
        assert!(install_argv(vcf, PackageManager::Brew).is_none());
    }
}
