extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use clap::{self, Parser};
use good_fences_rs_core::evaluate_fences::ImportRuleViolation;
use good_fences_rs_core::good_fences_runner::GoodFencesRunner;
use serde::Serialize;
use std::env::{self, set_current_dir};
use std::path::Path;
use std::time::Instant;

/**
 * Example usage: \n
 * ```
  good-fences packages --root ./my-user/my-project --project custom.tsconfig.json --output my-error-file.json
 * ```
 */
#[derive(Debug, Parser)]
struct Cli {
    /**
     * Dirs to look for fence and source files
     */
    paths: Vec<String>,

    /**
     * The tsconfig file used relative to '--root' argument
     */
    #[clap(short, long, default_value = "tsconfig.json")]
    project: String,

    /**
     *  Overrides `compilerOptions.baseUrl` property read from '--project' argument
     */
    #[clap(short, long)]
    base_url: Option<String>,

    /**
     * Argument to change the cwd of execution
     */
    #[clap(short, long, default_value = ".")]
    root: String,

    /**
     * Output file for violations, relative to '--root' argument
     */
    #[clap(short, long, default_value = "good-fences-violations.json")]
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
                    let cwd = env::current_dir().unwrap().to_string_lossy().to_string();
                    println!(
                        "See results in {:?}",
                        format!("{} at {}", cwd, error_file_output)
                    );
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
