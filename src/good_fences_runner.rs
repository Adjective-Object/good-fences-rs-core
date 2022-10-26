extern crate serde_json;
use crate::evaluate_fences::{evaluate_fences, FenceEvaluationResult};
use crate::fence::Fence;
use crate::fence_collection::FenceCollection;
use crate::file_extension::no_ext;
use crate::import_resolver::TsconfigPathsJson;
use crate::walk_dirs::{discover_fences_and_files, ExternalFences, SourceFile, WalkFileData};
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter::{FromIterator, Iterator};

#[derive(Debug, PartialEq)]
pub struct GoodFencesRunner {
    tsconfig_paths_json: TsconfigPathsJson,
    fence_collection: FenceCollection,
    source_files: HashMap<String, SourceFile>,
}

#[derive(Debug, PartialEq)]
pub struct UndefinedTagReference<'a> {
    pub tag: &'a str,
    pub referencing_fence_path: &'a str,
}

impl GoodFencesRunner {
    pub fn new(
        tsconfig_paths_json: TsconfigPathsJson,
        directory_paths_to_walk: &Vec<&str>,
        external_fences: ExternalFences,
        ignored_dirs: &Vec<regex::Regex>,
    ) -> GoodFencesRunner {
        // find files
        let walked_files = directory_paths_to_walk
            .iter()
            .map(|path| discover_fences_and_files(path, external_fences, ignored_dirs.clone()))
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
        let fences_map: HashMap<String, Fence> =
            HashMap::from_iter(fences.into_iter().map(|fence_file| {
                let k = fence_file.fence_path.clone();
                (k, fence_file)
            }));
        return GoodFencesRunner {
            source_files: source_file_map,
            fence_collection: FenceCollection {
                fences_map: fences_map,
            },
            tsconfig_paths_json: tsconfig_paths_json,
        };
    }

    pub fn find_import_violations<'a>(&'a self) -> FenceEvaluationResult<'a, 'a> {
        println!("Evaluating {} files", self.source_files.keys().len());
        let mut evaluation_results = FenceEvaluationResult::new();

        let violation_results = self
            .source_files
            .par_iter()
            .map(|(_, source_file)| {
                evaluate_fences(
                    &self.fence_collection,
                    &self.source_files,
                    &self.tsconfig_paths_json,
                    &source_file,
                )
            })
            .collect::<Vec<_>>();
        for result in violation_results {
            for v in result.violations {
                evaluation_results.violations.push(v);
            }
            for eval_error in result.evaluation_errors {
                evaluation_results.evaluation_errors.push(eval_error);
            }
        }

        return evaluation_results;
    }

    /**
     * Finds tags that are referenced but not set in any fences
     */
    pub fn find_undefined_tags<'a>(&'a self) -> Vec<UndefinedTagReference<'a>> {
        let mut defined_tags = HashSet::<&'a str>::new();
        let mut referenced_tags = HashSet::<&'a str>::new();
        for (_, fence) in self.fence_collection.fences_map.iter() {
            // add encountered tags
            match fence.fence.tags.as_ref() {
                Some(tag_set) => {
                    for tag in tag_set {
                        defined_tags.insert(&tag);
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
                                referenced_tags.insert(tag);
                            }
                        }
                    }
                }
                // noop on nothing
                None => {}
            }
        }

        // it's probably cheaper on average to iterate over the fence vec twice, when
        // there are unreferenced tags, since we expect having undefined tags to be an
        // outlier, and maintaining the map between the consuming file paths and fence
        // items is overhead we don't want to deal with
        let undefined_tags_set: HashSet<&'a str> =
            HashSet::from_iter(referenced_tags.difference(&defined_tags).map(|x| *x));
        if undefined_tags_set.len() > 0 {
            let mut undefined_tag_references = Vec::<UndefinedTagReference>::new();

            for (_, fence) in self.fence_collection.fences_map.iter() {
                // add consumed tags
                match fence.fence.exports.as_ref() {
                    Some(exports) => {
                        for export in exports {
                            for tag in export.accessible_to.iter() {
                                let tag_as_str_ref: &'a str = tag;
                                if undefined_tags_set.contains(tag_as_str_ref) {
                                    undefined_tag_references.push(UndefinedTagReference {
                                        tag: tag,
                                        referencing_fence_path: fence.fence_path.as_ref(),
                                    })
                                }
                            }
                        }
                    }
                    // noop on nothing
                    None => {}
                }
            }

            debug_assert_eq!(undefined_tag_references.len(), undefined_tags_set.len());
            undefined_tag_references
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod test {
    extern crate text_diff;
    use crate::evaluate_fences::{ImportRuleViolation, ViolatedFenceClause};
    use crate::fence::{DependencyRule, ExportRule, Fence, ParsedFence};
    use crate::fence_collection::FenceCollection;
    use crate::good_fences_runner::{GoodFencesRunner, UndefinedTagReference};
    use crate::import_resolver::{TsconfigPathsCompilerOptions, TsconfigPathsJson};
    use crate::walk_dirs::{ExternalFences, SourceFile};
    use std::collections::{HashMap, HashSet};
    use std::iter::FromIterator;
    use text_diff::print_diff;

    macro_rules! map(
        { $($key:expr => $value:expr),+ } => {
            {
                let mut m = ::std::collections::HashMap::new();
                $(
                    m.insert(String::from($key), $value);
                )+
                m
            }
        };
    );

    macro_rules! set(
        { $($member:expr),+ } => {
            {
                HashSet::from_iter(vec!(
                    $(
                        String::from($member),
                    )+
                ))
            }
        };
    );

    #[test]
    fn good_fences_integration_test_runner_initialized() {
        let good_fences_runner = GoodFencesRunner::new(
            TsconfigPathsJson::from_path("tests/good_fences_integration/tsconfig.json".to_string())
                .unwrap(),
            &vec!["tests/good_fences_integration/src"],
            ExternalFences::Ignore,
            &Vec::new(),
        );

        assert_eq!(
            good_fences_runner,
            GoodFencesRunner {
                tsconfig_paths_json: TsconfigPathsJson {
                    compiler_options: TsconfigPathsCompilerOptions {
                        base_url: None,
                        paths: HashMap::new(),
                    },
                },
                fence_collection: FenceCollection {
                    fences_map: map!(
                        "tests/good_fences_integration/src/componentB/someDeep/complexComponentA/fence.json" => Fence {
                            fence_path: "tests/good_fences_integration/src/componentB/someDeep/complexComponentA/fence.json".to_owned(),
                            fence: ParsedFence {
                                tags: Some(
                                    vec!["tagB".to_owned()]
                                ),
                                exports: None,
                                dependencies: Some(
                                    vec![
                                        DependencyRule {
                                            dependency: "fs".to_owned(),
                                            accessible_to: vec!["*".to_owned()]
                                        }
                                    ]
                                ),
                                imports: None
                            }
                        },
                        "tests/good_fences_integration/src/componentB/someDeep/componentA/fence.json" => Fence {
                            fence_path:"tests/good_fences_integration/src/componentB/someDeep/componentA/fence.json".to_owned(),
                            fence: ParsedFence {
                                tags: Some(
                                    vec!["tagB".to_owned()]
                                ),
                                exports: None,
                                dependencies: Some(
                                    vec![
                                        DependencyRule {
                                            dependency: "fs".to_owned(),
                                            accessible_to: vec!["*".to_owned()]
                                        }
                                    ]
                                ),
                                imports: None
                            }
                        },

                        "tests/good_fences_integration/src/componentC/fence.json" => Fence {
                            fence_path: "tests/good_fences_integration/src/componentC/fence.json".to_owned(),
                            fence: ParsedFence {
                                tags: Some(vec!["tagC".to_owned()]),
                                exports: Some(
                                    vec![
                                        ExportRule {
                                            accessible_to:vec!["tagA".to_owned()],
                                            modules: "helperC1".to_owned()
                                        }
                                    ]
                                ),
                                dependencies: None,
                                imports: None
                            }
                        },
                        "tests/good_fences_integration/src/componentA/fence.json" => Fence {
                            fence_path: "tests/good_fences_integration/src/componentA/fence.json".to_owned(),
                            fence: ParsedFence {
                                tags: Some(
                                    vec![
                                        "tagA".to_owned()
                                    ],
                                ),
                                exports: Some(
                                    vec![
                                        ExportRule {
                                            accessible_to: vec!(
                                                "*".to_owned()
                                            ),
                                            modules: "componentA".to_owned(),
                                        },
                                        ExportRule {
                                            accessible_to: vec!(
                                                "unknownTag".to_owned()
                                            ),
                                            modules: "helperA1".to_owned(),
                                        }
                                    ],
                                ),
                                dependencies: None,
                                imports: Some(
                                    vec![],
                                ),
                            },
                        },
                        "tests/good_fences_integration/src/componentB/fence.json" => Fence {
                            fence_path: "tests/good_fences_integration/src/componentB/fence.json".to_owned(),
                            fence: ParsedFence {
                                tags: Some(
                                    vec![
                                        "tagB".to_owned(),
                                 ],
                                ),
                                exports: Some(
                                    vec![
                                        ExportRule {
                                            accessible_to: vec!(
                                                "tagA".to_owned()
                                            ),
                                            modules: "componentB".to_owned(),
                                        },
                                    ],
                                ),
                                dependencies: None,
                                imports: None,
                            },
                        }
                    ),
                },
                source_files: map!(
                    "tests/good_fences_integration/src/componentB/someDeep/complexComponentA/index" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentB/someDeep/complexComponentA/index.ts".to_owned(),
                        tags: set!("tagB".to_owned()),
                        imports: map!(
                            "../../../componentC/helperC1" => Some(set!("default".to_owned()))
                        )
                    },
                    "tests/good_fences_integration/src/componentB/someDeep/componentA/index" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentB/someDeep/componentA/index.ts".to_owned(),
                        tags: set!("tagB".to_owned()),
                        imports: map!(
                            "../../../componentC/helperC1" => Some(set!("default".to_owned()))
                        )
                    },
                    "tests/good_fences_integration/src/componentC/helperC1" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentC/helperC1.ts".to_owned(),
                        tags: set!("tagC".to_owned()),
                        imports: HashMap::new(),
                    },
                    "tests/good_fences_integration/src/requireImportTest" => SourceFile {
                        source_file_path:"tests/good_fences_integration/src/requireImportTest.ts".to_owned(),
                        tags: HashSet::new(),
                        imports: map!(
                            "something" => None,
                            "fs" => None
                        )
                    },
                    "tests/good_fences_integration/src/componentA/helperA1" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentA/helperA1.ts".to_owned(),
                        tags: set!(
                            "tagA".to_owned()
                        ),
                        imports: map!(
                            "../componentB/helperB1" => Some(
                                set!(
                                    "default".to_owned()
                                ),
                            )
                        ),
                    },
                    "tests/good_fences_integration/src/componentB/componentB" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentB/componentB.ts".to_owned(),
                        tags: set!(
                            "tagB".to_owned()
                        ),
                        imports: map!(
                            "./helperB2" => Some(
                                set!(
                                    "default".to_owned()
                                ),
                            ),
                            "./helperB1" => Some(
                                set!(
                                    "default".to_owned()
                                ),
                            )
                        ),
                    },
                    "tests/good_fences_integration/src/componentB/helperB2" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentB/helperB2.ts".to_owned(),
                        tags: set!(
                            "tagB".to_owned()
                        ),
                        imports: HashMap::new(),
                    },
                    "tests/good_fences_integration/src/componentA/helperA2" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentA/helperA2.ts".to_owned(),
                        tags: set!(
                            "tagA".to_owned()
                        ),
                        imports: HashMap::new(),
                    },
                    "tests/good_fences_integration/src/index" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/index.ts".to_owned(),
                        tags: HashSet::new(),
                        imports: map!(
                                "./componentA/componentA" => Some(
                                    set!(
                                        "default".to_owned()
                                    ),
                                ),
                                "./componentB/componentB" => Some(
                                    set!(
                                        "default".to_owned()
                                    ),
                                )
                            ),
                    },
                    "tests/good_fences_integration/src/componentB/helperB1" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentB/helperB1.ts".to_owned(),
                        tags: set!(
                            "tagB".to_owned()
                        ),
                        imports: HashMap::new()
                    },
                    "tests/good_fences_integration/src/componentA/componentA" => SourceFile {
                        source_file_path: "tests/good_fences_integration/src/componentA/componentA.ts".to_owned(),
                        tags: set!(
                            "tagA".to_owned()
                        ),
                        imports: map!(
                                "./helperA2" => Some(
                                    set!(
                                        "default".to_owned()
                                    ),
                                ),
                                "./helperA1" => Some(
                                    set!(
                                        "default".to_owned(),
                                        "some".to_owned(),
                                        "other".to_owned(),
                                        "stuff".to_owned()
                                    ),
                                ),
                                "../componentB/componentB" => Some(
                                    set!(
                                        "default".to_owned()
                                    ),
                                )
                            ),
                    }
                )
            }
        );
    }

    fn compare_violations(a: &ImportRuleViolation, b: &ImportRuleViolation) -> std::cmp::Ordering {
        a.violating_file_path
            .cmp(b.violating_file_path)
            .then(
                a.violating_fence
                    .fence_path
                    .cmp(&b.violating_fence.fence_path),
            )
            .then(
                a.violating_import_specifier
                    .cmp(&b.violating_import_specifier),
            )
    }

    #[test]
    fn good_fences_integration_test_violations() {
        let good_fences_runner = GoodFencesRunner::new(
            TsconfigPathsJson::from_path("tests/good_fences_integration/tsconfig.json".to_string())
                .unwrap(),
            &vec!["tests/good_fences_integration"],
            ExternalFences::Ignore,
            &Vec::new(),
        );

        let mut results = good_fences_runner.find_import_violations();
        results.violations.sort_by(compare_violations);

        let rule = ExportRule {
            accessible_to: vec!["tagA".to_owned()],
            modules: "componentB".to_owned(),
        };

        let rule_complex = ExportRule {
            accessible_to: vec!["tagA".to_owned()],
            modules: "helperC1".to_owned(),
        };
        let mut expected_violations = vec![
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/componentB/someDeep/componentA/index.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentC/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Some(&rule_complex)),
                violating_import_specifier: "../../../componentC/helperC1",
                violating_imported_name: None,
            },
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/componentB/someDeep/complexComponentA/index.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentC/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Some(&rule_complex)),
                violating_import_specifier: "../../../componentC/helperC1",
                violating_imported_name: None,
            },
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/componentA/helperA1.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentB/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(None),
                violating_import_specifier: "../componentB/helperB1",
                violating_imported_name: None,
            },
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/componentA/helperA1.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentA/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                violating_import_specifier: "../componentB/helperB1",
                violating_imported_name: None,
            },
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/componentA/componentA.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentA/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                violating_import_specifier: "../componentB/componentB",
                violating_imported_name: None,
            },
            ImportRuleViolation {
                violating_file_path: "tests/good_fences_integration/src/index.ts",
                violating_fence: good_fences_runner
                    .fence_collection
                    .fences_map
                    .get("tests/good_fences_integration/src/componentB/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Some(&rule)),
                violating_import_specifier: "./componentB/componentB",
                violating_imported_name: None,
            },
        ];
        expected_violations.sort_by(compare_violations);

        let a: String = format!("{:#?}", results);
        let b: String = format!("{:#?}", expected_violations);
        if results.violations != expected_violations {
            print_diff(&a, &b, "\n");
            assert!(false);
        }
    }

    #[test]
    fn good_fences_integration_test_find_undefined_tags() {
        let good_fences_runner = GoodFencesRunner::new(
            TsconfigPathsJson::from_path("tests/good_fences_integration/tsconfig.json".to_string())
                .unwrap(),
            &vec!["tests/good_fences_integration/src"],
            ExternalFences::Ignore,
            &Vec::new(),
        );

        let orphans = good_fences_runner.find_undefined_tags();

        // print some junk
        assert_eq!(
            orphans,
            vec!(UndefinedTagReference {
                tag: "unknownTag",
                referencing_fence_path: "tests/good_fences_integration/src/componentA/fence.json"
            })
        );
    }
}
