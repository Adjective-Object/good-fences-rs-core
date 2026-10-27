//! Shared prepared-resource identity and require metadata.
use std::path::{Path, PathBuf};

use crate::loaders::{Loader, LoaderPolicy, StylesheetScoping};
use ast_segmenter::ModuleFormat;

#[cfg(test)]
pub(crate) use crate::loaders::file::url_module;
pub(crate) use crate::loaders::file::{asset_path, public_url_module};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    Script,
    Json,
    Stylesheet { module: bool },
    Asset,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequireValue {
    Default,
    LocalizedDefault,
    Namespace,
}

pub(crate) fn kind(path: &Path, policy: &LoaderPolicy) -> ResourceKind {
    match policy.select(path) {
        Ok(loader) if loader.script_syntax().is_some() => ResourceKind::Script,
        Ok(Loader::Json | Loader::Jsonc | Loader::Locstring) => ResourceKind::Json,
        Ok(Loader::File) => ResourceKind::Asset,
        Ok(loader) => ResourceKind::Stylesheet {
            module: loader
                .stylesheet()
                .is_some_and(|(_, scope)| scope == StylesheetScoping::Modules),
        },
        Err(_) => ResourceKind::Rejected,
    }
}

pub(crate) fn stylesheet_kind(path: &Path, policy: &LoaderPolicy) -> eyre::Result<ResourceKind> {
    let loader = policy.select_stylesheet(path)?;
    Ok(ResourceKind::Stylesheet {
        module: loader
            .stylesheet()
            .is_some_and(|(_, scope)| scope == StylesheetScoping::Modules),
    })
}

/// All generated wrappers are plain JS, independent of original input syntax.
pub(crate) fn parser_path(path: &Path, format: ModuleFormat) -> PathBuf {
    path.with_extension(if format == ModuleFormat::Esm {
        "prepared.mjs"
    } else {
        "prepared.cjs"
    })
}

pub(crate) fn prepare(
    path: &Path,
    input: &[u8],
    base: &crate::output_url::PublicBase,
    format: ModuleFormat,
    loader: Loader,
) -> eyre::Result<String> {
    match loader {
        Loader::Json | Loader::Jsonc => {
            crate::loaders::json::prepare(path, input, format, loader == Loader::Jsonc)
        }
        Loader::File => crate::loaders::file::prepare(path, input, base, format),
        _ => eyre::bail!(
            "loader {loader:?} is not a JSON/file pipeline: {}",
            path.display()
        ),
    }
}
