use crate::fence::{DependencyRule, ExportRule, Fence};
use crate::fence_collection::FenceCollection;
use crate::file_extension::no_ext;
use crate::import_resolver::{resolve_ts_import, ResolvedImport, TsconfigPathsJson};
use crate::walk_dirs::SourceFile;
use glob::Pattern;
use relative_path::RelativePath;
use std::collections::{HashMap, HashSet};
use std::iter::{FromIterator, Iterator};
use std::path::{Path, PathBuf};
use std::vec::Vec;

#[derive(Debug, PartialEq, Eq)]
pub enum ViolatedFenceClause<'a> {
    ExportRule(Option<&'a ExportRule>),
    DependencyRule(Option<&'a DependencyRule>),
    ImportAllowList,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImportRuleViolation<'fencelifetime, 'importlifetime> {
    pub violating_file_path: &'importlifetime str,
    pub violating_fence: &'fencelifetime Fence,
    pub violating_fence_clause: ViolatedFenceClause<'fencelifetime>,
    pub violating_import_specifier: &'importlifetime str,
    pub violating_imported_name: Option<&'importlifetime str>,
}

fn is_node_dependency_matching(
    permitted_node_dependency_pattern: &str,
    node_dependency: &str,
) -> bool {
    let export_rule_glob = Pattern::new(permitted_node_dependency_pattern);

    match export_rule_glob {
        Ok(glob) => glob.matches(node_dependency),
        Err(_e) => false,
    }
}

fn export_rule_applies_to_import_path(
    fence_path: &str,
    export_rule: &ExportRule,
    imported_file_path: &Path,
) -> Result<bool, glob::PatternError> {
    let mut buf = PathBuf::from(fence_path);
    buf.pop();
    buf.push(export_rule.modules.clone());
    let export_rule_glob = Pattern::new(buf.to_str().unwrap());

    match export_rule_glob {
        Ok(glob) => Ok(glob.matches(imported_file_path.to_str().unwrap())
            || glob.matches(no_ext(imported_file_path.to_str().unwrap()))),
        Err(e) => Err(e),
    }
}

fn is_importer_allowed(accessible_to: &Vec<String>, source_file: &SourceFile) -> bool {
    return accessible_to.iter().any(|accessible_to_tag| {
        accessible_to_tag == "*" || source_file.tags.contains(accessible_to_tag)
    });
}

pub fn evaluate_fences<'fencecollectionlifetime, 'sourcefilelifetime>(
    fence_collection: &'fencecollectionlifetime FenceCollection,
    source_files: &HashMap<String, SourceFile>,
    tsconfig_paths_json: &TsconfigPathsJson,
    source_file: &'sourcefilelifetime SourceFile,
) -> Result<Option<Vec<ImportRuleViolation<'fencecollectionlifetime, 'sourcefilelifetime>>>, String>
{
    let mut violations = Vec::<ImportRuleViolation>::new();
    let source_fences: Vec<&'fencecollectionlifetime Fence> =
        fence_collection.get_fences_for_path(&PathBuf::from(source_file.source_file_path.clone()));

    // fences only apply to files between their boundaries, so
    // fences will not filter imports within their bounds at all.
    //
    // the same goes for exported files
    let source_fences_set: HashSet<&Fence> = HashSet::from_iter(source_fences);

    for (import_specifier, _imported_names) in source_file.imports.imports.iter() {
        let resolved_import = resolve_ts_import(
            tsconfig_paths_json,
            &RelativePath::new(&source_file.source_file_path),
            &import_specifier,
        );

        match resolved_import {
            Ok(resolved_import) => match resolved_import {
                // grab the project local file, check our tags against the exports of the
                // fences of the file we are importing.
                ResolvedImport::ProjectLocalImport(project_local_path) => {
                    let project_local_path_str = project_local_path.to_str().unwrap();
                    let imported_source_file_opt = source_files.get(project_local_path_str);
                    let imported_source_file_with_idx_opt = if imported_source_file_opt.is_none() {
                        let mut clone_path_with_idx = project_local_path.clone();
                        clone_path_with_idx.push("index");
                        let clone_path_with_idx_str = clone_path_with_idx.to_str().unwrap();

                        source_files.get(clone_path_with_idx_str)
                    } else {
                        None
                    };

                    let imported_source_file = match imported_source_file_opt {
                        None => match imported_source_file_with_idx_opt {
                            Some(x) => x,
                            None => {
                                return Err(format!(
                                    "could not find project local path {} imported by {} with specifier {}",
                                    project_local_path_str, source_file.source_file_path, import_specifier
                                ));
                            }
                        },
                        Some(x) => x,
                    };

                    let imported_file_path =
                        &PathBuf::from(imported_source_file.source_file_path.clone());
                    let imported_source_file_fences: Vec<&Fence> =
                        fence_collection.get_fences_for_path(imported_file_path);
                    let imported_source_file_fences_set: HashSet<&Fence> =
                        HashSet::from_iter(imported_source_file_fences);

                    let exclusive_source_fences: HashSet<&Fence> = source_fences_set
                        .difference(&imported_source_file_fences_set)
                        .map(|x| *x)
                        .collect();
                    let exclusive_target_fences: HashSet<&Fence> = imported_source_file_fences_set
                        .difference(&source_fences_set)
                        .map(|x| *x)
                        .collect();

                    // check allowed imports against tags of the imported source file
                    for source_fence in exclusive_source_fences.iter() {
                        if source_fence.fence.imports.is_some()
                            && (imported_source_file
                                .tags
                                .iter()
                                .all(|imported_source_file_tag| {
                                    !source_fence
                                        .fence
                                        .imports
                                        .as_ref()
                                        .unwrap()
                                        .contains(imported_source_file_tag)
                                }))
                        {
                            // our source fences do not allow consuming this tag
                            violations.push(ImportRuleViolation {
                                violating_file_path: &source_file.source_file_path,
                                violating_fence: source_fence,
                                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                                violating_import_specifier: &import_specifier,
                                violating_imported_name: None,
                            })
                        }
                    }

                    // check imports against exports of each fence
                    for destination_fence in exclusive_target_fences.iter() {
                        if destination_fence.fence.exports.is_some() {
                            let export_rules_unfiltered =
                                destination_fence.fence.exports.as_ref().unwrap();
                            let destination_export_rules: Vec<&ExportRule> =
                                export_rules_unfiltered
                                    .iter()
                                    .filter(|export_rule| {
                                        export_rule_applies_to_import_path(
                                            &destination_fence.fence_path,
                                            export_rule,
                                            imported_file_path,
                                        )
                                        .unwrap()
                                    })
                                    .collect();
                            if destination_export_rules.len() == 0 {
                                // rule violation: this importer is not on the allow list
                                violations.push(ImportRuleViolation {
                                    violating_file_path: &source_file.source_file_path,
                                    violating_fence: destination_fence,
                                    violating_fence_clause: ViolatedFenceClause::ExportRule(None),
                                    violating_import_specifier: &import_specifier,
                                    violating_imported_name: None,
                                })
                            }
                            let any_destination_export_rule_allows_import =
                                destination_export_rules.iter().any(|clause| {
                                    is_importer_allowed(&clause.accessible_to, source_file)
                                });

                            if !any_destination_export_rule_allows_import {
                                // check that the rule allows exports to the tag of the file
                                for destination_export_rule in destination_export_rules {
                                    // rule violation this importer is on the allow list but
                                    // not to this tag
                                    violations.push(ImportRuleViolation {
                                        violating_file_path: &source_file.source_file_path,
                                        violating_fence: destination_fence,
                                        violating_fence_clause: ViolatedFenceClause::ExportRule(
                                            Some(&destination_export_rule),
                                        ),
                                        violating_import_specifier: &import_specifier,
                                        violating_imported_name: None,
                                    })
                                }
                            }
                        }
                    }
                }
                // node imports: check the tags against the source fence allow list
                ResolvedImport::NodeModulesImport(node_module_filter) => {
                    for source_fence in source_fences_set.iter() {
                        // only filter on dependencies if there is a dependency list
                        if source_fence.fence.dependencies.is_some() {
                            let allowed_dependencies: &'fencecollectionlifetime Vec<
                                DependencyRule,
                            > = source_fence.fence.dependencies.as_ref().unwrap();
                            let matching_dependency_clauses: Vec<
                                &'fencecollectionlifetime DependencyRule,
                            > = allowed_dependencies
                                .iter()
                                // TODO handle glob dependency matches
                                .filter(|dependency| {
                                    is_node_dependency_matching(
                                        &dependency.dependency,
                                        &node_module_filter,
                                    )
                                })
                                .collect();
                            if matching_dependency_clauses.len() == 0 {
                                // violation: dependency not on allowlist
                                violations.push(ImportRuleViolation {
                                    violating_file_path: &source_file.source_file_path,
                                    violating_fence: source_fence,
                                    violating_fence_clause: ViolatedFenceClause::DependencyRule(
                                        None,
                                    ),
                                    violating_import_specifier: &import_specifier,
                                    violating_imported_name: None,
                                })
                            } else {
                                // if any of the applicable clauses allow the import, allow it.
                                let any_matching_dependency_clauses_allows_import =
                                    matching_dependency_clauses.iter().any(|clause| {
                                        is_importer_allowed(&clause.accessible_to, source_file)
                                    });
                                if !any_matching_dependency_clauses_allows_import {
                                    // none of the applicable clauses allow this import
                                    for dependency_clause in matching_dependency_clauses {
                                        {
                                            // violation: dependency on allowlist, but not exposed
                                            // to tags for this file
                                            violations.push(ImportRuleViolation {
                                                violating_file_path: &source_file.source_file_path,
                                                violating_fence: source_fence,
                                                violating_fence_clause:
                                                    ViolatedFenceClause::DependencyRule(Some(
                                                        &dependency_clause,
                                                    )),
                                                violating_import_specifier: &import_specifier,
                                                violating_imported_name: None,
                                            })
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // do nothing for resource file imports
                ResolvedImport::ResourceFileImport => {}
            },
            Err(e) => {
                return Err(e);
            }
        }
    }

    return if violations.len() > 0 {
        Ok(Some(violations))
    } else {
        Ok(None)
    };
}

#[cfg(test)]
mod test {
    use crate::evaluate_fences::{evaluate_fences, ImportRuleViolation, ViolatedFenceClause};
    use crate::fence::{parse_fence_str, DependencyRule, ExportRule};
    use crate::fence_collection::FenceCollection;
    use crate::import_resolver::{TsconfigPathsCompilerOptions, TsconfigPathsJson};
    use crate::walk_dirs::SourceFile;
    use find_ts_imports::SourceFileImportData;
    use lazy_static::lazy_static;
    use relative_path::RelativePathBuf;
    use std::collections::{HashMap, HashSet};
    use std::iter::FromIterator;

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

    lazy_static! {
        static ref SOURCE_FILES: HashMap<String, SourceFile> = map!(
            "path/to/source/index" => SourceFile {
                tags: HashSet::new(),
                source_file_path: "path/to/source/index.ts".to_owned(),
                imports: SourceFileImportData {
                    imports: map!(
                        "../protected/internal" => Option::None,
                        "node-import" => Option::None
                    ),
                }
            },
            "path/to/source/friend/index" => SourceFile {
                tags: set!(
                    "friend"
                ),
                source_file_path: "path/to/source/friend/index.ts".to_owned(),
                imports: SourceFileImportData {
                    imports: map!(
                        "../../protected/internal" => Option::None,
                        "node-import" => Option::None
                    ),
                }
            },
            "path/to/protected/internal" => SourceFile {
                tags: set!(
                    "protected"
                ),
                source_file_path: "path/to/protected/internal.ts".to_owned(),
                imports: SourceFileImportData {
                    imports: HashMap::new(),
                }
            }
        );
    }

    lazy_static! {
        static ref TSCONFIG_PATHS_JSON: TsconfigPathsJson = TsconfigPathsJson {
            compiler_options: TsconfigPathsCompilerOptions {
                paths: HashMap::new(),
                base_url: Option::None,
            },
        };
    }

    #[test]
    pub fn test_imports_allow_list_empty_violation() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{"imports": []}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/source/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_allow_list_mismatch_violation() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{"imports": ["some_tag"]}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/source/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_exports_list_empty() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": []}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/protected/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Option::None),
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_exports_list_mismatch() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": ["protected-exposed"]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/protected/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Option::None),
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_exports_list_conflicting_match_allowed() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/source/friend/fence.json" => parse_fence_str(
                    r#"{
                        "tags": ["friend"]
                    }"#,
                    &RelativePathBuf::from("path/to/source/friend/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": [{"modules": "*", "accessibleTo": "test"}, {"modules": "*", "accessibleTo": "friend"}]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/friend/index").unwrap(),
        );

        assert_eq!(violations, Ok(None));
    }

    #[test]
    pub fn test_imports_exports_list_not_on_allow_list() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": [{
                         "modules": "internal.ts",
                         "accessibleTo": [
                             "nothing"
                         ]
                    }]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        let d = ExportRule {
            modules: "internal.ts".to_owned(),
            accessible_to: vec!["nothing".to_owned()],
        };

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/protected/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Some(&d)),
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_exports_list_not_on_allow_list_glob() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": [{
                         "modules": "*.ts",
                         "accessibleTo": [
                             "nothing"
                         ]
                    }]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        let d = ExportRule {
            modules: "*.ts".to_owned(),
            accessible_to: vec!["nothing".to_owned()],
        };

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/protected/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::ExportRule(Some(&d)),
                violating_import_specifier: "../protected/internal",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_imports_exports_list_on_allow_list_glob() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/source/friend/fence.json" => parse_fence_str(
                    r#"{
                        "tags": ["friend"]
                    }"#,
                    &RelativePathBuf::from("path/to/source/friend/fence.json")
                ).unwrap(),
                "path/to/protected/fence.json" => parse_fence_str(
                    r#"{"tags": ["protected"], "exports": [{
                         "modules": "*.ts",
                         "accessibleTo": [
                             "friend"
                         ]
                    }]}"#,
                    &RelativePathBuf::from("path/to/protected/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/friend/index").unwrap(),
        );

        assert_eq!(violations, Ok(None));
    }

    #[test]
    pub fn test_dependencies_not_allowed_empty_arr() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{"dependencies": []}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/source/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::DependencyRule(None),
                violating_import_specifier: "node-import",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_dependencies_allowed_on_allow_list() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{"dependencies": ["node-import"]}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        assert_eq!(violations, Ok(None));
    }

    #[test]
    pub fn test_dependencies_not_allowed_when_not_accessible_to() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{"dependencies": [
                        {
                            "dependency": "node-import",
                            "accessibleTo": "some-tag"
                        }
                    ]}"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/index").unwrap(),
        );

        let d = DependencyRule {
            dependency: "node-import".to_owned(),
            accessible_to: vec!["some-tag".to_owned()],
        };

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/source/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::DependencyRule(Some(&d)),
                violating_import_specifier: "node-import",
                violating_imported_name: Option::None
            }]))
        );
    }

    #[test]
    pub fn test_dependencies_allowed_when_on_dependency_allow_list() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{
                        "dependencies": [
                            {
                                "dependency": "node-import",
                                "accessibleTo": "friend"
                            }
                        ]
                    }"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/source/friend/fence.json" => parse_fence_str(
                    r#"{
                        "tags": ["friend"]
                    }"#,
                    &RelativePathBuf::from("path/to/source/friend/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/friend/index").unwrap(),
        );

        assert_eq!(violations, Ok(None));
    }

    #[test]
    pub fn test_dependencies_not_allowed_when_on_dependency_not_on_allow_list() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{
                        "dependencies": [
                            {
                                "dependency": "node-import",
                                "accessibleTo": "friendzzz"
                            }
                        ]
                    }"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/source/friend/fence.json" => parse_fence_str(
                    r#"{
                        "tags": ["friend"]
                    }"#,
                    &RelativePathBuf::from("path/to/source/friend/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/friend/index").unwrap(),
        );

        let r = DependencyRule {
            dependency: "node-import".to_owned(),
            accessible_to: vec!["friendzzz".to_owned()],
        };

        assert_eq!(
            violations,
            Ok(Some(vec![ImportRuleViolation {
                violating_file_path: "path/to/source/friend/index.ts",
                violating_fence: fence_collection
                    .fences_map
                    .get("path/to/source/fence.json")
                    .unwrap(),
                violating_fence_clause: ViolatedFenceClause::DependencyRule(Some(&r)),
                violating_import_specifier: "node-import",
                violating_imported_name: None
            }]))
        );
    }

    #[test]
    pub fn test_dependencies_allowed_when_on_dependency_allow_list_with_accessible_to_conflict() {
        let fence_collection = FenceCollection {
            fences_map: map!(
                "path/to/source/fence.json" => parse_fence_str(
                    r#"{
                        "dependencies": [
                            {
                                "dependency": "node-import",
                                "accessibleTo": "friend"
                            },
                            {
                                "dependency": "node-import",
                                "accessibleTo": "friendzzz"
                            }
                        ]
                    }"#,
                    &RelativePathBuf::from("path/to/source/fence.json")
                ).unwrap(),
                "path/to/source/friend/fence.json" => parse_fence_str(
                    r#"{
                        "tags": ["friend"]
                    }"#,
                    &RelativePathBuf::from("path/to/source/friend/fence.json")
                ).unwrap()
            ),
        };

        let violations = evaluate_fences(
            &fence_collection,
            &SOURCE_FILES,
            &TSCONFIG_PATHS_JSON,
            SOURCE_FILES.get("path/to/source/friend/index").unwrap(),
        );

        assert_eq!(violations, Ok(None));
    }
}
