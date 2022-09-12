extern crate relative_path;
extern crate serde;
use path_clean::PathClean as _;
use relative_path::{RelativePath, RelativePathBuf};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::string::String;
use std::vec::Vec;
use swc_common::FileName;
use swc_ecma_loader::resolve::Resolve;
use swc_ecma_loader::resolvers::node::NodeModulesResolver;

use crate::path_utils::{as_slashed_pathbuf, slashed_as_relative_path};

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsJson {
    pub compiler_options: TsconfigPathsCompilerOptions,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsCompilerOptions {
    pub base_url: Option<String>,
    pub paths: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum ResolvedImport {
    NodeModulesImport(String),
    ProjectLocalImport(PathBuf),
    ResourceFileImport,
}

pub fn resolve_import<'a>(
    tsconfig_paths: &'a TsconfigPathsJson,
    initial_path: &RelativePath,
    raw_import_path: &'a str,
) -> Result<ResolvedImport, String> {
    let resolver = NodeModulesResolver::default();
    let base = FileName::Real(PathBuf::from(initial_path.as_str()));

    let resolved = resolver.resolve(&base, raw_import_path);

    let file_path = match resolved {
        Ok(filename) => {
            if let FileName::Real(f) = filename {
                f
            } else {
                return Err("".to_string());
            }
        }
        Err(e) => return Err(e.to_string()),
    };

    if is_resource_file(&file_path) {
        return Ok(ResolvedImport::ResourceFileImport);
    }
    if raw_import_path.starts_with(".") {
        return Ok(ResolvedImport::ProjectLocalImport(file_path));
    }

    for segment in file_path.ancestors() {
        let stub_to_check_option = segment.to_str();

        if !stub_to_check_option.is_some() {
            return Err("accumulated specifier was empty.".to_owned());
        }
        let stub_to_check = stub_to_check_option.unwrap();
        if let Some(no_star_stub_entry) = tsconfig_paths.compiler_options.paths.get(stub_to_check) {
            if no_star_stub_entry.len() != 1 {
                return Err(format!(
                    "Expected all members of paths: to have a single entry, but got {:?}",
                    no_star_stub_entry
                ));
            }
            return Ok(ResolvedImport::ProjectLocalImport(path_buf_from_tsconfig(
                tsconfig_paths,
                &no_star_stub_entry[0],
            )));
        }
        let mut star_stub_to_check = stub_to_check.to_owned();
        star_stub_to_check.push_str("/*");
        if let Some(star_stub_entry) = tsconfig_paths
            .compiler_options
            .paths
            .get(&star_stub_to_check)
        {
            if star_stub_entry.len() != 1 {
                return Err(format!(
                    "Expected all members of paths: to have a single entry, but got {:?}",
                    star_stub_entry,
                ));
            }
            return Ok(ResolvedImport::ProjectLocalImport(path_buf_from_tsconfig(
                tsconfig_paths,
                &switch_specifier_prefix(
                    &star_stub_to_check,
                    &star_stub_entry[0],
                    &file_path.to_str().unwrap(),
                ),
            )));
        }
    }

    return Ok(ResolvedImport::NodeModulesImport(
        raw_import_path.to_string(),
    ));
}

fn is_resource_file(file: &PathBuf) -> bool {
    if let Some(ext) = file.extension() {
        if ext != ".tsx" && ext != ".ts" {
            return true;
        }
    }
    false
}

pub fn resolve_ts_import<'a>(
    tsconfig_paths: &'a TsconfigPathsJson,
    initial_path: &RelativePath,
    raw_import_specifier: &'a str,
) -> Result<ResolvedImport, String> {
    // println!("resole import! {:?}, {:?}", initial_path, import_specifier);

    // this is a directory import, so we want to add index.ts to the end of the file
    let import_specifier: String =
        if raw_import_specifier.ends_with('/') || raw_import_specifier.ends_with('.') {
            let mut x = RelativePathBuf::from(raw_import_specifier);
            x.push("index");
            x.normalize();
            x.to_string()
        } else {
            raw_import_specifier.to_owned()
        };

    // short circuit when importing non-ts resource files.
    let buf = PathBuf::from(import_specifier.clone());
    let ext = buf.extension();
    match ext {
        Some(ext) => {
            if ext != "tsx" && ext != "ts" {
                return Ok(ResolvedImport::ResourceFileImport);
            }
        }
        None => {}
    }

    if import_specifier.starts_with(".") {
        // relative import -- bypass tsconfig
        let parent_path = initial_path.parent();
        if !parent_path.is_some() {
            return Err(format!("source path {:} had no parent?", initial_path));
        }
        let joined_path: RelativePathBuf = parent_path
            .unwrap()
            .join(RelativePath::new(&import_specifier));
        return Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
            joined_path.normalize().as_str(),
        )));
    } else {
        // tsconfig.paths.json imports
        let import_specifier_path = Path::new(&import_specifier);
        for segment in import_specifier_path.ancestors() {
            // match on starless stub
            let stub_to_check_option = segment.to_str();
            if !stub_to_check_option.is_some() {
                return Err("accumulated specifier was empty.".to_owned());
            }
            let stub_to_check = stub_to_check_option.unwrap();
            let no_star_stub_option = tsconfig_paths.compiler_options.paths.get(stub_to_check);
            if no_star_stub_option.is_some() {
                let no_star_stub_entry = no_star_stub_option.unwrap();
                if no_star_stub_entry.len() != 1 {
                    return Err(format!(
                        "Expected all members of paths: to have a single entry, but got {:?} for stub {:?}",
                        no_star_stub_entry,
                        no_star_stub_option
                    ));
                }
                return Ok(ResolvedImport::ProjectLocalImport(path_buf_from_tsconfig(
                    tsconfig_paths,
                    &no_star_stub_entry[0],
                )));
            }
            let mut star_stub_to_check = stub_to_check.to_owned();
            star_stub_to_check.push_str("/*");
            let star_stub_option = tsconfig_paths
                .compiler_options
                .paths
                .get(&star_stub_to_check);
            if star_stub_option.is_some() {
                // match on star stub
                let star_stub_entry = star_stub_option.unwrap();
                if star_stub_entry.len() != 1 {
                    return Err(format!(
                        "Expected all members of paths: to have a single entry, but got {:?} for stub {:?}",
                        star_stub_entry,
                        no_star_stub_option
                    ));
                }
                return Ok(ResolvedImport::ProjectLocalImport(path_buf_from_tsconfig(
                    tsconfig_paths,
                    &switch_specifier_prefix(
                        &star_stub_to_check,
                        &star_stub_entry[0],
                        &import_specifier,
                    ),
                )));
            }
        }
    }

    // import specifier is not from the resolver. Use it here.
    return Ok(ResolvedImport::NodeModulesImport(
        import_specifier.to_owned(),
    ));
}

fn switch_specifier_prefix(
    matched_star_path: &str,
    replace_star_path: &str,
    import_specifier: &str,
) -> String {
    //
    // { "paths":
    //      "foo": [ "./packages/foo" ]
    //      "foo/lib/*": [ "./packages/foo/src/*" ]
    // }
    //
    // import "foo/lib/bar" -> "packages/foo/src/bar"

    if !replace_star_path.ends_with("/*") {
        return replace_star_path.to_owned();
    }
    let trailing_slice: &str =
        &import_specifier[matched_star_path.len() - 2..import_specifier.len()];
    let replace_no_star_slice: &str = &replace_star_path[0..replace_star_path.len() - 2];
    let mut resulting_string = String::from(replace_no_star_slice);
    resulting_string.push_str(trailing_slice);
    resulting_string
}

// Prefixes the specifier with the baseurl in the tsconfig, if any is defined
fn path_buf_from_tsconfig(
    tsconfig_paths_json: &TsconfigPathsJson,
    specifier_from_tsconfig_paths: &str,
) -> PathBuf {
    if tsconfig_paths_json.compiler_options.base_url.is_some() {
        // Join the base url onto the path, if present in the config
        let mut builder: RelativePathBuf = RelativePathBuf::new();
        builder.push(
            tsconfig_paths_json
                .compiler_options
                .base_url
                .as_ref()
                .unwrap(),
        );
        builder.push(specifier_from_tsconfig_paths);
        let rel_path =
            slashed_as_relative_path(&as_slashed_pathbuf(builder.into_string().as_str())).unwrap();
        return PathBuf::from(rel_path.as_str()).clean();
    } else {
        return PathBuf::from(RelativePathBuf::from(specifier_from_tsconfig_paths).as_str())
            .clean();
    }
}

#[cfg(test)]
mod test {
    extern crate lazy_static;
    extern crate relative_path;
    use crate::import_resolver::{
        resolve_ts_import, ResolvedImport, TsconfigPathsCompilerOptions, TsconfigPathsJson,
    };
    use lazy_static::lazy_static;
    use relative_path::RelativePathBuf;
    use std::path::PathBuf;

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

    lazy_static! {
        static ref TEST_TSCONFIG_JSON: TsconfigPathsJson = TsconfigPathsJson {
            compiler_options: TsconfigPathsCompilerOptions {
                base_url: Some(".".to_owned()),
                paths: map!(
                    "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                    "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                )
            }
        };
    }

    #[test]
    fn test_import_resolvers_relative() {
        let result = resolve_ts_import(
            &TEST_TSCONFIG_JSON,
            &RelativePathBuf::from("packages/my/importing/module"),
            "../imported/module",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/imported/module"
            )))
        )
    }

    #[test]
    fn test_non_glob_specifier() {
        let result = resolve_ts_import(
            &TEST_TSCONFIG_JSON,
            &RelativePathBuf::from("packages/my/importing/module"),
            "non-glob-specifier",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/non-glob-specifier/lib/index"
            )))
        )
    }

    #[test]
    fn test_glob_specifier() {
        let result = resolve_ts_import(
            &TEST_TSCONFIG_JSON,
            &RelativePathBuf::from("packages/my/importing/module"),
            "glob-specifier/lib/relative/after/glob/specifier/../../the/./specifier",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/glob-specifier/src/relative/after/the/specifier"
            )))
        )
    }

    #[test]
    fn test_import_resolvers_relative_with_base_url() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: Some("./base/url".to_owned()),
                    paths: map!(
                        "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module"),
            "../imported/module",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/imported/module"
            )))
        )
    }

    #[test]
    fn test_import_resolvers_relative_with_base_url_as_tsconfig_file() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: Some("./base/url".to_owned()),
                    paths: map!(
                        "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module.ts"),
            "../imported/module",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/imported/module"
            )))
        )
    }

    #[test]
    fn test_import_resolvers_relative_index() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: None,
                    paths: map!(
                        "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module.ts"),
            ".",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/importing/index"
            )))
        )
    }

    #[test]
    fn test_import_resolvers_relative_parent_index() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: None,
                    paths: map!(
                        "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module.ts"),
            "..",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/index"
            )))
        )
    }

    #[test]
    fn test_import_resolvers_relative_parent_specifier() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: None,
                    paths: map!(
                        "glob-specifier/lib/*" => vec!["packages/glob-specifier/src/*".to_owned()],
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module.ts"),
            "../imported/module",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "packages/my/imported/module"
            )))
        )
    }

    #[test]
    fn test_non_glob_specifier_with_base_url() {
        let result = resolve_ts_import(
            &TsconfigPathsJson {
                compiler_options: TsconfigPathsCompilerOptions {
                    base_url: Some("./base/url".to_owned()),
                    paths: map!(
                        "non-glob-specifier" => vec!["packages/non-glob-specifier/lib/index".to_owned()]
                    ),
                },
            },
            &RelativePathBuf::from("packages/my/importing/module"),
            "non-glob-specifier",
        );
        assert_eq!(
            result,
            Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
                "base/url/packages/non-glob-specifier/lib/index"
            )))
        )
    }
}
