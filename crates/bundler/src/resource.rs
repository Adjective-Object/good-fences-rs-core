//! Resource preparation keeps original input identity separate from JS coordinates.
use std::path::{Path, PathBuf};

use eyre::WrapErr;

pub(crate) fn parser_path(path: &Path) -> PathBuf {
    if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        path.with_extension("json.cjs")
    } else {
        path.to_owned()
    }
}

pub(crate) fn prepare(path: &Path, input: &[u8]) -> eyre::Result<String> {
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
