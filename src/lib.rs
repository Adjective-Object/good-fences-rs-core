use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::time::Instant;

pub mod cli;
pub mod error;
pub mod evaluate_fences;
pub mod fence;
pub mod fence_collection;
pub mod file_extension;
pub mod get_imports;
pub mod good_fences_runner;
pub mod import_resolver;
mod path_utils;
pub mod walk_dirs;

#[napi]
pub fn good_fences(
    paths: Vec<String>,
    project: String,
    base_url: Option<String>,
    output: Option<String>,
) {
    let args = cli::Cli {
        paths,
        project,
        base_url,
        // root,
        output: output.unwrap_or("good-fences-violations.json".to_owned()),
    };
    run_evaluations(args);
}

pub fn run_evaluations(args: cli::Cli) {
    let start = Instant::now();
    let tsconfig_path = args.project;
    let mut tsconfig = import_resolver::TsconfigPathsJson::from_path(tsconfig_path).unwrap();

    if args.base_url.is_some() {
        tsconfig.compiler_options.base_url = args.base_url;
    }
    println!("beginning file walks");

    let dirs_to_walk = &args.paths.iter().map(|x| x.as_str()).collect();
    let good_fences_runner = good_fences_runner::GoodFencesRunner::new(tsconfig, dirs_to_walk);

    println!("beginning fence evaluations");
    let violations = good_fences_runner.find_import_violations();
    let elapsed = start.elapsed();

    // Print results and statistics
    println!("Violations: {:#?}", violations);
    println!("Total violations: {}", violations.len());

    // Write results to file
    error::write_errors_as_json(violations, args.output);
    println!("Elapsed time since start: {:?}", elapsed);
}
