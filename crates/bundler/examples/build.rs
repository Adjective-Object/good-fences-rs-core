//! Small library driver for the differential fixture harness; not a CLI framework.
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use tracing_subscriber::prelude::*;

use bundler::{
    build::{BuildOptions, BuildSession, CssLoad, CssModules, Entry, OutputOptions},
    lazy_modules::{ConstructorExport, LazyRuntime},
    publish::Cancellation,
};

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureConfig {
    target: FixtureTarget,
    workers: std::num::NonZeroUsize,
    output: FixtureOutput,
    css: FixtureCss,
    #[serde(rename = "resolveConditions")]
    resolve_conditions: Vec<String>,
    cache: FixtureCache,
}

impl Default for FixtureConfig {
    fn default() -> Self {
        Self {
            target: FixtureTarget::default(),
            workers: std::num::NonZeroUsize::MIN,
            output: FixtureOutput::default(),
            css: FixtureCss::default(),
            resolve_conditions: Vec::new(),
            cache: FixtureCache::default(),
        }
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FixtureTarget {
    #[default]
    Node,
    Browser,
}

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureOutput {
    base_url: String,
    min_chunk_bytes: usize,
    synthesize_async_boundaries: bool,
    lazy_runtimes: Vec<FixtureLazyRuntime>,
    source_maps: bool,
}

impl Default for FixtureOutput {
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

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FixtureCache {
    #[default]
    ReadWrite,
    Disabled,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureLazyRuntime {
    module: FixtureConstructor,
    import: FixtureConstructor,
    action: FixtureConstructor,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureConstructor {
    module: String,
    export: String,
}

impl FixtureConstructor {
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
        let module = root.join(self.module).canonicalize()?;
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

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureCss {
    load: CssLoad,
    css_modules: CssModules,
    css_modules_exclude: Vec<String>,
}

fn main() -> eyre::Result<()> {
    run(std::env::args_os().skip(1), &mut std::io::stdout().lock())
}

fn run(args: impl IntoIterator<Item = OsString>, stdout: &mut impl Write) -> eyre::Result<()> {
    let mut args = args.into_iter();
    let root = PathBuf::from(args.next().ok_or_else(|| {
        eyre::eyre!("usage: build ROOT OUTPUT [--fixture-config FILE] [--trace-file FILE] ENTRY...")
    })?);
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing output directory"))?,
    );
    let mut fixture_config = None;
    let mut trace_file = None;
    let mut entries = Vec::new();
    let mut auto_mount = false;
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional && (arg == "--fixture-config" || arg == "--trace-file") {
            let (slot, missing) = if arg == "--trace-file" {
                (&mut trace_file, "missing trace file")
            } else {
                (&mut fixture_config, "missing fixture config file")
            };
            eyre::ensure!(slot.is_none(), "duplicate option {}", arg.to_string_lossy());
            *slot = Some(PathBuf::from(
                args.next().ok_or_else(|| eyre::eyre!(missing))?,
            ));
        } else if !positional && arg == "--css-auto-mount" {
            auto_mount = true;
        } else if !positional && arg == "--" {
            positional = true;
        } else {
            eyre::ensure!(
                positional || !arg.to_string_lossy().starts_with("--"),
                "unknown option"
            );
            entries.push(arg);
        }
    }
    if let Some(path) = trace_file {
        let file = open_trace(&root, &output, &path)?;
        let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
            .writer(file)
            .include_args(true)
            .include_locations(false)
            .trace_style(tracing_chrome::TraceStyle::Threaded)
            .build();
        // Only these audited span targets are recorded; caller data and unrelated
        // library events must not leak into a profiling artifact.
        let layer = layer.with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.is_span()
                && (metadata.target() == "bundler"
                    || metadata.target().starts_with("bundler::")
                    || metadata.target() == "segment_cache"
                    || metadata.target().starts_with("segment_cache::"))
        }));
        let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
        let result = tracing::dispatcher::with_default(&dispatch, || {
            build(root, output, fixture_config, entries, auto_mount, stdout)
        });
        // Drop after all worker joins and span exits, on errors as well as success.
        drop(dispatch);
        drop(guard);
        result
    } else {
        build(root, output, fixture_config, entries, auto_mount, stdout)
    }
}

fn absolute_destination(path: &Path) -> eyre::Result<PathBuf> {
    eyre::ensure!(
        !path.components().any(|c| c == Component::ParentDir),
        "parent traversal is not supported"
    );
    let absolute = std::path::absolute(path)?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.try_exists()? {
        missing.push(
            existing
                .file_name()
                .ok_or_else(|| eyre::eyre!("invalid destination"))?,
        );
        existing = existing
            .parent()
            .ok_or_else(|| eyre::eyre!("invalid destination"))?;
    }
    let mut canonical = existing.canonicalize()?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn open_trace(root: &Path, output: &Path, path: &Path) -> eyre::Result<std::fs::File> {
    let root = root.canonicalize()?;
    let output = absolute_destination(output)?;
    eyre::ensure!(
        !path.components().any(|c| c == Component::ParentDir),
        "trace path must not contain parent traversal"
    );
    let name = path
        .file_name()
        .ok_or_else(|| eyre::eyre!("missing trace filename"))?;
    eyre::ensure!(
        !name.to_string_lossy().contains(':'),
        "trace filename must not name an alternate data stream"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let destination = parent.join(name);
    eyre::ensure!(
        !destination.starts_with(&output),
        "trace must be outside the output directory"
    );
    eyre::ensure!(
        !destination.starts_with(&root)
            || (parent == root && name.to_string_lossy().starts_with(".bundler-trace-")),
        "trace must be outside the build root, or a root-level .bundler-trace-* file"
    );
    eyre::ensure!(
        !parent
            .ancestors()
            .any(|p| p.join(".bundler-current.json").exists()
                || p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".bundler-generation-"))),
        "trace must not modify a published output tree"
    );
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?)
}

fn build(
    root: PathBuf,
    output: PathBuf,
    fixture_config: Option<PathBuf>,
    entries: Vec<OsString>,
    auto_mount: bool,
    stdout: &mut impl Write,
) -> eyre::Result<()> {
    let mut options = BuildOptions::default();
    if let Some(filename) = fixture_config {
        let mut bytes = Vec::new();
        std::fs::File::open(filename)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        eyre::ensure!(bytes.len() <= 16 * 1024, "fixture config exceeds 16 KiB");
        let config: FixtureConfig = serde_json::from_slice(&bytes)?;
        options.resolver =
            import_resolver::node_resolver::caching::NodeModulesResolverOptions::default_for_env(
                match config.target {
                    FixtureTarget::Node => import_resolver::resolve::TargetEnv::Node,
                    FixtureTarget::Browser => import_resolver::resolve::TargetEnv::Browser,
                },
            );
        let canonical_root = root.canonicalize()?;
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
            FixtureCache::ReadWrite => segment_cache::CacheMode::ReadWrite,
            FixtureCache::Disabled => segment_cache::CacheMode::Disabled,
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
    }
    if auto_mount {
        options.css.load = CssLoad::AutoMount;
    }
    for path in entries {
        let path = PathBuf::from(path);
        let name = path
            .to_str()
            .ok_or_else(|| eyre::eyre!("entry must be UTF-8"))?
            .to_owned();
        options.entries.insert(
            name,
            Entry {
                path,
                output_name: None,
                namespace: true,
            },
        );
    }
    let (build, published) =
        BuildSession::new(root)?.build_and_publish(&options, &output, &Cancellation::default())?;
    writeln!(
        stdout,
        "{}",
        serde_json::json!({
            "entries": published.entries,
            "backend": build.product.artifacts.backend,
            "layout": build.product.artifacts.backend.layout(),
            "directory": published.directory,
            "stylesheets": build.product.artifacts.stylesheets,
            "resources": build.product.artifacts.resources.keys().collect::<Vec<_>>(),
            "generation": build.product.generation,
            "sourceParses": build.counters.source_parses,
            "declarationFingerprints": build.counters.declaration_fingerprints,
            "diskHits": build.counters.disk_hits,
            "resolutions": build.counters.resolutions,
            "emissions": build.counters.emissions,
            "outputParses": build.counters.output_parses,
            "sourceReparses": build.counters.source_reparses,
            "fragmentReparses": build.counters.fragment_reparses,
            "finalValidations": build.counters.final_validations,
            "artifactWrites": published.files_written,
            "warnings": build.warnings,
            "planWarnings": build.product.plan.retention.warnings,
            "publicationWarnings": published.warnings,
            "generatedBoundaries": build.product.plan.generated_boundaries.len(),
            "moduleMembers": build.product.artifacts.members.iter().filter_map(|(node, artifact)| {
                let bundler::chunks::NodeId::Module(source) = node else { return None; };
                Some(serde_json::json!({"source": source, "artifact": artifact}))
            }).collect::<Vec<_>>(),
            "segmentMembers": build.product.artifacts.members.iter().filter_map(|(node, artifact)| {
                let bundler::chunks::NodeId::Segment {module, index} = node else { return None; };
                let roots = build.product.plan.node_roots.get(node).into_iter().flatten().map(|root| {
                    use bundler::chunks::RootId;
                    match root {
                        RootId::Entry(entry) => serde_json::json!({"entry": entry}),
                        RootId::Async(site) => serde_json::json!({"module": site.module, "offset": site.offset}),
                        RootId::AsyncExport {site, name} => serde_json::json!({
                            "module": site.module, "offset": site.offset, "name": name
                        }),
                    }
                }).collect::<Vec<_>>();
                Some(serde_json::json!({"source": module, "segment": index, "artifact": artifact, "roots": roots}))
            }).collect::<Vec<_>>(),
        })
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_tmpdir::TmpDir;

    fn args(root: &Path, extra: &[&str]) -> Vec<OsString> {
        [
            root.as_os_str().to_owned(),
            root.join(".bundler-out").into_os_string(),
        ]
        .into_iter()
        .chain(extra.iter().map(OsString::from))
        .collect()
    }

    fn complete_trace(path: &Path) -> Vec<serde_json::Value> {
        let bytes = std::fs::read(path).unwrap();
        let events: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        let mut stacks = std::collections::BTreeMap::<u64, Vec<String>>::new();
        for event in &events {
            let thread = event["tid"].as_u64().unwrap();
            match event["ph"].as_str().unwrap() {
                "B" => stacks
                    .entry(thread)
                    .or_default()
                    .push(event["name"].as_str().unwrap().to_owned()),
                "E" => {
                    stacks
                        .get_mut(&thread)
                        .unwrap()
                        .pop()
                        .expect("unmatched span exit");
                }
                _ => {}
            }
        }
        assert!(stacks.values().all(Vec::is_empty));
        assert!(events
            .iter()
            .any(|event| event["name"] == "build" && event["ph"] == "B"));
        events
    }

    #[test]
    fn tracing_preserves_json_and_artifacts_and_flushes_on_returned_error() {
        let dir = TmpDir::new();
        let root = dir.root();
        std::fs::write(
            root.join("entry.mjs"),
            "import './other.mjs'; export const value = 'PRIVATE_SOURCE_LITERAL';",
        )
        .unwrap();
        std::fs::write(root.join("other.mjs"), "export const other = 2;").unwrap();
        std::fs::write(root.join("workers.json"), r#"{"workers":4}"#).unwrap();
        let trace_args = |name: &str| {
            let mut command = args(root, &["--trace-file"]);
            command.push(root.join(name).into_os_string());
            command.push("--fixture-config".into());
            command.push(root.join("workers.json").into_os_string());
            command.push("entry.mjs".into());
            command.push("other.mjs".into());
            command
        };
        let mut cold = Vec::new();
        run(trace_args(".bundler-trace-cold.json"), &mut cold).unwrap();
        let cold: serde_json::Value = serde_json::from_slice(&cold).unwrap();
        assert_eq!(cold["sourceParses"], 2);
        assert_eq!(cold["declarationFingerprints"], 1);
        let cold_events = complete_trace(&root.join(".bundler-trace-cold.json"));
        for name in [
            "build.declarations",
            "inventory.admit",
            "inventory.recheck",
            "build.resolution_fingerprint",
            "prepare.package_format",
            "prepare.read_bytes",
            "cache.extract",
            "cache.validate",
            "cache.encode",
            "cache.publish",
            "build.resolve",
            "resolve.dependency",
            "build.plan",
            "emit.planning",
            "emit.owner",
            "emit.finalize",
            "build.finalize",
            "build.render",
            "build.recheck",
            "build.publish",
        ] {
            assert!(
                cold_events.iter().any(|event| event["name"] == name),
                "{name}"
            );
        }
        for name in [
            "build.inventory",
            "inventory.read_directory",
            "inventory.config_probes",
        ] {
            assert!(
                !cold_events.iter().any(|event| event["name"] == name),
                "{name}"
            );
        }
        let mut plain = Vec::new();
        run(args(root, &["entry.mjs", "other.mjs"]), &mut plain).unwrap();
        let plain: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        let mut traced = Vec::new();
        run(trace_args(".bundler-trace-success.json"), &mut traced).unwrap();
        let traced: serde_json::Value = serde_json::from_slice(&traced).unwrap();
        assert_eq!(plain, traced);
        assert_eq!(cold["directory"], traced["directory"]);
        assert_eq!(traced["diskHits"], 2);
        assert_eq!(traced["sourceParses"], 0);
        assert_eq!(traced["sourceReparses"], 0);
        assert_eq!(traced["fragmentReparses"], 0);
        assert_eq!(traced["finalValidations"], cold["finalValidations"]);
        assert_eq!(traced["outputParses"], traced["finalValidations"]);
        assert_eq!(traced["artifactWrites"], 0);
        let trace_path = root.join(".bundler-trace-success.json");
        let events = complete_trace(&trace_path);
        for name in ["cache.read", "cache.decode", "cache.validate"] {
            assert!(events.iter().any(|event| event["name"] == name), "{name}");
        }
        let coordinator = &events
            .iter()
            .find(|event| event["name"] == "build")
            .unwrap()["tid"];
        for name in ["prepare.job", "emit.owner"] {
            let jobs: Vec<_> = events
                .iter()
                .filter(|event| event["name"] == name)
                .collect();
            assert!(!jobs.is_empty());
            assert!(
                jobs.iter().any(|event| &event["tid"] != coordinator),
                "{name}: {jobs:?}; coordinator: {coordinator}"
            );
        }
        assert!(events.iter().any(|event| event["name"] == "emit.owner"
            && event["args"]["owner_id"]
                .as_str()
                .is_some_and(|id| id.parse::<usize>().is_ok())));
        assert!(events.iter().any(|event| event["name"] == "build.publish"));
        for path in [root.join(".bundler-trace-cold.json"), trace_path] {
            let text = std::fs::read_to_string(path).unwrap();
            for private in [
                "PRIVATE_SOURCE_LITERAL",
                "entry.mjs",
                "other.mjs",
                "workers.json",
            ] {
                assert!(!text.contains(private));
            }
        }
        assert!(!String::from_utf8(serde_json::to_vec(&traced).unwrap())
            .unwrap()
            .contains("PRIVATE_SOURCE_LITERAL"));

        // A parse failure occurs after the trace and source worker have started.
        std::fs::write(root.join("entry.mjs"), "export const = ;").unwrap();
        let error_path = root.join(".bundler-trace-error.json");
        let mut command = args(root, &["--trace-file"]);
        command.push(error_path.clone().into_os_string());
        command.push("--fixture-config".into());
        command.push(root.join("workers.json").into_os_string());
        command.push("entry.mjs".into());
        let mut stdout = Vec::new();
        assert!(run(command, &mut stdout).is_err());
        assert!(stdout.is_empty());
        complete_trace(&error_path);

        // Failure while writing the result still closes the completed build trace.
        struct BrokenOutput;
        impl Write for BrokenOutput {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        std::fs::write(root.join("entry.mjs"), "export const value = 1;").unwrap();
        assert!(run(
            trace_args(".bundler-trace-stdout-error.json"),
            &mut BrokenOutput
        )
        .is_err());
        complete_trace(&root.join(".bundler-trace-stdout-error.json"));

        std::fs::write(root.join("workers.json"), "invalid config").unwrap();
        assert!(run(
            trace_args(".bundler-trace-config-error.json"),
            &mut Vec::new()
        )
        .is_err());
        let events: Vec<serde_json::Value> = serde_json::from_slice(
            &std::fs::read(root.join(".bundler-trace-config-error.json")).unwrap(),
        )
        .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn trace_destinations_never_overwrite_inputs_outputs_or_inventory() {
        let dir = TmpDir::new();
        let root = dir.root();
        let output = root.join(".bundler-out");
        std::fs::create_dir(&output).unwrap();
        let input = root.join("entry.mjs");
        std::fs::write(&input, "original").unwrap();
        for path in [&input, &root.join("trace.json"), &output.join("trace.json")] {
            assert!(open_trace(root, &output, path).is_err());
        }
        let reserved = root.join(".bundler-trace-existing.json");
        std::fs::write(&reserved, "original").unwrap();
        assert!(open_trace(root, &output, &reserved).is_err());
        assert_eq!(std::fs::read_to_string(reserved).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(input).unwrap(), "original");
        assert!(!output.join("trace.json").exists());
        assert!(!root.join("trace.json").exists());
        assert!(open_trace(root, &output, &root.join(".bundler-trace-file:stream")).is_err());
        let outer = root.join("project");
        std::fs::create_dir(&outer).unwrap();
        drop(open_trace(&outer, &output, &root.join("outside.json")).unwrap());
    }

    #[test]
    fn trace_arguments_reject_missing_duplicate_and_unknown_options() {
        let dir = TmpDir::new();
        for extra in [
            vec!["--trace-file"],
            vec!["--trace-file", "a", "--trace-file", "b"],
            vec!["--fixture-config", "a", "--fixture-config", "b"],
            vec!["--unknown"],
        ] {
            assert!(run(args(dir.root(), &extra), &mut Vec::new()).is_err());
        }
    }
}
