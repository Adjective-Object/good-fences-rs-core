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
    #[serde(rename = "resolveExtensions")]
    resolve_extensions: Option<Vec<String>>,
    cache: Cache,
    #[serde(rename = "policyAllowlist")]
    policy_allowlist: Vec<PolicyAllowance>,
    env: std::collections::BTreeMap<String, serde_json::Value>,
    locale: bundler::build::LocaleOptions,
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
            resolve_extensions: None,
            cache: Cache::default(),
            policy_allowlist: Vec::new(),
            env: Default::default(),
            locale: Default::default(),
        }
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyAllowance {
    source: String,
    policies: Vec<bundler::build::PolicyName>,
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
        options.env = config.env;
        config
            .locale
            .validate()
            .wrap_err("validating locale configuration")?;
        options.locale = config.locale;
        options.policy_allowlist = config
            .policy_allowlist
            .into_iter()
            .map(|entry| bundler::build::PolicyAllowance::new(entry.source, entry.policies))
            .collect::<eyre::Result<_>>()?;
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
        if let Some(extensions) = config.resolve_extensions {
            options.resolver.prefer_js_extension = false;
            options.resolver.extensions = extensions
                .into_iter()
                .map(|extension| {
                    let suffix = extension.strip_prefix('.').unwrap_or(&extension);
                    eyre::ensure!(
                        suffix.split('.').all(|part| !part.is_empty())
                            && !suffix.chars().any(|character| {
                                character.is_whitespace()
                                    || character.is_control()
                                    || "/\\:*?\"<>|".contains(character)
                            }),
                        "resolveExtensions entries must be nonempty file suffixes without whitespace, path separators or reserved characters: {extension:?}"
                    );
                    Ok(suffix.to_owned())
                })
                .collect::<eyre::Result<_>>()?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use test_tmpdir::TmpDir;

    #[test]
    fn localization_configuration_is_strict_and_preserves_empty_pseudo_options() {
        let dir = TmpDir::new();
        for json in [
            r#"{"locale":null}"#,
            r#"{"locale":"fr"}"#,
            r#"{"locale":{"default":42}}"#,
            r#"{"locale":{"unknown":true}}"#,
            r#"{"locale":{"ploc":{"extend":"0.2"}}}"#,
            r#"{"locale":{"ploc":{"omitKeys":true}}}"#,
            r#"{"locale":{"ploc":{"unknown":true}}}"#,
        ] {
            assert!(serde_json::from_str::<Config>(json).is_err(), "{json}");
        }
        for locale in ["../fr", "fr/x", "fr\\x", "."] {
            let config: Config =
                serde_json::from_value(serde_json::json!({"locale":{"default":locale}})).unwrap();
            assert!(config.into_options(dir.root()).is_err());
        }
        let config: Config = serde_json::from_str(
            r#"{"locale":{"default":"ploc","ploc":{"prepend":"","append":"","startDelimiter":"","endDelimiter":"","extend":-1,"override":"","omitKeys":["a"]}}}"#,
        ).unwrap();
        let locale = config.into_options(dir.root()).unwrap().locale;
        assert_eq!(locale.default, "ploc");
        assert_eq!(locale.ploc.prepend.as_deref(), Some(""));
        assert_eq!(locale.ploc.end_delimiter.as_deref(), Some(""));
        assert_eq!(locale.ploc.extend, Some(-1.0));
        assert!(locale.ploc.omit_keys.contains("a"));
    }

    #[test]
    fn env_is_an_explicit_json_object() {
        let dir = TmpDir::new();
        for json in [
            r#"{"env":null}"#,
            r#"{"env":[]}"#,
            r#"{"env":"secret"}"#,
            r#"{"env":42}"#,
        ] {
            assert!(serde_json::from_str::<Config>(json).is_err(), "{json}");
        }
        let defaults: Config = serde_json::from_str("{}").unwrap();
        assert!(defaults.into_options(dir.root()).unwrap().env.is_empty());
        let config: Config = serde_json::from_str(
            r#"{"env":{"MODE":"production","nested":{"ok":true},"count":2,"empty":null}}"#,
        )
        .unwrap();
        let options = config.into_options(dir.root()).unwrap();
        assert_eq!(options.env["MODE"], "production");
        assert_eq!(options.env["nested"]["ok"], true);
        assert_eq!(options.env["count"], 2);
        assert_eq!(options.env["empty"], serde_json::Value::Null);
    }

    #[test]
    fn policy_allowlist_rejects_unknown_names_fields_and_nonportable_paths() {
        let root = TmpDir::new();
        for source in [
            "",
            "/absolute.js",
            "C:/absolute.js",
            "../a.js",
            "./a.js",
            "a//b.js",
            "a/../b.js",
            "a\\b.js",
            "*.js",
            "a?.js",
            "a/\u{1b}.js",
        ] {
            let json =
                serde_json::json!({"policyAllowlist":[{"source":source,"policies":["uses-eval"]}]});
            let config: Config = serde_json::from_value(json).unwrap();
            let error = config.into_options(root.root()).err().unwrap();
            assert!(
                format!("{error:#}").contains("policyAllowlist source"),
                "{source:?}: {error:#}"
            );
        }
        for json in [
            r#"{"policyAllowlist":[{"source":"a.js","policies":["not-a-policy"]}]}"#,
            r#"{"policyAllowlist":[{"source":"a.js","policies":["uses-eval"],"extra":true}]}"#,
            r#"{"policyAllowlist":[{"source":"a.js"}]}"#,
            r#"{"policyAllowlist":{"source":"a.js","policies":["uses-eval"]}}"#,
        ] {
            assert!(serde_json::from_str::<Config>(json).is_err(), "{json}");
        }
        let config: Config =
            serde_json::from_str(r#"{"policyAllowlist":[{"source":"a.js","policies":[]}]}"#)
                .unwrap();
        assert!(config.into_options(root.root()).is_err());
        let config: Config = serde_json::from_str(r#"{"policyAllowlist":[{"source":"node_modules/lottie-web/build/player/lottie.js","policies":["uses-eval"]}]}"#).unwrap();
        let options = config.into_options(root.root()).unwrap();
        assert_eq!(
            options.policy_allowlist[0].source(),
            "node_modules/lottie-web/build/player/lottie.js"
        );
        assert_eq!(
            options.policy_allowlist[0].policies(),
            &std::collections::BTreeSet::from([bundler::build::PolicyName::UsesEval])
        );
    }
}
