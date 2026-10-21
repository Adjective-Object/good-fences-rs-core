//! Bounded add-only structural invocation rules; never JavaScript evaluation.
use ast_segmenter::views::{CalleeShape, IdentifierCallee, Invocation};
use eyre::{ensure, WrapErr};
use sha2::{Digest, Sha256};

pub const MAX_PATTERNS: usize = 128;
pub const MAX_PATTERN_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatternShape {
    Global(String),
    GlobalMember { receiver: String, property: String },
    AnyMember(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    canonical: String,
    shape: PatternShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectPolicy {
    patterns: Vec<Pattern>,
    honor_pure: bool,
}

impl Default for EffectPolicy {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            honor_pure: true,
        }
    }
}

impl EffectPolicy {
    pub fn parse(patterns: impl IntoIterator<Item = impl AsRef<str>>) -> eyre::Result<Self> {
        let mut parsed = Vec::new();
        for (index, pattern) in patterns.into_iter().enumerate() {
            ensure!(
                index < MAX_PATTERNS,
                "at most {MAX_PATTERNS} effect patterns are supported"
            );
            parsed.push(Pattern::parse(pattern.as_ref()).wrap_err_with(|| {
                format!("effect pattern {}: {:?}", index + 1, pattern.as_ref())
            })?);
        }
        parsed.sort_by(|a, b| a.canonical.cmp(&b.canonical));
        parsed.dedup();
        Ok(Self {
            patterns: parsed,
            ..Self::default()
        })
    }

    pub fn with_pure_annotations(mut self, enabled: bool) -> Self {
        self.honor_pure = enabled;
        self
    }

    pub fn honors_pure_annotations(&self) -> bool {
        self.honor_pure
    }

    pub fn canonical_patterns(&self) -> impl Iterator<Item = &str> {
        self.patterns.iter().map(|p| p.canonical.as_str())
    }

    pub fn matching_patterns<'a>(&'a self, call: Invocation<'a>) -> impl Iterator<Item = &'a str> {
        self.patterns
            .iter()
            .filter(move |p| p.matches(call))
            .map(|p| p.canonical.as_str())
    }

    /// Canonical effective policy identity, separate from parser/cache identity.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"good-fences-effects-v1\0");
        hash.update([u8::from(self.honor_pure)]);
        for pattern in &self.patterns {
            hash.update((pattern.canonical.len() as u32).to_be_bytes());
            hash.update(pattern.canonical.as_bytes());
        }
        hash.finalize().into()
    }
}

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'_' | b'$'))
        && bytes.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$'))
}

fn global(id: &IdentifierCallee<'_>, name: &str) -> bool {
    id.binding().is_none() && id.name() == name
}

impl Pattern {
    fn parse(source: &str) -> eyre::Result<Self> {
        ensure!(
            source.len() <= MAX_PATTERN_BYTES,
            "pattern exceeds {MAX_PATTERN_BYTES} bytes"
        );
        let (callee, arguments) = source
            .trim()
            .split_once('(')
            .ok_or_else(|| eyre::eyre!("expected callee(...)"))?;
        ensure!(
            arguments
                .strip_suffix(')')
                .is_some_and(|a| a.trim() == "..."),
            "only the literal (...) argument wildcard is supported"
        );
        let parts: Vec<_> = callee.split('.').map(str::trim).collect();
        let (canonical, shape) = match parts.as_slice() {
            [name] if identifier(name) => (
                format!("{name}(...)"),
                PatternShape::Global((*name).to_owned()),
            ),
            ["*", property] if identifier(property) => (
                format!("*.{property}(...)"),
                PatternShape::AnyMember((*property).to_owned()),
            ),
            [receiver, property] if identifier(receiver) && identifier(property) => (
                format!("{receiver}.{property}(...)"),
                PatternShape::GlobalMember {
                    receiver: (*receiver).to_owned(),
                    property: (*property).to_owned(),
                },
            ),
            _ => eyre::bail!("expected ASCII name(...), global.member(...), or *.member(...)"),
        };
        Ok(Self { canonical, shape })
    }

    fn matches(&self, call: Invocation<'_>) -> bool {
        match (&self.shape, &call.shape()) {
            (PatternShape::Global(name), CalleeShape::Identifier(id)) => global(&id, name),
            (
                PatternShape::AnyMember(name),
                CalleeShape::Member {
                    property: Some(property),
                    ..
                },
            ) => *name == *property,
            (
                PatternShape::GlobalMember {
                    receiver,
                    property: name,
                },
                CalleeShape::Member {
                    receiver_identifier: Some(id),
                    property: Some(property),
                    ..
                },
            ) => global(&id, receiver) && *name == *property,
            _ => false,
        }
    }
}
