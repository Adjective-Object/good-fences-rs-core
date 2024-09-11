use ftree_cache::context_data::FileContextCache;
use anyhow::{bail, Context, Result};
use std::path::{Component, Path};
use swc_common::FileName;
use swc_ecma_loader::resolve::{Resolution, Resolve};
use tracing::info;
use tracing::{debug, trace, warn, Level};

use super::tsconfig::{Pattern, ProcessedTsconfig, ProcessedTsconfigPaths};

/// The type of the cache that stores data derived from tsconfig.json
///
pub type TsconfigCache = FileContextCache<ProcessedTsconfig, "tsconfig.json">;

// Wrapper struct used to implement resolve() for ProcessedTsconfigPaths without needing to
// copy the processed paths each time a resolver is constructed
pub struct TsconfigPathsResolver<'tsconfig, R: Resolve> {
    tsconfig: &'tsconfig ProcessedTsconfigPaths,
    inner_resolver: R,
}

impl<'tsconfig, R: Resolve> TsconfigPathsResolver<'tsconfig, R> {
    pub fn new(
        tsconfig: &'tsconfig ProcessedTsconfigPaths,
        inner_resolver: R,
    ) -> TsconfigPathsResolver<'tsconfig, R> {
        TsconfigPathsResolver {
            tsconfig,
            inner_resolver,
        }
    }

    /// Calls the inner resolver, and if it fails, tries again using the base_url of the
    /// tsconfig.json file as the base of the resolution
    ///
    /// Lifted from swc_ecma_loader-0.45.23/src/resolvers/tsc.rs
    fn invoke_inner_resolver(&self, base: &FileName, module_specifier: &str) -> Result<Resolution> {
        let res = self
            .inner_resolver
            .resolve(base, module_specifier)
            .with_context(|| {
                format!(
                    "failed to resolve `{module_specifier}` from `{base}` using inner \
                 resolver\nbase_url={}",
                    self.tsconfig.base_url_filename
                )
            });

        match res {
            Ok(resolved) => {
                info!(
                    "Resolved `{}` as `{}` from `{}`",
                    module_specifier, resolved.filename, base
                );

                let is_base_in_node_modules = if let FileName::Real(v) = base {
                    v.components().any(|c| match c {
                        Component::Normal(v) => v == "node_modules",
                        _ => false,
                    })
                } else {
                    false
                };
                let is_target_in_node_modules = if let FileName::Real(v) = &resolved.filename {
                    v.components().any(|c| match c {
                        Component::Normal(v) => v == "node_modules",
                        _ => false,
                    })
                } else {
                    false
                };

                // If node_modules is in path, we should return module specifier.
                if !is_base_in_node_modules && is_target_in_node_modules {
                    return Ok(Resolution {
                        filename: FileName::Real(module_specifier.into()),
                        ..resolved
                    });
                }

                Ok(resolved)
            }

            Err(err) => {
                warn!("{:?}", err);
                Err(err)
            }
        }
    }
}

/// Implements Resolve for the ProcessedTsconfigPathsResolver
///
/// Lifted from swc_ecma_loader-0.45.23/src/resolvers/tsc.rs
impl<'tsconfig, R: Resolve> Resolve for TsconfigPathsResolver<'tsconfig, R> {
    fn resolve(&self, base: &FileName, module_specifier: &str) -> Result<Resolution> {
        if module_specifier.contains("getReadWriteRecipientViewStateFromEmailAddress") {
            println!("tsconfig-resolver: {:?}", module_specifier);
        }

        let _tracing = if cfg!(debug_assertions) {
            Some(
                tracing::span!(
                    Level::ERROR,
                    "TsConfigResolver::resolve",
                    base_url = tracing::field::display(self.tsconfig.base_url.display()),
                    base = tracing::field::display(base),
                    src = tracing::field::display(module_specifier),
                )
                .entered(),
            )
        } else {
            None
        };

        if module_specifier.starts_with('.')
            && (module_specifier == ".."
                || module_specifier.starts_with("./")
                || module_specifier.starts_with("../"))
        {
            return self
                .invoke_inner_resolver(base, module_specifier)
                .context("not processed by tsc resolver because it's relative import");
        }

        if let FileName::Real(v) = base {
            if v.components().any(|c| match c {
                Component::Normal(v) => v == "node_modules",
                _ => false,
            }) {
                return self.invoke_inner_resolver(base, module_specifier).context(
                    "not processed by tsc resolver because base module is in node_modules",
                );
            }
        }

        info!("Checking `jsc.paths`");

        // https://www.typescriptlang.org/docs/handbook/module-resolution.html#path-mapping
        for (from, to) in &self.tsconfig.paths {
            match from {
                Pattern::Wildcard { prefix } => {
                    debug!("Checking `{}` in `jsc.paths`", prefix);

                    let extra = module_specifier.strip_prefix(prefix);
                    let extra = match extra {
                        Some(v) => v,
                        None => {
                            if cfg!(debug_assertions) {
                                trace!("skip because src doesn't start with prefix");
                            }
                            continue;
                        }
                    };

                    if cfg!(debug_assertions) {
                        debug!("Extra: `{}`", extra);
                    }

                    let mut errors = vec![];
                    for target in to {
                        let replaced = target.replace('*', extra);

                        let _tracing = if cfg!(debug_assertions) {
                            Some(
                                tracing::span!(
                                    Level::ERROR,
                                    "TsConfigResolver::resolve::jsc.paths",
                                    replaced = tracing::field::display(&replaced),
                                )
                                .entered(),
                            )
                        } else {
                            None
                        };

                        let relative = format!("./{}", replaced);

                        let res = self
                            .invoke_inner_resolver(base, module_specifier)
                            .or_else(|_| {
                                self.invoke_inner_resolver(
                                    &self.tsconfig.base_url_filename,
                                    &relative,
                                )
                            })
                            .or_else(|_| {
                                self.invoke_inner_resolver(
                                    &self.tsconfig.base_url_filename,
                                    &replaced,
                                )
                            });

                        errors.push(match res {
                            Ok(resolved) => return Ok(resolved),
                            Err(err) => err,
                        });

                        if to.len() == 1 && !prefix.is_empty() {
                            info!(
                                "Using `{}` for `{}` because the length of the jsc.paths entry is \
                                 1",
                                replaced, module_specifier
                            );
                            return Ok(Resolution {
                                slug: Some(
                                    replaced
                                        .split([std::path::MAIN_SEPARATOR, '/'])
                                        .last()
                                        .unwrap()
                                        .into(),
                                ),
                                filename: FileName::Real(replaced.into()),
                            });
                        }
                    }

                    bail!(
                        "`{}` matched `{}` (from tsconfig.paths) but failed to resolve:\n{:?}",
                        module_specifier,
                        prefix,
                        errors
                    )
                }
                Pattern::Exact(from) => {
                    // Should be exactly matched
                    if module_specifier != from {
                        continue;
                    }

                    let tp = Path::new(&to[0]);
                    let slug = to[0]
                        .split([std::path::MAIN_SEPARATOR, '/'])
                        .last()
                        .map(|v| v.into());
                    if tp.is_absolute() {
                        return Ok(Resolution {
                            filename: FileName::Real(tp.into()),
                            slug,
                        });
                    }

                    if let Ok(res) =
                        self.resolve(&self.tsconfig.base_url_filename, &format!("./{}", &to[0]))
                    {
                        return Ok(Resolution {
                            slug: match &res.filename {
                                FileName::Real(p) => p
                                    .file_stem()
                                    .filter(|&s| s != "index")
                                    .map(|v| v.to_string_lossy().into()),
                                _ => None,
                            },
                            ..res
                        });
                    }

                    return Ok(Resolution {
                        filename: FileName::Real(self.tsconfig.base_url.join(&to[0])),
                        slug,
                    });
                }
            }
        }

        if !module_specifier.starts_with('.') {
            let path = self.tsconfig.base_url.join(module_specifier);

            // https://www.typescriptlang.org/docs/handbook/modules/reference.html#baseurl
            if let Ok(v) = self.invoke_inner_resolver(base, &path.to_string_lossy()) {
                return Ok(v);
            }
        }

        self.invoke_inner_resolver(base, module_specifier)
    }
}
