//! Public resource addresses are URLs, independent of owned artifact paths.
use std::path::{Component, Path};

use eyre::{ensure, WrapErr};

pub(crate) struct PublicBase {
    value: String,
    relative: bool,
}

impl PublicBase {
    pub(crate) fn new(value: &str) -> eyre::Result<Self> {
        ensure!(!value.is_empty(), "output.base_url must not be empty");
        ensure!(
            !value.contains(['?', '#', '\\']) && !value.chars().any(char::is_control),
            "output.base_url must be a directory URL without queries, fragments, backslashes or controls"
        );
        ensure!(
            !value.starts_with("//"),
            "output.base_url must not be protocol-relative"
        );
        let bytes = value.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'%' {
                ensure!(
                    bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                        && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit),
                    "output.base_url contains an invalid percent escape"
                );
                let decoded = u8::from_str_radix(&value[index + 1..index + 3], 16)?;
                ensure!(
                    !decoded.is_ascii_control() && decoded != b'\\',
                    "output.base_url contains an unsafe percent escape"
                );
            }
        }
        let first = value.split('/').next().unwrap_or_default();
        let absolute = first.contains(':');
        let mut value = if absolute {
            let (scheme, rest) = value
                .split_once("://")
                .ok_or_else(|| eyre::eyre!("output.base_url requires an HTTP(S) directory URL"))?;
            ensure!(
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"),
                "output.base_url only supports HTTP(S) URLs"
            );
            let authority = rest.split('/').next().unwrap_or_default();
            ensure!(
                !authority.is_empty()
                    && !authority.contains('@')
                    && !authority.chars().any(char::is_whitespace),
                "output.base_url requires a host and must not contain credentials"
            );
            let escaped = format!(
                "{scheme}://{authority}{}",
                escape_path(&rest[authority.len()..])
            );
            let parsed = url::Url::parse(&escaped).wrap_err("invalid output.base_url")?;
            ensure!(parsed.has_host(), "output.base_url requires a host");
            parsed.to_string()
        } else {
            escape_path(value)
        };
        if !value.ends_with('/') {
            value.push('/');
        }
        Ok(Self {
            relative: !absolute && !value.starts_with('/'),
            value,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn relative(&self) -> bool {
        self.relative
    }

    pub(crate) fn join(&self, target: &Path) -> eyre::Result<String> {
        Ok(format!("{}{}", self.value, artifact_path(target)?))
    }

    pub(crate) fn css_fetch(&self, stylesheet: &Path, target: &Path) -> eyre::Result<String> {
        if !self.relative {
            return self.join(target);
        }
        // The browser already fetched the stylesheet through the relative base.
        // Applying that base again here would produce assets/assets/... requests.
        let from = artifact_path(stylesheet)?;
        let to = artifact_path(target)?;
        let mut from: Vec<_> = from.split('/').collect();
        from.pop();
        let to: Vec<_> = to.split('/').collect();
        let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
        Ok(format!(
            "{}{}",
            if common == from.len() {
                "./".to_owned()
            } else {
                "../".repeat(from.len() - common)
            },
            to[common..].join("/")
        ))
    }
}

fn artifact_path(path: &Path) -> eyre::Result<String> {
    path.components()
        .map(|part| {
            let Component::Normal(part) = part else {
                eyre::bail!("resource URL requires an owned relative artifact path");
            };
            Ok(escape_path(part.to_str().ok_or_else(|| {
                eyre::eyre!("resource URL path is not UTF-8")
            })?))
        })
        .collect::<eyre::Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn escape_path(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'/'
                    | b':'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b'%'
            )
        {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").expect("write to String");
        }
    }
    result
}
