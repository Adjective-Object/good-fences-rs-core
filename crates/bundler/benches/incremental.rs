//! Reproducible end-to-end measurements; counters are public pipeline outcomes.
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

use bundler::{
    build::{BuildOptions, BuildResult, BuildSession, Entry},
    publish::{Cancellation, Published},
};
use test_tmpdir::TmpDir;

const MODULES: usize = 64;
const SAMPLES: usize = 7;

struct Sample {
    total_ms: f64,
    cache_bytes: u64,
    build: BuildResult,
    published: Published,
}

fn cache_bytes(root: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            total += cache_bytes(&entry.path())?;
        } else if segment_cache::is_cache_artifact(&entry.path()) {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

fn run(session: &mut BuildSession, options: &BuildOptions, output: &Path) -> eyre::Result<Sample> {
    let start = Instant::now();
    let (build, published) =
        session.build_and_publish(options, output, &Cancellation::default())?;
    eyre::ensure!(
        build.warnings.is_empty() && published.warnings.is_empty(),
        "benchmark IO warnings: {:?} / {:?}",
        build.warnings,
        published.warnings
    );
    let artifacts = &build.product.artifacts;
    assert_eq!(artifacts.modules.len(), 1);
    assert_eq!(build.product.plan.chunks.len(), 1);
    assert_eq!(artifacts.entries["main"], Path::new("main.mjs"));
    assert_eq!(artifacts.files.len(), 1);
    assert_eq!(
        (
            build.counters.source_reparses,
            build.counters.fragment_reparses
        ),
        (0, 0),
    );
    assert_eq!(
        build.counters.final_validations,
        usize::from(cfg!(debug_assertions)) * build.counters.emissions
    );
    assert_eq!(
        build.counters.output_parses,
        build.counters.final_validations
    );
    assert_eq!(
        published.files_written,
        if published.changed {
            artifacts.files.len()
        } else {
            0
        },
    );
    Ok(Sample {
        total_ms: start.elapsed().as_secs_f64() * 1000.0,
        cache_bytes: cache_bytes(session.root())?,
        build,
        published,
    })
}

fn main() -> eyre::Result<()> {
    let mut results: BTreeMap<&str, Vec<Sample>> = BTreeMap::new();
    for _ in 0..SAMPLES {
        let dir = TmpDir::new();
        let root = dir.root();
        fs::write(root.join("package.json"), r#"{"type":"module"}"#)?;
        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"customConditions":[]}}"#,
        )?;
        let package = root.join("node_modules/benchmark-config");
        fs::create_dir_all(&package)?;
        fs::write(
            package.join("package.json"),
            r#"{"type":"module","exports":{".":{"alternate":"./config-b.mjs","default":"./config-a.mjs"}}}"#,
        )?;
        fs::write(package.join("config-a.mjs"), "export const config = 'a';")?;
        fs::write(package.join("config-b.mjs"), "export const config = 'b';")?;
        fs::write(
            root.join("m0.mjs"),
            "export let value = 1; export function bump() { value++; }",
        )?;
        for index in 1..MODULES {
            fs::write(
                root.join(format!("m{index}.mjs")),
                format!(
                    "export * from './m{}.mjs'; export const local{index} = {index};",
                    index - 1
                ),
            )?;
        }
        fs::write(
            root.join("main.mjs"),
            format!(
                "export * from './m{}.mjs'; import {{config}} from 'benchmark-config'; console.log(config);",
                MODULES - 1
            ),
        )?;
        let options = BuildOptions {
            entries: BTreeMap::from([(
                "main".into(),
                Entry {
                    path: "main.mjs".into(),
                    output_name: None,
                    namespace: true,
                },
            )]),
            ..Default::default()
        };
        let output = root.join(".bundler-output");
        let mut session = BuildSession::new(root)?;
        let cold = run(&mut session, &options, &output)?;
        assert_eq!(cold.build.counters.source_parses, MODULES + 2);
        results.entry("cold").or_default().push(cold);

        // New process-equivalent session with warm disk facts and existing output.
        let mut session = BuildSession::new(root)?;
        let warm = run(&mut session, &options, &output)?;
        assert_eq!(warm.build.counters.source_parses, 0);
        assert_eq!(warm.build.counters.disk_hits, MODULES + 2);
        results.entry("warm").or_default().push(warm);

        let noop = run(&mut session, &options, &output)?;
        assert_eq!(noop.build.counters.source_parses, 0);
        assert_eq!(noop.build.counters.resolutions, 0);
        assert_eq!(noop.build.counters.emissions, 0);
        assert_eq!(noop.published.files_written, 0);
        results.entry("no-op").or_default().push(noop);

        let local = root.join("m32.mjs");
        let text = fs::read_to_string(&local)?;
        fs::write(local, format!("{text}\nconst unusedLocal = 42;"))?;
        let edit = run(&mut session, &options, &output)?;
        assert_eq!(edit.build.counters.source_parses, 1);
        results.entry("local-edit").or_default().push(edit);

        fs::write(
            root.join("m0.mjs"),
            "export let value = 1; export function bump() { value++; } export const added = 7;",
        )?;
        let edit = run(&mut session, &options, &output)?;
        assert_eq!(edit.build.counters.source_parses, 1);
        results.entry("export-edit").or_default().push(edit);

        fs::write(
            root.join("tsconfig.json"),
            r#"{"compilerOptions":{"customConditions":["alternate"]}}"#,
        )?;
        let edit = run(&mut session, &options, &output)?;
        assert_eq!(edit.build.counters.source_parses, 1);
        results.entry("config-edit").or_default().push(edit);
    }
    println!("64-module star chain + entry + configured dependency; {SAMPLES} fresh-root samples per scenario");
    println!("scenario,median_total_ms,min_total_ms,max_total_ms,median_build_ms,extract,resolutions,link,output_parses,source_reparses,fragment_reparses,final_validations,memory_hits,disk_hits,artifact_writes,cache_bytes");
    for name in [
        "cold",
        "warm",
        "no-op",
        "local-edit",
        "export-edit",
        "config-edit",
    ] {
        let samples = results.get_mut(name).expect("all scenarios");
        samples.sort_by(|a, b| a.total_ms.total_cmp(&b.total_ms));
        let middle = &samples[SAMPLES / 2];
        let c = &middle.build.counters;
        println!(
            "{name},{:.3},{:.3},{:.3},{:.3},{},{},{},{},{},{},{},{},{},{},{}",
            middle.total_ms,
            samples[0].total_ms,
            samples[SAMPLES - 1].total_ms,
            c.elapsed.as_secs_f64() * 1000.0,
            c.source_parses,
            c.resolutions,
            c.emissions,
            c.output_parses,
            c.source_reparses,
            c.fragment_reparses,
            c.final_validations,
            c.memory_hits,
            c.disk_hits,
            middle.published.files_written,
            middle.cache_bytes
        );
    }
    Ok(())
}
