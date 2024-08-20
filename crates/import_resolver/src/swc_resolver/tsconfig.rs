use super::context_data::ContextData;
use anyhow::{anyhow, ensure, Context, Result};
use path_clean::PathClean;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use swc_common::FileName;

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsJson {
    pub compiler_options: TsconfigPathsCompilerOptions,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TsconfigPathsCompilerOptions {
    pub base_url: Option<String>,
    pub paths: Option<HashMap<String, Vec<String>>>,
}

// Lifted from swc_ecma_loader-0.45.23/src/resolvers/tsc.rs
#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard {
        prefix: String,
    },
    /// No wildcard.
    Exact(String),
}

#[derive(Debug, Clone)]
pub struct ProcessedTsconfigPaths {
    pub paths: Vec<(Pattern, Vec<String>)>,
    pub base_url: PathBuf,
    // A filename representation of base_url, used for invoking the inner_resolver
    // with the base_url as a filename
    pub base_url_filename: FileName,
}

#[derive(Debug, Clone)]
pub enum ProcessedTsconfig {
    HasPaths(ProcessedTsconfigPaths),
    NoPaths,
}

impl ContextData for ProcessedTsconfig {
    // Parses a tsconfig.json file and returns a ProcessedTsconfig
    //
    // If the file is not a valid tsconfig.json file, returns None
    fn read_context_data(_: (), file_path: &Path) -> Result<Option<Self>> {
        let f = match File::open(file_path) {
            Ok(f) => f,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }

                return Err(anyhow!(
                    "Failed to open {} as tsconfig.json file: {:?}",
                    file_path.to_string_lossy(),
                    e
                ));
            }
        };

        let buf_reader = BufReader::new(f);
        let tsconfig_paths_json: TsconfigPathsJson = match serde_json::from_reader(buf_reader) {
            Ok(tsconfig) => tsconfig,
            Err(e) => {
                return Err(anyhow!(
                    "Failed to parse {:?} as tsconfig.json file: {:?}",
                    file_path,
                    e
                ))
            }
        };

        let raw_paths = match tsconfig_paths_json.compiler_options.paths {
            Some(p) => p,
            None => return Ok(Some(ProcessedTsconfig::NoPaths)),
        };

        let base_url = match tsconfig_paths_json.compiler_options.base_url {
            Some(url) => file_path.parent().unwrap().join(url).clean(),
            None => return Err(anyhow!(
                "Failed to parse {:?} as tsconfig.json file: base_url is missing, but paths was set",
                &file_path.to_str()
            )),
        };

        // assert paths are well-formed
        //
        // Lifted from swc_ecma_loader-0.45.23/src/resolvers/tsc.rs
        let pattern_paths = raw_paths
            .into_iter()
            .map(|(from, to)| -> Result<(Pattern, Vec<String>)> {
                ensure!(
                    !to.is_empty(),
                    "value of `paths.{}` should not be an empty array",
                    from,
                );

                let pos = from.as_bytes().iter().position(|&c| c == b'*');
                let pat = if from.contains('*') {
                    if from.as_bytes().iter().rposition(|&c| c == b'*') != pos {
                        panic!("`paths.{}` should have only one wildcard", from)
                    }

                    Pattern::Wildcard {
                        prefix: from[..pos.unwrap()].to_string(),
                    }
                } else {
                    ensure!(
                        to.len() == 1,
                        "value of `paths.{}` should be an array with one element because the src \
                         path does not contains * (wildcard)",
                        from,
                    );

                    Pattern::Exact(from)
                };

                Ok((pat, to))
            })
            .try_collect()
            .with_context(|| {
                format!(
                    "Failed to parse {:?} as tsconfig.json file: paths is not well-formed",
                    file_path
                )
            })?;

        let as_buf = PathBuf::from(base_url);
        let for_swc = ProcessedTsconfigPaths {
            paths: pattern_paths,
            base_url_filename: FileName::Real(as_buf.clone()),
            base_url: as_buf,
        };

        return Ok(Some(ProcessedTsconfig::HasPaths(for_swc)));
    }
}
