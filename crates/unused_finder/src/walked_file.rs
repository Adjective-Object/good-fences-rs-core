use std::{
    borrow::Borrow,
    path::{self, Path, PathBuf},
};

use anyhow::{Context, Result};
use const_format::concatcp;
use packagejson::PackageJson;
use packagejson_exports::PackageExportRewriteData;
use path_clean::PathClean;
use path_slash::PathBufExt;

use crate::{parse::RawImportExportInfo, ResolvedImportExportInfo};

/// Source file discovered during the source walk
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct WalkedSourceFile {
    pub owning_package: Option<String>,
    /// The full path of this source file
    pub source_file_path: PathBuf,
    /// The imports and exports parsed from this source file
    pub import_export_info: RawImportExportInfo,
}

/// package.json file discovered during the source walk
#[derive(Debug, PartialEq)]
pub struct WalkedPackage {
    /// path to the package.json this was parsed from
    pub package_path: PathBuf,
    /// The parsed package.json file
    pub package_json: PackageJson,
    /// cached export info, calculated from the PackageJson's "exports" field
    pub export_info: Option<PackageExportRewriteData>,
    /// The cleaned form of the "main" field of this package.json, if it
    /// exists and needs cleaning. Otherwise, this will be None.
    pub cleaned_main: Option<String>,
    /// The cleaned form of the "module" field of this package.json, if it
    /// exists and needs cleaning. Otherwise, this will be None.
    pub cleaned_module: Option<String>,
}

impl WalkedPackage {
    /// Parses a package.json file at the given path and stores the parsed data in a WalkedPackage
    ///
    /// WalkedPackage contains the parsed package.json data, as well as some cached data for
    /// faster lookups during the analysis phase.
    pub fn from_path(p: impl Borrow<Path> + Into<PathBuf>) -> Result<Self, anyhow::Error> {
        let as_display = p.borrow().display();
        let f = std::fs::File::open(p.borrow())
            .with_context(|| format!("Failed to open package.json file at path: {}", as_display))?;
        Self::read(p, &f)
    }

    /// Parses a package.json file at the given path and stores the parsed data in a WalkedPackage
    ///
    /// WalkedPackage contains the parsed package.json data, as well as some cached data for
    /// faster lookups during the analysis phase.
    pub fn read(
        filepath: impl Borrow<Path> + Into<PathBuf>,
        reader: impl std::io::Read,
    ) -> Result<Self, anyhow::Error> {
        let package_json: PackageJson = serde_json::from_reader(reader).with_context(|| {
            format!(
                "Failed to read package.json file at path: {}",
                filepath.borrow().display()
            )
        })?;

        let export_info = match &package_json.exports {
            Some(exports) => Some(PackageExportRewriteData::try_from(exports).with_context(
                || {
                    format!(
                        "Failed to parse package.json exports field at path: {}",
                        filepath.borrow().display(),
                    )
                },
            )?),
            None => None,
        };

        let cleaned_main = package_json
            .main
            .as_ref()
            .map(|main| format!("./{}", PathBuf::from(main).clean().to_slash_lossy()));
        let cleaned_module = package_json
            .module
            .as_ref()
            .map(|module| format!("./{}", PathBuf::from(module).clean().to_slash_lossy()));

        Ok(Self {
            package_path: filepath.into(),
            package_json,
            export_info,
            cleaned_main,
            cleaned_module,
        })
    }

    pub fn is_abspath_exported(
        &self,
        // The absolute path of the file to check
        abs_path: impl AsRef<Path>,
    ) -> Result<bool, anyhow::Error> {
        let as_buf = PathBuf::from(&self.package_path);
        let package_dir_path = as_buf.parent().unwrap_or_else(|| &as_buf);
        let as_relative_path = pathdiff::diff_paths(abs_path, package_dir_path)
            .with_context(|| "Failed to diff paths")?;

        if as_relative_path.starts_with(concatcp!("..", path::MAIN_SEPARATOR)) {
            // The file is outside the package, so it's not exported
            return Ok(false);
        }

        let export_info = match &self.export_info {
            Some(info) => info,
            None => {
                // if there is no "exports" field, treat all files in the package as exported
                return Ok(true);
            }
        };

        let as_rel_slashed = as_relative_path.to_slash().unwrap();
        let mut package_relative_path = String::with_capacity(as_rel_slashed.len() + 2);
        package_relative_path.push_str("./");
        package_relative_path.push_str(&as_rel_slashed);

        // Check main and module fields
        if let Some(ref main) = self.cleaned_main {
            if main == &package_relative_path {
                return Ok(true);
            }
        }
        if let Some(ref module) = self.cleaned_module {
            if module == &package_relative_path {
                return Ok(true);
            }
        }

        // check against the export info
        Ok(!export_info.is_exported(&package_relative_path))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use stringreader::StringReader;

    #[test]
    fn test_exports_main() {
        let pkg = WalkedPackage::read(
            PathBuf::from_slash("/path/to/package.json"),
            StringReader::new(
                r#"{
                "main": "./main.js",
                "exports": {}
            }"#,
            ),
        )
        .context("parsing test data to string")
        .unwrap();

        assert!(pkg.is_abspath_exported("/path/to/main.js").unwrap());
        assert!(!pkg.is_abspath_exported("/path/to/other.js").unwrap());
    }

    #[test]
    fn test_exports_module() {
        let pkg = WalkedPackage::read(
            PathBuf::from_slash("/path/to/package.json"),
            StringReader::new(
                r#"{
                "module": "./esm/module.js",
                "exports": {}
            }"#,
            ),
        )
        .context("parsing test data to string")
        .unwrap();

        assert!(pkg.is_abspath_exported("/path/to/esm/module.js").unwrap(),);
        assert!(!pkg.is_abspath_exported("/path/to/cjs/module.js").unwrap());
    }

    #[test]
    fn test_exports_all_export_conditions() {
        let pkg = WalkedPackage::read(
            PathBuf::from_slash("/path/to/package.json"),
            StringReader::new(
                r#"{
                "exports": {
                    ".": "./main.js",
                    "./foo": {
                        "default": "./foo/index.js",
                        "bar": "./foo/bar.js"
                    }
                }
            }"#,
            ),
        )
        .context("parsing test data to string")
        .unwrap();

        assert!(pkg.is_abspath_exported("/path/to/main.js").unwrap());
        assert!(pkg.is_abspath_exported("/path/to/foo/index.js").unwrap());
        assert!(pkg.is_abspath_exported("/path/to/foo/bar.js").unwrap());
        assert!(!pkg.is_abspath_exported("/path/to/foo/other.js").unwrap());
    }
}

/// Source file discovered during the source walk
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ResolvedSourceFile {
    /// Name of the package that this source file belongs to, as of the last walk
    pub owning_package: Option<String>,
    /// The full path of this source file
    pub source_file_path: PathBuf,
    /// The imports and
    pub import_export_info: ResolvedImportExportInfo,
}
