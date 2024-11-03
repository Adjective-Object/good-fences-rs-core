use path_clean::PathClean as _;
use path_slash::PathBufExt;
use relative_path::{RelativePath, RelativePathBuf};
use serde::Deserialize;
use std::env::current_dir;
use std::path::{Path, PathBuf};
use std::string::String;
use swc_common::FileName;
use swc_ecma_loader::resolve::Resolve;
use tsconfig_paths::TsconfigPathsJson;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum ResolvedImport {
    NodeModulesImport(String),
    ProjectLocalImport(PathBuf),
    ResourceFileImport,
}

pub const SOURCE_EXTENSIONS: &[&str] = &["js", "ts", "d.ts", "tsx", "jsx"];
pub const ASSET_EXTENSION: &[&str] = &["scss", "css", "svg", "png", "json", "gif"];

pub fn resolve_with_extension(
    base: FileName,
    imported_path: &str,
    resolver: impl Resolve,
) -> anyhow::Result<ResolvedImport> {
    if is_resource_file(imported_path) {
        return Ok(ResolvedImport::ResourceFileImport);
    }
    // canonicalize() adds "\\\\?\\" for windows to guarantee we remove cwd from the final resolved path
    let cwd = current_dir()?
        .canonicalize()?
        .to_slash()
        .unwrap()
        .to_string();
    let resolved = match resolver.resolve(&base, imported_path) {
        Ok(r) => r,
        Err(e) => {
            if let Some(source) = e.source() {
                if source.to_string() == "failed to get the node_modules path" {
                    return Ok(ResolvedImport::NodeModulesImport(imported_path.to_owned()));
                }
            }
            for ext in SOURCE_EXTENSIONS {
                let file_with_ext = format!("{}.{}", &imported_path, ext);
                if let Ok(resolved) = resolver.resolve(&base, &file_with_ext) {
                    let resolved = match resolved.filename {
                        FileName::Real(f) => f.to_slash().unwrap().to_string(),
                        _ => resolved.filename.to_string(),
                    };
                    return Ok(ResolvedImport::ProjectLocalImport(
                        resolved.replacen(&format!("{}/", cwd), "", 1).into(),
                    ));
                }
            }
            return Err(e);
            // return Ok(ResolvedImport::NodeModulesImport(imported_specifier.to_string()))
        }
    };
    // let resolved = RelativePath::new(&resolved.to_string())..to_path("");
    let resolved = match resolved.filename {
        FileName::Real(f) => f.to_slash().unwrap().to_string(),
        _ => resolved.filename.to_string(),
    };
    // If we found a local file it starts with cwd
    if resolved.starts_with(&cwd) {
        if is_resource_file(&resolved) || resolved.ends_with(".graphql") {
            return Ok(ResolvedImport::ResourceFileImport);
        }
        return Ok(ResolvedImport::ProjectLocalImport(
            resolved.replacen(&format!("{}/", cwd), "", 1).into(),
        ));
    }
    Ok(ResolvedImport::NodeModulesImport(imported_path.into()))
}

fn is_resource_file(resolved: &str) -> bool {
    ASSET_EXTENSION
        .iter()
        .any(|ext| resolved.ends_with(&format!(".{}", ext)))
}

pub fn resolve_ts_import<'a>(
    tsconfig_paths: &'a TsconfigPathsJson,
    initial_path: &RelativePath,
    raw_import_specifier: &'a str,
) -> anyhow::Result<ResolvedImport, String> {
    tracing::debug!(
        "resolve_ts_import! {:?}, {:?}",
        initial_path,
        raw_import_specifier
    );

    // this is a directory import, so we want to add index.ts to the end of the file
    let import_specifier: String = if raw_import_specifier.ends_with('/') {
        let mut r = String::with_capacity(raw_import_specifier.len() + 5);
        r.push_str(raw_import_specifier);
        r.push_str("index");
        r
    } else if raw_import_specifier == "." || raw_import_specifier == ".." {
        let mut r = String::with_capacity(raw_import_specifier.len() + 6);
        r.push_str(raw_import_specifier);
        r.push_str("/index");
        r
    } else {
        raw_import_specifier.to_owned()
    };

    // short circuit when importing non-ts resource files.
    let buf = PathBuf::from(import_specifier.clone());
    let ext = buf.extension();
    if let Some(ext) = ext {
        if ext != "tsx" && ext != "ts" {
            return Ok(ResolvedImport::ResourceFileImport);
        }
    }

    if import_specifier.starts_with(".") {
        // relative import -- bypass tsconfig
        let parent_path = initial_path.parent();
        let joined_path: RelativePathBuf = match parent_path {
            Some(p) => p.join(RelativePath::new(&import_specifier)),
            None => return Err(format!("source path {:} had no parent?", initial_path)),
        };
        return Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
            joined_path.normalize().as_str(),
        )));
    } else {
        // tsconfig.paths.json imports
        let import_specifier_path = Path::new(&import_specifier);
        for segment in import_specifier_path.ancestors() {
            // match on starless stub
            let stub_to_check_option = segment.to_str();
            if stub_to_check_option.is_none() {
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
    Ok(ResolvedImport::NodeModulesImport(
        import_specifier.to_owned(),
    ))
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
    if let Some(base_url) = &tsconfig_paths_json.compiler_options.base_url {
        // Join the base url onto the path, if present in the config
        let mut builder: RelativePathBuf = RelativePathBuf::new();
        builder.push(base_url);
        builder.push(specifier_from_tsconfig_paths);
        PathBuf::from(RelativePathBuf::from(builder.to_string()).as_str()).clean()
    } else {
        PathBuf::from(RelativePathBuf::from(specifier_from_tsconfig_paths).as_str()).clean()
    }
}

#[cfg(test)]
mod test {
    extern crate lazy_static;
    extern crate relative_path;
    use super::{resolve_ts_import, ResolvedImport, TsconfigPathsJson};
    use lazy_static::lazy_static;
    use relative_path::RelativePathBuf;
    use std::path::PathBuf;
    use tsconfig_paths::TsconfigPathsCompilerOptions;

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
