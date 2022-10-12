use napi_derive::napi;
use serde::Serialize;
use std::time::Instant;

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
pub fn good_fences(opts: GoodFencesOptions) -> Vec<GoodFencesError> {
    let start = Instant::now();
    let tsconfig_path = opts.project;
    let mut tsconfig = import_resolver::TsconfigPathsJson::from_path(tsconfig_path).unwrap();

    if opts.base_url.is_some() {
        tsconfig.compiler_options.base_url = opts.base_url;
    }
    println!("beginning file walks");

    let dirs_to_walk = &opts.paths.iter().map(|x| x.as_str()).collect();
    let good_fences_runner = good_fences_runner::GoodFencesRunner::new(
        tsconfig,
        dirs_to_walk,
        opts.ignore_external_fences,
    );

    println!("beginning fence evaluations");
    let violations = good_fences_runner.find_import_violations();
    let elapsed = start.elapsed();

    // Print results and statistics
    println!("Violations: {:#?}", violations);
    println!("Total violations: {}", violations.len());
    let errors: Vec<GoodFencesError> = violations
        .iter()
        .filter_map(|violation| -> Option<GoodFencesError> {
            match violation {
                Ok(v) => {
                    return Some(GoodFencesError {
                        message: "".to_owned(),
                        source_file: Some(v.violating_file_path.to_string()),
                        raw_import: Some(v.violating_import_specifier.to_string()),
                        fence_path: Some(v.violating_fence.fence_path.clone()),
                        detailed_message: format!(
                            "Good-fences violation in {}\n",
                            &v.violating_file_path
                        )
                        .to_string(),
                    })
                }
                Err(_) => return None,
            }
        })
        .collect();
    // Write results to file
    if let Some(output) = opts.err_output_path {
        write_violations_as_json(violations, output);
    }

    println!("Elapsed time since start: {:?}", elapsed);
    errors
}

#[napi(object)]
pub struct GoodFencesOptions {
    pub paths: Vec<String>,
    pub project: String,
    pub base_url: Option<String>,
    pub err_output_path: Option<String>,
    pub ignore_external_fences: bool,
}

#[napi]
pub struct GoodFencesError {
    pub message: String,
    pub source_file: Option<String>,
    pub raw_import: Option<String>,
    pub fence_path: Option<String>,
    pub detailed_message: String,
}

pub fn write_violations_as_json(
    violations: Vec<Result<evaluate_fences::ImportRuleViolation, String>>,
    err_file_output_path: String,
) {
    let unwraped_violations: Result<Vec<evaluate_fences::ImportRuleViolation>, String> =
        violations.into_iter().collect();
    match unwraped_violations {
        Ok(v) => {
            match std::fs::write(
                &err_file_output_path,
                serde_json::to_string_pretty(&JsonErrorFile { violations: v }).unwrap(),
            ) {
                Ok(_) => {
                    let cwd = std::env::current_dir()
                        .unwrap()
                        .to_string_lossy()
                        .to_string();
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
pub struct JsonErrorFile<'a> {
    pub violations: Vec<evaluate_fences::ImportRuleViolation<'a, 'a>>,
}
