use crate::fence::{DependencyRule, ExportRule, Fence};
use crate::fence_collection::FenceCollection;
use crate::import_resolver::{resolve_ts_import, ResolvedImport, TsconfigPathsJson};
use crate::walk_dirs::SourceFile;
use glob::Pattern;
use relative_path::RelativePath;
use std::collections::{HashMap, HashSet};
use std::iter::Iterator;
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
    violating_fence: &'fencelifetime Fence,
    violating_fence_clause: ViolatedFenceClause<'fencelifetime>,
    violating_import_path: &'importlifetime str,
    violating_imported_name: Option<&'importlifetime str>,
}

fn export_rule_applies_to_import_path(
    fence_path: &str,
    export_rule: &ExportRule,
    imported_file_path: &Path,
) -> Result<bool, glob::PatternError> {
    let mut buf = PathBuf::from(fence_path);
    buf.push(export_rule.modules.clone());
    let export_rule_glob = Pattern::new(buf.to_str().unwrap());

    match export_rule_glob {
        Ok(glob) => Ok(glob.matches(imported_file_path.to_str().unwrap())),
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
    source_files: &HashMap<&str, &SourceFile>,
    tsconfig_paths_json: &TsconfigPathsJson,
    source_file: &'sourcefilelifetime SourceFile,
) -> Result<Option<Vec<ImportRuleViolation<'fencecollectionlifetime, 'sourcefilelifetime>>>, String>
{
    let mut violations = Vec::<ImportRuleViolation>::new();
    let source_fences: Vec<&'fencecollectionlifetime Fence> =
        fence_collection.get_fences_for_path(&PathBuf::from(source_file.source_file_path.clone()));

    for (import_specifier, imported_names) in source_file.imports.imports.iter() {
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
                    if imported_source_file_opt.is_none() {
                        return Err(format!(
                            "could not find project local path {}",
                            project_local_path_str
                        ));
                    }

                    let imported_source_file = imported_source_file_opt.unwrap();

                    // check allowed imports against tags of the imported source file
                    for source_fence in source_fences.iter() {
                        if source_fence.fence.imports.is_some()
                            && (imported_source_file
                                .tags
                                .iter()
                                .any(|imported_source_file_tag| {
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
                                violating_fence: source_fence,
                                violating_fence_clause: ViolatedFenceClause::ImportAllowList,
                                violating_import_path: &import_specifier,
                                violating_imported_name: None,
                            })
                        }
                    }

                    let imported_file_path =
                        &PathBuf::from(imported_source_file.source_file_path.clone());
                    let imported_source_file_fences =
                        fence_collection.get_fences_for_path(imported_file_path);
                    // check imports against exports of each fence
                    for destination_fence in imported_source_file_fences.iter() {
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
                                    violating_fence: destination_fence,
                                    violating_fence_clause: ViolatedFenceClause::ExportRule(None),
                                    violating_import_path: &import_specifier,
                                    violating_imported_name: None,
                                })
                            }
                            for destination_export_rule in destination_export_rules {
                                // check that the rule allows exports to the tag of the file
                                if !is_importer_allowed(
                                    &destination_export_rule.accessible_to,
                                    source_file,
                                ) {
                                    // rule violation this importer is on the allow list but
                                    // not to this tag
                                    violations.push(ImportRuleViolation {
                                        violating_fence: destination_fence,
                                        violating_fence_clause: ViolatedFenceClause::ExportRule(
                                            Some(&destination_export_rule),
                                        ),
                                        violating_import_path: &import_specifier,
                                        violating_imported_name: None,
                                    })
                                }
                            }
                        }
                    }
                }
                // node imports: check the tags against the source fence allow list
                ResolvedImport::NodeModulesImport(node_module_name) => {
                    for source_fence in source_fences.iter() {
                        // only filter on dependencies if there is a dependency list
                        if source_fence.fence.dependencies.is_some() {
                            let allowed_dependencies: &'fencecollectionlifetime Vec<
                                DependencyRule,
                            > = source_fence.fence.dependencies.as_ref().unwrap();
                            let dependency_clauses: Vec<&'fencecollectionlifetime DependencyRule> =
                                allowed_dependencies
                                    .iter()
                                    // TODO handle glob dependency matches
                                    .filter(|dependency| dependency.dependency == node_module_name)
                                    .collect();
                            if dependency_clauses.len() == 0 {
                                // violation: dependency not on allowlist
                                violations.push(ImportRuleViolation {
                                    violating_fence: source_fence,
                                    violating_fence_clause: ViolatedFenceClause::DependencyRule(
                                        None,
                                    ),
                                    violating_import_path: &import_specifier,
                                    violating_imported_name: None,
                                })
                            } else {
                                for dependency_clause in dependency_clauses {
                                    if !is_importer_allowed(
                                        &dependency_clause.accessible_to,
                                        source_file,
                                    ) {
                                        // violation: dependency on allowlist, but not exposed
                                        // to tags for this file
                                        violations.push(ImportRuleViolation {
                                            violating_fence: source_fence,
                                            violating_fence_clause:
                                                ViolatedFenceClause::DependencyRule(Some(
                                                    &dependency_clause,
                                                )),
                                            violating_import_path: &import_specifier,
                                            violating_imported_name: None,
                                        })
                                    }
                                }
                            }
                        }
                    }
                }
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
