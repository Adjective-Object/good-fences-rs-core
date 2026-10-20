//! Resource preparation keeps original input identity separate from JS coordinates.
use std::path::{Path, PathBuf};

use ast_segmenter::ModuleFormat;
use eyre::WrapErr;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    Script,
    Json,
    Stylesheet { module: bool },
    Asset,
}

pub(crate) fn kind(path: &Path, selection: &crate::build::CssModules) -> ResourceKind {
    if selection.matches(path) || path.extension().is_some_and(|extension| extension == "css") {
        return stylesheet_kind(path, selection);
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => ResourceKind::Json,
        Some("js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx") => ResourceKind::Script,
        _ => ResourceKind::Asset,
    }
}

/// A CSS @import supplies a stylesheet even if its filename has no CSS suffix.
pub(crate) fn stylesheet_kind(path: &Path, selection: &crate::build::CssModules) -> ResourceKind {
    ResourceKind::Stylesheet {
        module: matches!(selection, crate::build::CssModules::Everything)
            || selection.matches(path),
    }
}

pub(crate) fn is_asset(path: &Path) -> bool {
    kind(path, &crate::build::CssModules::Disabled) == ResourceKind::Asset
}

pub(crate) fn url_module(
    path: &Path,
    base: &crate::output_url::PublicBase,
) -> eyre::Result<String> {
    public_url_module(&base.join(path)?, ModuleFormat::CommonJs)
}

pub(crate) fn public_url_module(public_url: &str, format: ModuleFormat) -> eyre::Result<String> {
    Ok(format!(
        "{} {};\n",
        export_value(format),
        serde_json::to_string(public_url)?
    ))
}

fn export_value(format: ModuleFormat) -> &'static str {
    match format {
        ModuleFormat::CommonJs => "module.exports =",
        ModuleFormat::Esm => "export default",
    }
}

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

pub(crate) fn parser_path(path: &Path, format: ModuleFormat) -> PathBuf {
    let extension = if format == ModuleFormat::Esm {
        "mjs"
    } else {
        "cjs"
    };
    if is_asset(path) {
        path.with_extension(format!("asset.{extension}"))
    } else if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        path.with_extension(format!("json.{extension}"))
    } else {
        path.to_owned()
    }
}

pub(crate) fn prepare(
    path: &Path,
    input: &[u8],
    base: &crate::output_url::PublicBase,
    format: ModuleFormat,
) -> eyre::Result<String> {
    if is_asset(path) {
        if format == ModuleFormat::CommonJs {
            return url_module(&asset_path(path, input), base);
        }
        return public_url_module(&base.join(&asset_path(path, input))?, format);
    }
    let text = std::str::from_utf8(input)
        .wrap_err_with(|| format!("{}: source is not UTF-8", path.display()))?;
    if !path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        return Ok(text.to_owned());
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    serde_json::from_str::<serde::de::IgnoredAny>(text)
        .wrap_err_with(|| format!("{}: invalid JSON module", path.display()))?;
    let mut source = format!("{} (", export_value(format));
    let mut start = 0;
    let mut bytes = text.bytes().enumerate();
    while let Some((index, byte)) = bytes.next() {
        if byte != b'"' {
            continue;
        }
        let mut end = index + 1;
        while let Some((index, byte)) = bytes.next() {
            if byte == b'\\' {
                bytes.next();
            } else if byte == b'"' {
                end = index + 1;
                break;
            }
        }
        if text[end..].trim_start().starts_with(':') {
            // Computed keys preserve JSON's own "__proto__" properties.
            source.push_str(&text[start..index]);
            source.push('[');
            source.push_str(&text[index..end]);
            source.push(']');
            start = end;
        }
    }
    source.push_str(&text[start..]);
    source.push_str(");\n");
    Ok(source)
}
