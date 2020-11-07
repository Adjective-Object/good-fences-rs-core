extern crate serde_json;
use crate::evaluate_fences::{evaluate_fences, ImportRuleViolation};
use crate::fence::Fence;
use crate::fence_collection::FenceCollection;
use crate::file_extension::no_ext;
use crate::import_resolver::TsconfigPathsJson;
use crate::walk_dirs::{discover_fences_and_files, SourceFile, WalkFileData};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::iter::{FromIterator, Iterator};

pub struct GoodFencesRunner {
    tsconfig_paths_json: TsconfigPathsJson,
    fence_collection: FenceCollection,
    source_files: HashMap<String, SourceFile>,
}

impl GoodFencesRunner {
    pub fn new(
        tsconfig_paths_json_path: &str,
        directory_paths_to_walk: &Vec<&str>,
    ) -> GoodFencesRunner {
        // load tsconfig.json
        let file = File::open(tsconfig_paths_json_path).unwrap();
        let buf_reader = BufReader::new(file);
        let tsconfig_paths_json: TsconfigPathsJson = serde_json::from_reader(buf_reader).unwrap();

        // find files
        let walked_files = directory_paths_to_walk
            .iter()
            .map(|path| discover_fences_and_files(path))
            .flatten();

        let (fences_wrapped, sources_wrapped): (Vec<WalkFileData>, Vec<WalkFileData>) =
            // filter to Fences and Source FIles
            walked_files.filter(|x| match x {
                WalkFileData::Fence(_fence) => true,
                WalkFileData::SourceFile(_source_file) => true,
                _ => false
            }).partition(|file| match file {
                // partition to 2 arrays
                WalkFileData::Fence(_fence) => true,
                WalkFileData::SourceFile(_source_file) => false,
                _ => false,
            });
        let fences: Vec<Fence> = fences_wrapped
            .into_iter()
            .map(|x| match x {
                WalkFileData::Fence(fence) => fence,
                a => panic!("found non-Fence {:?} in Fence partition", a),
            })
            .collect();
        let sources: Vec<SourceFile> = sources_wrapped
            .into_iter()
            .map(|x| match x {
                WalkFileData::SourceFile(fence) => fence,
                a => panic!("found non-SourceFile {:?} in SourceFile partition", a),
            })
            .collect();

        // build sources map
        let source_file_map: HashMap<String, SourceFile> =
            HashMap::from_iter(sources.into_iter().map(|source_file| {
                (
                    no_ext(&source_file.source_file_path).to_owned(),
                    source_file,
                )
            }));
        // println!("source file map: {:#?}", source_file_map);
        // build fences map
        let fences_map: HashMap<String, Fence> = HashMap::from_iter(
            fences
                .into_iter()
                .map(|fence_file| (no_ext(&fence_file.fence_path).to_owned(), fence_file)),
        );
        return GoodFencesRunner {
            source_files: source_file_map,
            fence_collection: FenceCollection {
                fences_map: fences_map,
            },
            tsconfig_paths_json: tsconfig_paths_json,
        };
    }

    pub fn evaluate_fences<'a>(&'a self) -> Vec<ImportRuleViolation<'a, 'a>> {
        let mut all_violations: Vec<ImportRuleViolation<'a, 'a>> = vec![];
        for (_, source) in self.source_files.iter() {
            let violations_wrapped = evaluate_fences(
                &self.fence_collection,
                &self.source_files,
                &self.tsconfig_paths_json,
                &source,
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

        return all_violations;
    }

    /**
     * Finds tags that are referenced but not set in any fences
     */
    pub fn find_orphaned_tags<'a>(&'a self) -> Vec<&'a str> {
        let mut encountered_tags = HashSet::<&'a str>::new();
        let mut consumed_tags = HashSet::<&'a str>::new();
        for (_, fence) in self.fence_collection.fences_map.iter() {
            // add encountered tags
            match fence.fence.tags.as_ref() {
                Some(tag_set) => {
                    for tag in tag_set {
                        encountered_tags.insert(&tag);
                    }
                }
                // noop on nothing
                None => {}
            }
            // add consumed tags
            match fence.fence.exports.as_ref() {
                Some(exports) => {
                    for export in exports {
                        for tag in export.accessible_to.iter() {
                            if tag != "*" {
                                consumed_tags.insert(tag);
                            }
                        }
                    }
                }
                // noop on nothing
                None => {}
            }
        }

        Vec::from_iter(encountered_tags.difference(&consumed_tags).map(|x| *x))
    }
}
