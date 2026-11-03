use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;

/// A single external conversion backend, as described by `assets/formats.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConverterSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub binaries: Vec<String>,
    #[serde(default)]
    pub version_args: Vec<String>,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub manual: Option<String>,
    #[serde(default)]
    pub install: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub from: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub to: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub priority: usize,
    /// Limits a converter to some operating systems, e.g. `["macos"]`.
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Deserialize)]
struct RegistryFile {
    converters: Vec<ConverterSpec>,
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// Different converters spell the same format differently (`jpg`/`jpeg`,
/// `tif`/`tiff`, `md`/`markdown`). Folding them onto one name keeps every
/// converter in the candidate list for a route instead of silently dropping some.
pub fn canonical_format(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "jpg" | "jpe" | "jfif" | "pjpeg" => "jpeg",
        "tif" => "tiff",
        "htm" | "xhtml" => "html",
        "md" | "commonmark" | "gfm" => "markdown",
        "tex" => "latex",
        "yml" => "yaml",
        other => other,
    }
    .to_string()
}

fn canonicalise(mut spec: ConverterSpec) -> ConverterSpec {
    let fold = |map: &mut BTreeMap<String, Vec<String>>| {
        let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (category, formats) in map.iter() {
            let mut seen: Vec<String> = Vec::new();
            for format in formats {
                let canonical = canonical_format(format);
                if !seen.contains(&canonical) {
                    seen.push(canonical);
                }
            }
            result.entry(category.clone()).or_default().extend(seen);
        }
        *map = result;
    };
    fold(&mut spec.from);
    fold(&mut spec.to);
    spec
}

fn load_converters() -> Vec<ConverterSpec> {
    let upstream: RegistryFile = serde_json::from_str(include_str!("../assets/formats.json"))
        .expect("assets/formats.json is embedded at build time and must be valid");
    let native: RegistryFile = serde_json::from_str(include_str!("../assets/native.json"))
        .expect("assets/native.json is embedded at build time and must be valid");

    let platform = current_platform();
    // Android cannot spawn external tools, so only the built-in engines apply.
    let android = cfg!(target_os = "android");
    upstream
        .converters
        .into_iter()
        .chain(native.converters)
        .filter(|spec| !android || spec.builtin)
        .filter(|spec| spec.platforms.is_empty() || spec.platforms.iter().any(|p| p == platform))
        .map(strip_pixel_dump_targets)
        .map(canonicalise)
        .collect()
}

/// ImageMagick advertises `txt`, `html`, `json` and friends as image outputs,
/// but they are literal pixel dumps rather than text documents. They shadow the
/// real text converters, so they are dropped from the routing table.
fn strip_pixel_dump_targets(mut spec: ConverterSpec) -> ConverterSpec {
    if !matches!(spec.id.as_str(), "imagemagick" | "graphicsmagick") {
        return spec;
    }
    for category in ["images", "image"] {
        if let Some(outputs) = spec.to.get_mut(category) {
            outputs.retain(|format| {
                !matches!(
                    format.as_str(),
                    "txt" | "html" | "htm" | "json" | "xhtml" | "xml" | "data" | "clipboard"
                )
            });
        }
    }
    spec
}

pub struct Registry {
    converters: Vec<ConverterSpec>,
    routes: HashMap<String, Vec<(String, usize)>>,
    targets: Vec<String>,
    target_category: HashMap<String, String>,
    inputs: BTreeSet<String>,
    compound_inputs: Vec<String>,
}

impl Registry {
    pub fn get() -> &'static Registry {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();
        REGISTRY.get_or_init(|| Registry::new(load_converters()))
    }

    fn new(mut converters: Vec<ConverterSpec>) -> Self {
        converters.sort_by_key(|c| c.priority);

        let mut routes: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        let mut targets: Vec<String> = Vec::new();
        let mut target_category: HashMap<String, String> = HashMap::new();
        let mut inputs: BTreeSet<String> = BTreeSet::new();

        for (index, converter) in converters.iter().enumerate() {
            for (category, from_list) in &converter.from {
                let Some(to_list) = converter.to.get(category) else {
                    continue;
                };
                inputs.extend(from_list.iter().cloned());
                for from in from_list {
                    for to in to_list {
                        let entry = routes.entry(from.clone()).or_default();
                        if !entry.iter().any(|(t, i)| t == to && *i == index) {
                            entry.push((to.clone(), index));
                        }
                        if !targets.contains(to) {
                            targets.push(to.clone());
                            target_category.insert(to.clone(), category.clone());
                        }
                    }
                }
            }
        }

        let compound_inputs = inputs
            .iter()
            .filter(|ext| ext.contains('.'))
            .cloned()
            .collect::<Vec<_>>();

        Registry {
            converters,
            routes,
            targets,
            target_category,
            inputs,
            compound_inputs,
        }
    }

    pub fn converters(&self) -> &[ConverterSpec] {
        &self.converters
    }

    pub fn converter(&self, id: &str) -> Option<&ConverterSpec> {
        self.converters.iter().find(|c| c.id == id)
    }

    /// Every format this app can read, across all converters.
    pub fn input_formats(&self) -> impl Iterator<Item = &str> {
        self.inputs.iter().map(String::as_str)
    }

    pub fn is_known_input(&self, extension: &str) -> bool {
        self.inputs.contains(extension)
    }

    pub fn all_targets(&self) -> &[String] {
        &self.targets
    }

    pub fn category_of(&self, target: &str) -> &str {
        self.target_category
            .get(target)
            .map(String::as_str)
            .unwrap_or("other")
    }

    /// Unique output formats that can be produced from `from`, best converters first.
    pub fn targets_for(&self, from: &str) -> Vec<&str> {
        self.routes
            .get(from)
            .map(|entries| {
                let mut seen: Vec<&str> = Vec::new();
                for (to, _) in entries {
                    if !seen.contains(&to.as_str()) {
                        seen.push(to.as_str());
                    }
                }
                seen
            })
            .unwrap_or_default()
    }

    /// All converters that can perform `from -> to`, in priority order.
    pub fn converters_for(&self, from: &str, to: &str) -> Vec<&ConverterSpec> {
        self.routes
            .get(from)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(t, _)| t == to)
                    .map(|(_, index)| &self.converters[*index])
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Picks the highest priority converter, preferring ones that are installed.
    pub fn best_converter(
        &self,
        from: &str,
        to: &str,
        is_installed: impl Fn(&str) -> bool,
    ) -> Option<&ConverterSpec> {
        let candidates = self.converters_for(from, to);
        candidates
            .iter()
            .find(|c| is_installed(&c.id))
            .or_else(|| candidates.first())
            .copied()
    }

    /// Detects the extension of `path`, understanding compound suffixes such as `nii.gz`.
    pub fn detect_extension(&self, path: &Path) -> Option<String> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if let Some(compound) = self
            .compound_inputs
            .iter()
            .find(|ext| name.ends_with(&format!(".{ext}")))
        {
            return Some(compound.clone());
        }
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        if ext.is_empty() { None } else { Some(ext) }
    }
}

/// Extensions are compared in their canonical form, mirroring upstream `normalizeFiletype`.
pub fn normalize_input(extension: &str) -> String {
    match extension.to_lowercase().as_str() {
        "jfif" | "jpg" => "jpeg".into(),
        "htm" => "html".into(),
        "tex" => "latex".into(),
        "md" => "markdown".into(),
        "unknown" => "m4a".into(),
        other => other.to_string(),
    }
}

/// Output extension shown to the user / used on disk, mirroring `normalizeOutputFiletype`.
pub fn normalize_output(extension: &str) -> String {
    match extension.to_lowercase().as_str() {
        "jpeg" => "jpg".into(),
        "latex" => "tex".into(),
        "markdown" | "markdown_phpextra" | "markdown_strict" | "markdown_mmd" => "md".into(),
        "glb2" => "glb".into(),
        "gltf2" => "gltf".into(),
        "objnomtl" => "obj".into(),
        "stlb" => "stl".into(),
        "plyb" => "ply".into(),
        "fbxa" => "fbx".into(),
        "assjson" => "json".into(),
        other => other.to_string(),
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_loads_upstream_formats() {
        let registry = Registry::get();
        assert!(registry.converters().len() >= 20);
        // Alias folding (jpg/jpeg, tif/tiff…) merges a few names, hence 890.
        assert!(registry.input_formats().count() >= 890);
        assert!(registry.all_targets().len() >= 480);
        assert!(registry.targets_for("png").contains(&"pdf"));
        assert!(registry.targets_for("docx").contains(&"pdf"));
        assert!(registry.converters_for("png", "pdf").len() >= 2);
    }

    #[test]
    fn categories_must_match() {
        let registry = Registry::get();
        // vcf is only routed through the contacts category.
        assert!(
            !registry
                .converters_for("vcf", "png")
                .iter()
                .any(|c| c.id == "vcf")
        );
        assert_eq!(registry.converters_for("vcf", "csv").len(), 1);
    }

    #[test]
    fn aliases_fold_into_one_format() {
        let registry = Registry::get();
        let targets = registry.targets_for("svg");
        assert!(
            targets.contains(&"jpeg"),
            "jpeg must be a target: {targets:?}"
        );
        assert!(!targets.contains(&"jpg"), "jpg must fold into jpeg");
        assert_eq!(canonical_format("JPG"), "jpeg");
        assert_eq!(canonical_format("tif"), "tiff");
        assert_eq!(canonical_format("md"), "markdown");

        // Every converter that can write JPEG is a candidate, not just the ones
        // spelled "jpg".
        let candidates = registry.converters_for("svg", "jpeg");
        assert!(candidates.iter().any(|spec| spec.id == "native-svg"));
    }

    #[test]
    fn extension_normalization() {
        assert_eq!(normalize_input("JPG"), "jpeg");
        assert_eq!(normalize_input("md"), "markdown");
        assert_eq!(normalize_output("jpeg"), "jpg");
        assert_eq!(normalize_output("markdown_strict"), "md");
        assert_eq!(normalize_output("glb2"), "glb");
    }

    #[test]
    fn compound_extensions_are_detected() {
        let registry = Registry::get();
        assert_eq!(
            registry
                .detect_extension(Path::new("/tmp/brain.nii.gz"))
                .as_deref(),
            Some("nii.gz")
        );
        assert_eq!(
            registry
                .detect_extension(Path::new("/tmp/photo.PNG"))
                .as_deref(),
            Some("png")
        );
    }
}
