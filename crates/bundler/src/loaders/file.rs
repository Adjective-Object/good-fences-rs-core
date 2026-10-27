//! Byte-preserving content-addressed URL resources.
use std::path::{Path, PathBuf};

use ast_segmenter::ModuleFormat;
use sha2::{Digest, Sha256};

pub(crate) fn asset_path(path: &Path, input: &[u8]) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            extension.len() <= 16 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .unwrap_or("bin");
    PathBuf::from(format!("assets/{:x}.{extension}", Sha256::digest(input)))
}

#[cfg(test)]
pub(crate) fn url_module(
    path: &Path,
    base: &crate::output_url::PublicBase,
) -> eyre::Result<String> {
    public_url_module(&base.join(path)?, ModuleFormat::CommonJs)
}

pub(crate) fn public_url_module(public_url: &str, format: ModuleFormat) -> eyre::Result<String> {
    Ok(format!(
        "{} {};\n",
        if format == ModuleFormat::CommonJs {
            "module.exports ="
        } else {
            "export default"
        },
        serde_json::to_string(public_url)?
    ))
}

pub(crate) fn prepare(
    path: &Path,
    input: &[u8],
    base: &crate::output_url::PublicBase,
    format: ModuleFormat,
) -> eyre::Result<String> {
    public_url_module(&base.join(&asset_path(path, input))?, format)
}
