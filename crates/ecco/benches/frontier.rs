use std::{hint::black_box, path::Path, time::Instant};

use ecco::{infer_function_contracts, parse_declarations, parse_from_ast};
use eyre::{Result, WrapErr};
use oxc_allocator::Allocator;
use oxc_semantic::SemanticBuilder;

fn measure(name: &str, iterations: usize, mut work: impl FnMut()) {
    work();
    let mut samples = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        for _ in 0..iterations {
            work();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}: iterations={iterations} median_us={:.3} min_us={:.3} max_us={:.3}",
        samples[2], samples[0], samples[4]
    );
}

fn run() -> Result<()> {
    let depth = 192;
    let cases = [
        ("ordinary", "identity(a + 1)".to_owned()),
        ("wide", format!("({})", vec!["1 + 2"; 256].join(","))),
        ("logical", vec!["a"; depth].join(" || ")),
        (
            "calls",
            format!("{}a{}", "identity(".repeat(depth), ")".repeat(depth)),
        ),
        (
            "objects",
            format!("{}a{}", "({key:".repeat(depth), "})".repeat(depth)),
        ),
    ];
    let iterations = std::env::var("ECCO_FRONTIER_ITERATIONS")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(100);
    eyre::ensure!(iterations > 0, "benchmark iterations must be positive");
    println!("frontier benchmark: stack_bytes=67108864 depth={depth} samples=5");
    for (name, expression) in cases {
        let source = format!("export function fixture(a) {{ return {expression}; }}");
        let allocator = Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            &source,
            Path::new("frontier-benchmark.js"),
            oxc_utils_parse::ModuleFormat::Esm,
        )?;
        let semantic = SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        eyre::ensure!(semantic.diagnostics.is_empty(), "benchmark semantic errors");
        let facts = parse_declarations(
            "global fn identity(value) { call_effects = false returns = value }",
        )?;
        measure(&format!("{name}/infer"), iterations, || {
            black_box(
                infer_function_contracts(&parsed.program, &semantic.semantic, &facts).unwrap(),
            );
        });
        measure(&format!("{name}/extract"), iterations, || {
            black_box(parse_from_ast(&parsed.program, &semantic.semantic, facts.clone()).unwrap());
        });
    }
    Ok(())
}

fn main() -> Result<()> {
    // Identical benchmark-only stack for the recursive baseline and frontier.
    // Production/default-stack regressions must not use this accommodation.
    std::thread::Builder::new()
        .name("frontier-benchmark".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .wrap_err("spawning frontier benchmark worker")?
        .join()
        .map_err(|_| eyre::eyre!("frontier benchmark worker panicked"))?
}
