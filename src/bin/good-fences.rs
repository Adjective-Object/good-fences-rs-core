extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use clap::Parser;
use good_fences_rs_core::cli::Cli;
use good_fences_rs_core::evaluate_fences::ImportRuleViolation;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use good_fences_rs_core::import_resolver::TsconfigPathsJson;
use serde::Serialize;
use std::env;
use std::path::Path;
use std::time::Instant;

fn main() {
    // set working dir
    let start = Instant::now();
    let args = Cli::parse();
    let root = Path::new(args.root.as_str());
    let tsconfig_path = args.project;
    let mut tsconfig = TsconfigPathsJson::from_path(tsconfig_path);

    if args.base_url.is_some() {
        tsconfig.compiler_options.base_url = args.base_url;
    }

    assert!(env::set_current_dir(&root).is_ok());
    println!(
        "Successfully changed working directory to {}!",
        root.display()
    );

    println!("beginning file walks");

    let dirs_to_walk = &args.paths.iter().map(|x| x.as_str()).collect();
    let good_fences_runner = GoodFencesRunner::new(tsconfig, dirs_to_walk);

    println!("beginning fence evaluations");
    let violations = good_fences_runner.find_import_violations();
    let elapsed = start.elapsed();

    // Print results and statistics
    println!("Violations: {:#?}", violations);
    println!("Total violations: {}", violations.len());

    // Write results to file
    write_erros_as_json(violations, args.output);
    println!("Elapsed time since start: {:?}", elapsed);
}

fn write_erros_as_json(
    violations: Vec<Result<ImportRuleViolation, String>>,
    err_file_output_path: String,
) {
    let unwraped_violations: Result<Vec<ImportRuleViolation>, String> =
        violations.into_iter().collect();
    match unwraped_violations {
        Ok(v) => {
            match std::fs::write(
                &err_file_output_path,
                serde_json::to_string_pretty(&JsonErrorFile { violations: v }).unwrap(),
            ) {
                Ok(_) => {
                    let cwd = env::current_dir().unwrap().to_string_lossy().to_string();
                    println!(
                        "Violations written to {}",
                        format!("{} at {}", err_file_output_path, cwd)
                    );
                }
                Err(err) => {
                    eprintln!("Unable to write violations to {err_file_output_path}.\nError: {err}")
                }
            };
        }
        Err(e) => {
            eprintln!("Error evaluating fences: {e}");
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonErrorFile<'a> {
    violations: Vec<ImportRuleViolation<'a, 'a>>,
}
