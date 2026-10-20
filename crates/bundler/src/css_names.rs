//! Versioned, checkout-independent CSS selector identities and export surfaces.
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

use eyre::ensure;
use sha2::{Digest, Sha256};

use crate::css_syntax::{self, Edit, Name};

pub(crate) const REVISION: &str = "css-local-v1";

#[derive(Default)]
pub(crate) struct Registry {
    identities: BTreeMap<String, (String, String)>,
}

#[derive(Default)]
pub(crate) struct PreparedNames {
    pub mappings: BTreeMap<String, String>,
    pub edits: Vec<Edit>,
}

impl Registry {
    pub(crate) fn prepare(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
    ) -> eyre::Result<PreparedNames> {
        self.prepare_with(root, path, source, scoped_name)
    }

    fn prepare_with(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
        hash: impl Fn(&str, &str) -> String,
    ) -> eyre::Result<PreparedNames> {
        let syntax = css_syntax::parse(path, source)?;
        ensure!(
            syntax.scopes.is_empty(),
            "{}: CSS Modules :local/:global scope syntax is unsupported at {:?}",
            path.display(),
            syntax.scopes.first().map(|scope| &scope.span)
        );
        ensure!(
            syntax.compositions.is_empty(),
            "{}: CSS Modules composes is unsupported at {:?}",
            path.display(),
            syntax
                .compositions
                .first()
                .map(|composition| &composition.span)
        );
        let relative = path.strip_prefix(root)?;
        let relative = relative
            .components()
            .map(|component| match component {
                Component::Normal(value) => value
                    .to_str()
                    .ok_or_else(|| eyre::eyre!("non-UTF-8 CSS Modules path")),
                _ => eyre::bail!("CSS Modules path is not normalized: {}", path.display()),
            })
            .collect::<eyre::Result<Vec<_>>>()?
            .join("/");
        let mut prepared = PreparedNames::default();
        let mut exports: BTreeMap<String, &Name> = BTreeMap::new();
        for selector in &syntax.names {
            let name = &selector.name;
            let export = camel_case(&name.decoded);
            ensure!(
                !export.is_empty(),
                "{}: unrepresentable CSS Modules export for {:?} at {:?}",
                path.display(),
                name.decoded,
                name.span
            );
            if let Some(previous) = exports.insert(export.clone(), name) {
                ensure!(
                    previous.decoded == name.decoded,
                    "{}: CSS Modules export collision {:?}: {:?} at {:?} and {:?} at {:?}",
                    path.display(),
                    export,
                    previous.decoded,
                    previous.span,
                    name.decoded,
                    name.span
                );
            }
            let scoped = hash(&relative, &name.decoded);
            let identity = (relative.clone(), name.decoded.clone());
            if let Some(previous) = self.identities.insert(scoped.clone(), identity.clone()) {
                ensure!(
                    previous == identity,
                    "CSS Modules scoped-name collision {scoped:?}: {previous:?} and {identity:?} at {:?}",
                    name.span
                );
            }
            prepared.mappings.insert(export, scoped.clone());
            prepared.edits.push(Edit {
                span: name.span.clone(),
                replacement: scoped,
            });
        }
        Ok(prepared)
    }
}

fn camel_case(name: &str) -> String {
    let mut output = String::new();
    let mut upper = false;
    let chars: Vec<_> = name.chars().collect();
    for (index, &ch) in chars.iter().enumerate() {
        if matches!(ch, '-' | '_') {
            upper = true;
        } else {
            let boundary = ch.is_uppercase()
                && index > 0
                && (chars[index - 1].is_lowercase()
                    || chars[index - 1].is_numeric()
                    || chars[index - 1].is_uppercase()
                        && chars.get(index + 1).is_some_and(|ch| ch.is_lowercase()));
            if !output.is_empty() && (upper || boundary) {
                output.extend(ch.to_uppercase());
            } else {
                output.extend(ch.to_lowercase());
            }
            upper = false;
        }
    }
    output
}

fn scoped_name(path: &str, symbol: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(REVISION.as_bytes());
    digest.update([0]);
    for value in [path, symbol] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut name = String::from("_");
    for index in 0..10 {
        let bit = index * 6;
        let pair = u16::from(digest[bit / 8]) << 8 | u16::from(digest[bit / 8 + 1]);
        name.push(alphabet[((pair >> (10 - bit % 8)) & 63) as usize] as char);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_camel_case_handles_word_boundaries_without_dropping_nonidentifiers() {
        for (input, expected) in [
            ("my-class-name", "myClassName"),
            ("my_class_name", "myClassName"),
            ("myClassName", "myClassName"),
            ("--XMLHttp-request", "xmlHttpRequest"),
            ("_Upper_NAME", "upperName"),
            ("class", "class"),
            ("123", "123"),
            ("a+b", "a+b"),
            ("你好", "你好"),
        ] {
            assert_eq!(camel_case(input), expected);
        }
    }

    #[test]
    fn names_depend_only_on_relative_path_and_decoded_symbol() {
        let prepare = |root: &str, source| {
            Registry::default()
                .prepare(
                    Path::new(root),
                    &Path::new(root).join("a.module.css"),
                    source,
                )
                .unwrap()
        };
        let a = prepare("/checkout/one", r".my-class, #my-class, .caf\e9 {}");
        let b = prepare("/checkout/two", ".café {color:red} #my-class {}");
        assert_eq!(a.mappings, b.mappings);
        assert_eq!(a.edits[0].replacement, a.edits[1].replacement);
        assert_ne!(
            scoped_name("a.module.css", "café"),
            scoped_name("b.module.css", "café")
        );
        for name in a.mappings.values() {
            assert_eq!(name.len(), 11);
            assert!(name.starts_with('_'));
            assert!(name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-')));
        }
    }

    #[test]
    fn rejects_export_and_detected_hash_collisions_and_future_syntax() {
        let root = Path::new("/project");
        let path = root.join("a.module.css");
        let error = Registry::default()
            .prepare(root, &path, ".foo-bar, #fooBar {}")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("export collision")
                && error.contains("foo-bar")
                && error.contains("fooBar")
                && error.contains("1..8")
                && error.contains("11..17"),
            "{error}"
        );
        let error = Registry::default()
            .prepare_with(root, &path, ".a, .b {}", |_, _| "_collision".into())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("scoped-name collision"), "{error}");
        let mut registry = Registry::default();
        registry
            .prepare_with(root, &path, ".a, #a {}", |_, _| "_collision".into())
            .unwrap();
        assert!(registry
            .prepare_with(root, &root.join("b.module.css"), ".a {}", |_, _| {
                "_collision".into()
            })
            .is_err());
        for source in [
            ":global .a {}",
            ":local .a {}",
            ".a {composes: b}",
            ".--- {}",
        ] {
            assert!(
                Registry::default().prepare(root, &path, source).is_err(),
                "{source}"
            );
        }
    }
}
