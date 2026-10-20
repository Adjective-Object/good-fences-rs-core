//! Small library driver for the differential fixture harness; not a CLI framework.
use std::path::PathBuf;

use bundler::{
    build::{BuildOptions, BuildSession, CssLoad, Entry},
    publish::Cancellation,
};

fn main() -> eyre::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("usage: build ROOT OUTPUT ENTRY..."))?,
    );
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| eyre::eyre!("missing output directory"))?,
    );
    let mut options = BuildOptions::default();
    for path in args {
        if path == "--css-auto-mount" {
            options.css.load = CssLoad::AutoMount;
            continue;
        }
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
        })
    );
    Ok(())
}
