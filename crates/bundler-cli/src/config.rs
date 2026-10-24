use bundler::{
    build::{BuildOptions, CssLoad, CssModules, OutputOptions},
    lazy_modules::{ConstructorExport, LazyRuntime},
};
use eyre::WrapErr;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub root: Option<PathBuf>,
    pub outdir: Option<PathBuf>,
    pub entries: Option<Vec<PathBuf>>,
    #[serde(rename = "traceFile")]
    pub trace_file: Option<PathBuf>,
    target: Target,
    workers: std::num::NonZeroUsize,
    output: Output,
    css: Css,
    #[serde(rename = "resolveConditions")]
    resolve_conditions: Vec<String>,
    cache: Cache,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: None,
            outdir: None,
            entries: None,
            trace_file: None,
            target: Target::default(),
            workers: std::num::NonZeroUsize::MIN,
            output: Output::default(),
            css: Css::default(),
            resolve_conditions: Vec::new(),
            cache: Cache::default(),
        }
    }
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Target {
    #[default]
    Node,
    Browser,
}

#[derive(Clone, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Output {
    base_url: String,
    min_chunk_bytes: usize,
    synthesize_async_boundaries: bool,
    lazy_runtimes: Vec<Runtime>,
    source_maps: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            base_url: OutputOptions::default().base_url,
            min_chunk_bytes: 0,
            synthesize_async_boundaries: false,
            lazy_runtimes: Vec::new(),
            source_maps: false,
        }
    }
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Cache {
    #[default]
    ReadWrite,
    Disabled,
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Runtime {
    module: Constructor,
    import: Constructor,
    action: Constructor,
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Constructor {
    module: String,
    export: String,
}

impl Constructor {
    fn resolve(self, root: &Path) -> eyre::Result<ConstructorExport> {
        eyre::ensure!(
            !self.module.is_empty()
                && !self.module.contains(['\\', ':', '\0'])
                && self
                    .module
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | "..")),
            "lazy runtime module must be a portable root-relative file"
        );
        eyre::ensure!(
            !self.export.trim().is_empty()
                && self.export.len() <= 256
                && !self.export.chars().any(char::is_control),
            "lazy runtime export must be a nonempty name of at most 256 bytes"
        );
        let filename = root.join(self.module);
        let module = filename.canonicalize().wrap_err_with(|| {
            format!("resolving lazy runtime constructor {}", filename.display())
        })?;
        eyre::ensure!(
            module.starts_with(root) && module.is_file(),
            "lazy runtime module must resolve to a file inside the fixture root"
        );
        Ok(ConstructorExport {
            module,
            export: self.export,
        })
    }
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Css {
    load: CssLoad,
    css_modules: CssModules,
    css_modules_exclude: Vec<String>,
}

impl Config {
    pub fn read(filename: &Path) -> eyre::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(filename)
            .wrap_err_with(|| format!("opening configuration {}", filename.display()))?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .wrap_err_with(|| format!("reading configuration {}", filename.display()))?;
        eyre::ensure!(bytes.len() <= 16 * 1024, "config exceeds 16 KiB");
        serde_json::from_slice(&bytes)
            .wrap_err_with(|| format!("parsing configuration {}", filename.display()))
    }

    pub fn into_options(self, root: &Path) -> eyre::Result<BuildOptions> {
        let mut options = BuildOptions::default();
        let config = self;
        options.resolver =
            import_resolver::node_resolver::caching::NodeModulesResolverOptions::default_for_env(
                match config.target {
                    Target::Node => import_resolver::resolve::TargetEnv::Node,
                    Target::Browser => import_resolver::resolve::TargetEnv::Browser,
                },
            );
        let canonical_root = root
            .canonicalize()
            .wrap_err_with(|| format!("resolving configured input root {}", root.display()))?;
        eyre::ensure!(
            config.output.lazy_runtimes.len() <= 16,
            "at most 16 lazy runtimes"
        );
        for runtime in config.output.lazy_runtimes {
            options.output.lazy_runtimes.push(LazyRuntime {
                module: runtime.module.resolve(&canonical_root)?,
                import: runtime.import.resolve(&canonical_root)?,
                action: runtime.action.resolve(&canonical_root)?,
            });
        }
        options.cache = match config.cache {
            Cache::ReadWrite => segment_cache::CacheMode::ReadWrite,
            Cache::Disabled => segment_cache::CacheMode::Disabled,
        };
        eyre::ensure!(
            config.workers.get() <= 64,
            "workers must be between 1 and 64"
        );
        options.execution = bundler::execution::ExecutionOptions::new(config.workers);
        eyre::ensure!(
            config
                .resolve_conditions
                .iter()
                .all(|condition| !condition.trim().is_empty()),
            "resolveConditions must contain nonempty strings"
        );
        eyre::ensure!(
            config
                .resolve_conditions
                .iter()
                .all(|condition| !matches!(condition.as_str(), "import" | "require")),
            "resolveConditions cannot override request-mode conditions import or require"
        );
        options
            .resolver
            .export_conditions
            .extend(config.resolve_conditions);
        options.output.base_url = config.output.base_url;
        options.output.min_chunk_bytes = config.output.min_chunk_bytes;
        options.output.synthesize_async_boundaries = config.output.synthesize_async_boundaries;
        options.output.source_maps = config.output.source_maps;
        options.css.load = config.css.load;
        options.css.css_modules = config.css.css_modules;
        options.css.css_modules_exclude = config.css.css_modules_exclude;
        Ok(options)
    }
}
