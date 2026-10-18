//! Small library driver for the differential fixture harness; not a CLI framework.
use std::path::PathBuf;

use bundler::{
    build::{BuildOptions, BuildSession, Entry},
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
        let path = PathBuf::from(path);
        let name = path
            .to_str()
            .ok_or_else(|| eyre::eyre!("entry must be UTF-8"))?
            .to_owned();
        options.entries.insert(
            name,
            Entry {
                path,
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
            "layout": "native-original-modules-v1",
            "directory": published.directory,
            "generation": build.product.generation,
            "sourceParses": build.counters.source_parses,
            "diskHits": build.counters.disk_hits,
            "resolutions": build.counters.resolutions,
            "emissions": build.counters.emissions,
            "outputParses": build.counters.output_parses,
            "artifactWrites": published.files_written,
            "warnings": build.warnings,
            "publicationWarnings": published.warnings,
        })
    );
    Ok(())
}
