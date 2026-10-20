//! Resource preparation keeps original input identity separate from JS coordinates.
use std::path::{Path, PathBuf};

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
        Some("json" | "jsonc") => ResourceKind::Json,
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
    public_url_module(&base.join(path)?)
}

pub(crate) fn public_url_module(public_url: &str) -> eyre::Result<String> {
    Ok(format!(
        "module.exports = {};\n",
        serde_json::to_string(public_url)?
    ))
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

pub(crate) fn parser_path(path: &Path) -> PathBuf {
    if is_asset(path) {
        path.with_extension("asset.cjs")
    } else if path
        .extension()
        .is_some_and(|extension| extension == "json" || extension == "jsonc")
    {
        path.with_extension("json.cjs")
    } else {
        path.to_owned()
    }
}

pub(crate) fn prepare(
    path: &Path,
    input: &[u8],
    base: &crate::output_url::PublicBase,
) -> eyre::Result<String> {
    if is_asset(path) {
        return url_module(&asset_path(path, input), base);
    }
    let text = std::str::from_utf8(input)
        .wrap_err_with(|| format!("{}: source is not UTF-8", path.display()))?;
    if !path
        .extension()
        .is_some_and(|extension| extension == "json" || extension == "jsonc")
    {
        return Ok(text.to_owned());
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized;
    let jsonc = path.extension().is_some_and(|extension| extension == "jsonc");
    let text = if jsonc {
        normalized = normalize_jsonc(text)
            .wrap_err_with(|| format!("{}: invalid JSONC module", path.display()))?;
        normalized.as_str()
    } else {
        text
    };
    serde_json::from_str::<serde::de::IgnoredAny>(text).wrap_err_with(|| {
        let format = if jsonc { "JSONC" } else { "JSON" };
        format!("{}: invalid {format} module", path.display())
    })?;
    let mut source = String::from("module.exports = (");
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

fn normalize_jsonc(text: &str) -> eyre::Result<String> {
    use oxc_parser::{config::TokensParserConfig, Kind, Parser};

    // Parse as an expression, including object/scalar roots and trailing line comments.
    // Keep original lexemes: deserializing/reserializing would lose duplicate keys,
    // negative zero, large numbers and escaped lone surrogates.
    let source = format!("(\n{text}\n)");
    // JSONC line comments end only at CR/LF, unlike JavaScript. Keep byte offsets
    // while hiding JS-only line separators from the lexer, not from the output.
    let lexer_source = source.replace(['\u{2028}', '\u{2029}'], "   ");
    let allocator = oxc_allocator::Allocator::default();
    let parsed = Parser::new(&allocator, &lexer_source, oxc_span::SourceType::mjs())
        .with_config(TokensParserConfig)
        .parse();
    eyre::ensure!(
        parsed.errors.is_empty() && !parsed.panicked,
        "JSONC syntax errors: {:?}",
        parsed.errors
    );
    let mut normalized = source.as_bytes().to_vec();
    for comment in &parsed.program.comments {
        let range = comment.span.start as usize..comment.span.end as usize;
        let original = source
            .get(range.clone())
            .ok_or_else(|| eyre::eyre!("invalid comment span"))?;
        eyre::ensure!(
            original.starts_with("//") || original.starts_with("/*"),
            "only // and /* */ comments are supported"
        );
        for byte in &mut normalized[range] {
            if !matches!(*byte, b'\r' | b'\n') {
                *byte = b' ';
            }
        }
    }
    for tokens in parsed.tokens.windows(3) {
        if tokens[1].kind() == Kind::Comma
            && matches!(tokens[2].kind(), Kind::RCurly | Kind::RBrack)
            // Array holes are not trailing commas: leave them for strict validation.
            && !matches!(tokens[0].kind(), Kind::Comma | Kind::LBrack)
        {
            let offset = tokens[1].start() as usize;
            eyre::ensure!(normalized.get(offset) == Some(&b','), "invalid comma span");
            normalized[offset] = b' ';
        }
    }
    // Strict JSON validation by the caller rejects every remaining JS/JSON5 extension.
    Ok(String::from_utf8(normalized[2..2 + text.len()].to_vec())?)
}
