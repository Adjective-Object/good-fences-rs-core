//! Strict JSON/JSONC to plain JavaScript, preserving JSON lexemes and own keys.
use std::path::Path;

use ast_segmenter::ModuleFormat;
use eyre::WrapErr;

fn export_value(format: ModuleFormat) -> &'static str {
    match format {
        ModuleFormat::CommonJs => "module.exports =",
        ModuleFormat::Esm => "export default",
    }
}

pub(crate) fn prepare(
    path: &Path,
    input: &[u8],
    format: ModuleFormat,
    jsonc: bool,
) -> eyre::Result<String> {
    let text = std::str::from_utf8(input)
        .wrap_err_with(|| format!("{}: source is not UTF-8", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized;
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
        parsed.diagnostics.is_empty() && !parsed.fatal_error,
        "JSONC syntax errors: {:?}",
        parsed.diagnostics
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
