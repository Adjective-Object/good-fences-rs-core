use path_slash::PathBufExt;
use relative_path::{RelativePath, RelativePathBuf};
use serde::Deserialize;
use std::env::current_dir;
use std::path::{Path, PathBuf};
use std::string::String;

use crate::resolve::PathResolver;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub enum ResolvedImport {
    NodeModulesImport(String),
    ProjectLocalImport(PathBuf),
    ResourceFileImport,
}

pub const SOURCE_EXTENSIONS: &[&str] = &["js", "ts", "d.ts", "tsx", "jsx"];
pub const ASSET_EXTENSION: &[&str] = &["scss", "css", "svg", "png", "json", "gif"];

pub fn resolve_with_extension(
    base: &Path,
    imported_path: &str,
    resolver: impl PathResolver,
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
    let resolved = match resolver.resolve(base, imported_path) {
        Ok(r) => r,
        Err(e) => {
            if let Some(source) = e.source() {
                if source.to_string() == "failed to get the node_modules path" {
                    return Ok(ResolvedImport::NodeModulesImport(imported_path.to_owned()));
                }
            }
            for ext in SOURCE_EXTENSIONS {
                let file_with_ext = format!("{}.{}", &imported_path, ext);
                if let Ok(resolved) = resolver.resolve(base, &file_with_ext) {
                    let resolved = resolved.path.to_slash().unwrap().to_string();
                    return Ok(ResolvedImport::ProjectLocalImport(
                        resolved.replacen(&format!("{}/", cwd), "", 1).into(),
                    ));
                }
            }
            return Err(e);
        }
    };
    let resolved = resolved.path.to_slash().unwrap().to_string();
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

/// Classify fence-checker imports without compiler path aliases.
pub fn resolve_import(
    initial_path: &RelativePath,
    raw_import_specifier: &str,
) -> eyre::Result<ResolvedImport> {
    tracing::debug!(
        "resolve_import! {:?}, {:?}",
        initial_path,
        raw_import_specifier
    );

    // Directory imports retain the fence checker's extension probing.
    let import_specifier: String = if raw_import_specifier.ends_with('/') {
        format!("{raw_import_specifier}index")
    } else if raw_import_specifier == "." || raw_import_specifier == ".." {
        format!("{raw_import_specifier}/index")
    } else {
        raw_import_specifier.to_owned()
    };

    let buf = PathBuf::from(&import_specifier);
    if let Some(ext) = buf.extension() {
        if ext != "tsx" && ext != "ts" {
            return Ok(ResolvedImport::ResourceFileImport);
        }
    }

    if import_specifier.starts_with('.') {
        let parent = initial_path
            .parent()
            .ok_or_else(|| eyre::eyre!("source path {initial_path} had no parent?"))?;
        let joined: RelativePathBuf = parent.join(RelativePath::new(&import_specifier));
        return Ok(ResolvedImport::ProjectLocalImport(PathBuf::from(
            joined.normalize().as_str(),
        )));
    }

    Ok(ResolvedImport::NodeModulesImport(import_specifier))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn relative_imports_keep_directory_and_extension_classification() {
        let source = RelativePath::new("packages/my/importing/module.ts");
        for (request, expected) in [
            ("../imported/module", "packages/my/imported/module"),
            (".", "packages/my/importing/index"),
            ("..", "packages/my/index"),
            ("./nested/", "packages/my/importing/nested/index"),
            ("../imported/module.ts", "packages/my/imported/module.ts"),
        ] {
            assert_eq!(
                resolve_import(source, request).unwrap(),
                ResolvedImport::ProjectLocalImport(expected.into()),
            );
        }
        assert_eq!(
            resolve_import(source, "./style.css").unwrap(),
            ResolvedImport::ResourceFileImport,
        );
    }

    #[test]
    fn bare_imports_remain_package_requests_without_compiler_aliases() {
        for request in ["non-glob-specifier", "glob-specifier/lib/nested", "node:fs"] {
            assert_eq!(
                resolve_import(RelativePath::new("src/main.ts"), request).unwrap(),
                ResolvedImport::NodeModulesImport(request.into()),
            );
        }
    }
}
