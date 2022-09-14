extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use clap::{self, Parser};
use good_fences_rs_core::evaluate_fences::ImportRuleViolation;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use serde::Serialize;
use std::env::set_current_dir;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Parser)]
struct Cli {
    // Directories
    paths: Vec<String>,

    #[clap(short, long, default_value = "tsconfig.json")]
    project: String,

    #[clap(short, long)]
    base_url: Option<String>,

    #[clap(short, long, default_value = ".")]
    root: String,

    #[clap(short, long, default_value = "good-fences-errors.log")]
    output: String,
}

fn main() {
    // set working dir
    let start = Instant::now();
    let args = Cli::parse();
    let root = Path::new(args.root.as_str());

    assert!(set_current_dir(&root).is_ok());
    println!(
        "Successfully changed working directory to {}!",
        root.display()
    );

    println!("beginning file walks");
    let tsconfig_path = args.project;
    let dirs_to_walk = &args.paths.iter().map(|x| x.as_str()).collect();
    println!("{:?}", &dirs_to_walk);
    let good_fences_runner = GoodFencesRunner::new(&tsconfig_path, dirs_to_walk);

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
    error_file_output: String,
) {
    let unwraped_violations: Result<Vec<ImportRuleViolation>, String> =
        violations.into_iter().collect();
    match unwraped_violations {
        Ok(v) => {
            match std::fs::write(
                &error_file_output,
                serde_json::to_string_pretty(&JsonErrorFile { violations: v }).unwrap(),
            ) {
                Ok(_) => {
                    println!("See results in {:?}", error_file_output);
                }
                Err(_) => {
                    println!("Unable to write error at {:?}", error_file_output);
                }
            };
        }
        Err(e) => {
            println!("Error unwrapping violations: {:?}", e);
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonErrorFile<'a> {
    violations: Vec<ImportRuleViolation<'a, 'a>>,
}
