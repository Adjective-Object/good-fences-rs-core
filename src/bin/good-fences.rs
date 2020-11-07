extern crate good_fences_rs_core; // Optional in Rust 2018
extern crate serde_json;
use good_fences_rs_core::evaluate_fences::{evaluate_fences, ImportRuleViolation};
use good_fences_rs_core::fence::Fence;
use good_fences_rs_core::fence_collection::FenceCollection;
use good_fences_rs_core::file_extension::no_ext;
use good_fences_rs_core::import_resolver::TsconfigPathsJson;
use good_fences_rs_core::walk_dirs::{discover_fences_and_files, SourceFile, WalkFileData};
use std::collections::HashMap;
use std::env::set_current_dir;
use std::fs::File;
use std::io::BufReader;
use std::iter::FromIterator;
use std::path::Path;

fn main() {
    // set working dir
    let root = Path::new("/home/adjective/Projects/client-web");
    assert!(set_current_dir(&root).is_ok());
    println!(
        "Successfully changed working directory to {}!",
        root.display()
    );

    // load tsconfig.json
    let file = File::open("tsconfig.paths.json").unwrap();
    let buf_reader = BufReader::new(file);
    let tsconfig_paths_json: TsconfigPathsJson = serde_json::from_reader(buf_reader).unwrap();

    // find files
    println!("File Walking 'packages'");
    let package_files = discover_fences_and_files("packages");
    println!("File Walking 'shared'");
    let shared_files = discover_fences_and_files("shared");
    let fences: Vec<&Fence> = package_files
        .iter()
        .chain(shared_files.iter())
        .filter(|file| match file {
            WalkFileData::Fence(_fence) => true,
            _ => false,
        })
        .map(|file| match file {
            WalkFileData::Fence(file) => file,
            _ => panic!("already filtered to Fences?"),
        })
        .collect();
    let sources: Vec<&SourceFile> = package_files
        .iter()
        .chain(shared_files.iter())
        .filter(|file| match file {
            WalkFileData::SourceFile(source_file) => true,
            _ => false,
        })
        .map(|file| match file {
            WalkFileData::SourceFile(file) => file,
            _ => panic!("already filtered to SourceFiles?"),
        })
        .collect();

    // build sources map
    let source_file_map: HashMap<&str, &SourceFile> = HashMap::from_iter(
        sources
            .iter()
            .map(|source_file| (no_ext(&source_file.source_file_path).as_ref(), *source_file)),
    );
    println!("source file map: {:#?}", source_file_map);
    // build fences map
    let fences_map: HashMap<&str, &Fence> = HashMap::from_iter(
        fences
            .iter()
            .map(|fence_file| (no_ext(&fence_file.fence_path).as_ref(), *fence_file)),
    );
    let fence_collection: FenceCollection = FenceCollection {
        fences_map: fences_map,
    };

    // analyze fences
    println!("Analyzing fences");
    let mut all_violations: Vec<ImportRuleViolation> = vec![];
    for source in sources {
        let violations_wrapped = evaluate_fences(
            &fence_collection,
            &source_file_map,
            &tsconfig_paths_json,
            source,
        );

        match violations_wrapped {
            Err(e) => println!("error! {}", e),
            Ok(None) => {}
            Ok(Some(violations)) => {
                println!(
                    "ERROR in file {:#?}: {:#?}",
                    source.source_file_path, violations
                );
                for violation in violations {
                    all_violations.push(violation);
                }
            }
        }
    }

    // print some junk
    println!("Violations: {:#?}", all_violations)
}
