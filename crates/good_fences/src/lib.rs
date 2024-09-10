use std::{collections::HashSet, iter::FromIterator};

extern crate import_resolver;
extern crate js_err;
extern crate serde;
extern crate serde_json;
extern crate swc_utils;
extern crate tsconfig_paths;
extern crate unused_finder;

use anyhow::Context;
use error::EvaluateFencesError;
use napi_derive::napi;
use serde::Serialize;
use walk_dirs::ExternalFences;
pub mod error;
pub mod evaluate_fences;
pub mod fence;
pub mod fence_collection;
pub mod file_extension;
pub mod get_imports;
pub mod good_fences_runner;
pub mod js_unused_finder;
pub mod walk_dirs;

#[napi]
pub fn good_fences(opts: GoodFencesOptions) -> Vec<GoodFencesResult> {
    let mut tsconfig = tsconfig_paths::TsconfigPathsJson::from_path(&opts.project)
        .with_context(|| format!("Unable to find --project path {}", &opts.project))
        .unwrap();

    if opts.base_url.is_some() {
        tsconfig.compiler_options.base_url = opts.base_url;
    }

    let ignored_dirs_regexs = create_ignored_dirs_regexes(opts.ignored_dirs);

    let dirs_to_walk: Vec<&str> = opts.paths.iter().map(|x| x.as_str()).collect();
    let good_fences_runner = good_fences_runner::GoodFencesRunner::new(
        tsconfig,
        &dirs_to_walk,
        match opts.ignore_external_fences {
            Some(ief) => ief,
            None => ExternalFences::Include,
        },
        &ignored_dirs_regexs,
    );

    let eval_results = good_fences_runner.find_import_violations();

    // Print results and statistics
    if !eval_results.violations.is_empty() {
        println!("Violations:");
        eval_results
            .violations
            .iter()
            .for_each(|v| println!("{}", v));
        println!("Total violations: {}", eval_results.violations.len());
    }

    if !eval_results.unresolved_files.is_empty() {
        println!("Unresolved files:",);
        eval_results
            .unresolved_files
            .iter()
            .for_each(|f| println!("{}", f));
        println!(
            "Total unresolved files: {}",
            eval_results.unresolved_files.len()
        );
    }

    let mut errors: Vec<GoodFencesResult> = Vec::new();

    eval_results.violations.iter().for_each(|v| {
        errors.push(GoodFencesResult {
            result_type: GoodFencesResultType::Violation,
            message: "Good fences violation".to_owned(),
            source_file: Some(v.violating_file_path.to_owned()),
            raw_import: Some(v.violating_import_specifier.to_owned()),
            fence_path: Some(v.violating_fence.fence_path.to_owned()),
            detailed_message: v.to_string(),
        });
    });

    eval_results.unresolved_files.iter().for_each(|e| {
        errors.push(GoodFencesResult {
            result_type: GoodFencesResultType::FileNotResolved,
            message: e.to_string(),
            source_file: None,
            raw_import: None,
            fence_path: None,
            detailed_message: e.to_string(),
        });
    });

    // Write results to file
    if let Some(output) = opts.err_output_path {
        write_violations_as_json(
            eval_results.violations,
            eval_results.unresolved_files,
            output,
        )
        .unwrap();
    }

    errors
}

fn create_ignored_dirs_regexes(ignored_dirs: Option<Vec<String>>) -> Vec<regex::Regex> {
    match ignored_dirs {
        Some(dirs) => dirs
            .iter()
            .map(|id| {
                regex::Regex::new(id.as_str())
                    .unwrap_or_else(|_| panic!("unable to create regex from --ignoredDirs {}", &id))
            })
            .collect(),
        None => Vec::new(),
    }
}

#[napi(object)]
pub struct GoodFencesOptions {
    pub paths: Vec<String>,
    pub project: String,
    pub base_url: Option<String>,
    pub err_output_path: Option<String>,
    pub ignore_external_fences: Option<ExternalFences>,
    pub ignored_dirs: Option<Vec<String>>,
}

#[derive(Eq, Debug, PartialEq)]
#[napi]
pub enum GoodFencesResultType {
    FileNotResolved = 0,
    Violation = 1,
}

#[napi]
pub struct GoodFencesResult {
    pub result_type: GoodFencesResultType,
    pub message: String,
    pub source_file: Option<String>,
    pub raw_import: Option<String>,
    pub fence_path: Option<String>,
    pub detailed_message: String,
}

pub fn write_violations_as_json(
    violations: Vec<evaluate_fences::ImportRuleViolation>,
    fence_eval_errors: Vec<EvaluateFencesError>,
    err_file_output_path: String,
) -> anyhow::Result<()> {
    let evaluation_errors: Vec<String> = fence_eval_errors
        .iter()
        .map(|error| error.to_string())
        .collect();
    match std::fs::write(
        &err_file_output_path,
        serde_json::to_string_pretty(&JsonErrorFile {
            violations,
            evaluation_errors,
        })?,
    ) {
        Ok(_) => {
            let cwd = std::env::current_dir()?.to_string_lossy().to_string();
            println!(
                "Violations written to {} at {}", err_file_output_path, cwd,
            );
        }
        Err(err) => {
            return Err(anyhow::format_err!(
                "Unable to write violations to {err_file_output_path}.\nError: {err}"
            ));
        }
    };
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct JsonErrorFile<'a> {
    pub violations: Vec<evaluate_fences::ImportRuleViolation<'a, 'a>>,
    pub evaluation_errors: Vec<String>,
}

/**
 * Members of the node-facing API are kept in
 * this separate module so that the remainder of
 * the crate can be compiled into a test binary
 *
 * References to symbols from the node api require
 * linking to a real instance of node, which means that
 * `cargo test` can't link anything
 */

#[napi]
pub fn find_unused_items(
    config: unused_finder::FindUnusedItemsConfig,
) -> napi::Result<unused_finder::UnusedFinderReport> {
    unused_finder::find_unused_items(config).map_err(|e: js_err::JsErr| e.to_napi())
}

#[napi]
pub fn find_unused_items_for_open_files(
    config: unused_finder::FindUnusedItemsConfig,
    files: Vec<String>,
) -> napi::Result<unused_finder::UnusedFinderReport> {
    match unused_finder::find_unused_items(config) {
        Ok(mut ok) => {
            let files: HashSet<String> = HashSet::from_iter(files);
            ok.unused_files_items.retain(|key, _| files.contains(key));
            Ok(ok)
        }
        Err(e) => Err(e.to_napi()),
    }
}
