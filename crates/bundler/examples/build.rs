//! Small library driver for the differential fixture harness; not a CLI framework.
use std::{io::Read, path::PathBuf};

use bundler::{
    build::{BuildOptions, BuildSession, CssLoad, CssModules, Entry, OutputOptions},
    publish::Cancellation,
};

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureConfig {
    output: FixtureOutput,
    css: FixtureCss,
}

#[derive(serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureOutput {
    base_url: String,
    min_chunk_bytes: usize,
    synthesize_async_boundaries: bool,
}

impl Default for FixtureOutput {
    fn default() -> Self {
        Self {
            base_url: OutputOptions::default().base_url,
            min_chunk_bytes: 0,
            synthesize_async_boundaries: false,
        }
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureCss {
    load: CssLoad,
    css_modules: CssModules,
}

fn main() -> eyre::Result<()> {
    let mut args = std::env::args_os().skip(1).peekable();
    let root =
        PathBuf::from(args.next().ok_or_else(|| {
            eyre::eyre!("usage: build ROOT OUTPUT [--fixture-config FILE] ENTRY...")
        })?);
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing output directory"))?,
    );
    let mut options = BuildOptions::default();
    if args.peek().is_some_and(|arg| arg == "--fixture-config") {
        args.next();
        let filename = args
            .next()
            .ok_or_else(|| eyre::eyre!("missing fixture config file"))?;
        let mut bytes = Vec::new();
        std::fs::File::open(filename)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        eyre::ensure!(bytes.len() <= 16 * 1024, "fixture config exceeds 16 KiB");
        let config: FixtureConfig = serde_json::from_slice(&bytes)?;
        options.output.base_url = config.output.base_url;
        options.output.min_chunk_bytes = config.output.min_chunk_bytes;
        options.output.synthesize_async_boundaries = config.output.synthesize_async_boundaries;
        options.css.load = config.css.load;
        options.css.css_modules = config.css.css_modules;
    }
    for path in args {
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
    println!(
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
                Some(serde_json::json!({"source": module, "segment": index, "artifact": artifact}))
            }).collect::<Vec<_>>(),
        })
    );
    Ok(())
}
