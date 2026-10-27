//! Source-local extraction and borrowed stylesheet syntax; linking stays derived.
use std::{
    fs,
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};

use cache_flatbuffers::{hash128, style_wire as wire, SharedBytes, WireStyle};
use eyre::WrapErr;
use segment_cache::CacheMode;

use crate::css_syntax as owned;

#[cfg(test)]
thread_local! {
    static EXTRACTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "css_facts_tests.rs"]
mod tests;

pub(crate) fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".style.seg.fb");
    name.into()
}

fn config(module: bool) -> [u8; 16] {
    let mut bytes = Vec::from(include_bytes!("css_syntax.rs").as_slice());
    bytes.extend_from_slice(include_bytes!("loaders/stylesheet.rs"));
    bytes.extend_from_slice(owned::PARSER_REVISION.as_bytes());
    bytes.push(u8::from(module));
    hash128(&bytes)
}

pub(crate) fn load(
    path: &Path,
    input: &[u8],
    source: &str,
    module: bool,
    mode: CacheMode,
) -> eyre::Result<(WireStyle, Option<String>)> {
    let config = config(module);
    let cache = sidecar(path);
    if mode != CacheMode::Disabled {
        if let Some(record) = fs::File::open(&cache).ok().and_then(|file| {
            SharedBytes::snapshot(file)
                .ok()
                .and_then(|bytes| WireStyle::from_backing(bytes, input, source, config).ok())
        }) {
            crate::prepared_text::InputIdentity::new(input).read(path)?;
            return Ok((record, None));
        }
    }
    let record = extract(path, input, source, module, config)?;
    let warning = if mode == CacheMode::ReadWrite {
        let write = || -> eyre::Result<()> {
            crate::prepared_text::InputIdentity::new(input).read(path)?;
            let parent = cache.parent().expect("stylesheet cache parent");
            let prefix = format!(
                "{}.stage-",
                cache.file_name().expect("cache name").to_string_lossy()
            );
            let mut staging = tempfile::Builder::new()
                .prefix(&prefix)
                .tempfile_in(parent)
                .wrap_err_with(|| {
                    format!("creating stylesheet facts staging file {}", cache.display())
                })?;
            staging
                .write_all(record.backing())
                .wrap_err_with(|| format!("writing stylesheet facts {}", cache.display()))?;
            staging
                .flush()
                .wrap_err_with(|| format!("flushing stylesheet facts {}", cache.display()))?;
            staging
                .as_file()
                .sync_all()
                .wrap_err_with(|| format!("syncing stylesheet facts {}", cache.display()))?;
            let staged = staging.into_temp_path();
            fs::rename(&staged, &cache)
                .wrap_err_with(|| format!("publishing stylesheet facts {}", cache.display()))?;
            Ok(())
        };
        write().err().map(|error| format!("{error:#}"))
    } else {
        None
    };
    crate::prepared_text::InputIdentity::new(input).read(path)?;
    Ok((record, warning))
}

fn span(value: Range<usize>) -> wire::SpanT {
    wire::SpanT {
        start: value.start as u64,
        end: value.end as u64,
    }
}

fn name(value: owned::Name) -> wire::NameT {
    let mut result = wire::NameT::default();
    result.decoded = value.decoded;
    result.span = span(value.span);
    result
}

pub(crate) fn extract(
    path: &Path,
    input: &[u8],
    source: &str,
    module: bool,
    config: [u8; 16],
) -> eyre::Result<WireStyle> {
    #[cfg(test)]
    EXTRACTIONS.with(|count| count.set(count.get() + 1));
    let syntax = if module {
        owned::parse(path, source)?
    } else {
        owned::ModuleSyntax::default()
    };
    let mut facts = wire::FactsT::default();
    facts.urls = crate::css::urls(source)?
        .into_iter()
        .map(|url| {
            let mut result = wire::UrlT::default();
            result.span = span(url.span);
            result.value = url.value;
            result.import = url.import;
            result.function = url.function;
            result
        })
        .collect();
    facts.names = syntax
        .names
        .into_iter()
        .map(|item| {
            let mut result = wire::SelectorNameT::default();
            result.kind = match item.kind {
                owned::SelectorKind::Class => wire::SelectorKind::Class,
                owned::SelectorKind::Id => wire::SelectorKind::Id,
            };
            result.name = Box::new(name(item.name));
            result.local = item.local;
            result
        })
        .collect();
    facts.scopes = syntax
        .scopes
        .into_iter()
        .map(|item| {
            let mut result = wire::ScopeT::default();
            result.kind = match item.kind {
                owned::ScopeKind::Local => wire::ScopeKind::Local,
                owned::ScopeKind::Global => wire::ScopeKind::Global,
            };
            result.span = span(item.span);
            result.argument = item.argument.map(span);
            result.owner = span(item.owner);
            result.alternatives = item.alternatives.into_iter().map(span).collect();
            result.commas = item.commas.into_iter().map(span).collect();
            result.first_in_compound = item.first_in_compound;
            result
        })
        .collect();
    facts.compositions = syntax
        .compositions
        .into_iter()
        .map(|item| {
            let mut result = wire::CompositionT::default();
            result.span = span(item.span);
            result.selector = item.selector.map(span);
            result.nested = item.nested;
            result.names = item.names.into_iter().map(name).collect();
            (result.kind, result.request) = match item.source {
                owned::CompositionSource::Local => (wire::CompositionKind::Local, None),
                owned::CompositionSource::Global => (wire::CompositionKind::Global, None),
                owned::CompositionSource::File(request) => {
                    (wire::CompositionKind::File, Some(Box::new(name(request))))
                }
            };
            result
        })
        .collect();
    facts.selectors = syntax.selectors.into_iter().map(span).collect();
    facts.animations = syntax
        .animations
        .into_iter()
        .map(|item| {
            let mut result = wire::AnimationT::default();
            result.name = Box::new(name(item.name));
            result.span = span(item.span);
            result.local = match item.local {
                None => wire::Locality::Implicit,
                Some(true) => wire::Locality::Local,
                Some(false) => wire::Locality::Global,
            };
            result.definition = item.definition;
            result
        })
        .collect();
    WireStyle::encode(input, source, config, facts)
        .wrap_err_with(|| format!("encoding stylesheet facts {}", path.display()))
}

fn range(span: &wire::Span) -> Range<usize> {
    span.start() as usize..span.end() as usize
}

#[derive(Clone, Debug)]
pub(crate) struct Name<'a> {
    pub(crate) decoded: &'a str,
    pub(crate) span: Range<usize>,
}

impl<'a> From<wire::Name<'a>> for Name<'a> {
    fn from(value: wire::Name<'a>) -> Self {
        Self {
            decoded: value.decoded(),
            span: range(value.span()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Owner<'a> {
    Facts(wire::Facts<'a>),
    Scope(wire::Scope<'a>),
    Composition(wire::Composition<'a>),
}

#[derive(Clone, Debug)]
pub(crate) struct List<'a, T> {
    owner: Owner<'a>,
    len: usize,
    get: fn(Owner<'a>, usize) -> T,
}

pub(crate) struct Iter<'b, 'a, T> {
    list: &'b List<'a, T>,
    range: Range<usize>,
}

impl<T> Iterator for Iter<'_, '_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.range
            .next()
            .map(|index| (self.list.get)(self.list.owner, index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}
impl<T> DoubleEndedIterator for Iter<'_, '_, T> {
    fn next_back(&mut self) -> Option<T> {
        self.range
            .next_back()
            .map(|index| (self.list.get)(self.list.owner, index))
    }
}
impl<T> ExactSizeIterator for Iter<'_, '_, T> {}

impl<'a, T> List<'a, T> {
    pub(crate) fn iter(&self) -> Iter<'_, 'a, T> {
        Iter {
            list: self,
            range: 0..self.len,
        }
    }
    pub(crate) fn get(&self, index: usize) -> Option<T> {
        (index < self.len).then(|| (self.get)(self.owner, index))
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<'a, 'b, T> IntoIterator for &'b List<'a, T> {
    type Item = T;
    type IntoIter = Iter<'b, 'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug)]
pub(crate) struct SelectorName<'a> {
    pub(crate) kind: owned::SelectorKind,
    pub(crate) name: Name<'a>,
    pub(crate) local: bool,
}
#[derive(Debug)]
pub(crate) struct Scope<'a> {
    pub(crate) span: Range<usize>,
    pub(crate) argument: Option<Range<usize>>,
    pub(crate) owner: Range<usize>,
    pub(crate) alternatives: List<'a, Range<usize>>,
    pub(crate) commas: List<'a, Range<usize>>,
    pub(crate) first_in_compound: bool,
}
#[derive(Debug)]
pub(crate) enum CompositionSource<'a> {
    Local,
    Global,
    File(Name<'a>),
}
pub(crate) struct Composition<'a> {
    pub(crate) span: Range<usize>,
    pub(crate) selector: Option<Range<usize>>,
    pub(crate) nested: bool,
    pub(crate) names: List<'a, Name<'a>>,
    pub(crate) source: CompositionSource<'a>,
}
pub(crate) struct Animation<'a> {
    pub(crate) name: Name<'a>,
    pub(crate) span: Range<usize>,
    pub(crate) local: Option<bool>,
    pub(crate) definition: bool,
}
pub(crate) struct Syntax<'a> {
    pub(crate) names: List<'a, SelectorName<'a>>,
    pub(crate) scopes: List<'a, Scope<'a>>,
    pub(crate) compositions: List<'a, Composition<'a>>,
    pub(crate) selectors: List<'a, Range<usize>>,
    pub(crate) animations: List<'a, Animation<'a>>,
}

impl<'a> Syntax<'a> {
    pub(crate) fn new(facts: wire::Facts<'a>) -> Self {
        let owner = Owner::Facts(facts);
        Self {
            names: List {
                owner,
                len: facts.names().len(),
                get: |owner, index| {
                    let Owner::Facts(facts) = owner else {
                        unreachable!()
                    };
                    let item = facts.names().get(index);
                    SelectorName {
                        kind: if item.kind() == wire::SelectorKind::Class {
                            owned::SelectorKind::Class
                        } else {
                            owned::SelectorKind::Id
                        },
                        name: item.name().into(),
                        local: item.local(),
                    }
                },
            },
            scopes: List {
                owner,
                len: facts.scopes().len(),
                get: |owner, index| {
                    let Owner::Facts(facts) = owner else {
                        unreachable!()
                    };
                    let item = facts.scopes().get(index);
                    Scope {
                        span: range(item.span()),
                        argument: item.argument().map(range),
                        owner: range(item.owner()),
                        alternatives: List {
                            owner: Owner::Scope(item),
                            len: item.alternatives().len(),
                            get: |owner, index| {
                                let Owner::Scope(item) = owner else {
                                    unreachable!()
                                };
                                range(item.alternatives().get(index))
                            },
                        },
                        commas: List {
                            owner: Owner::Scope(item),
                            len: item.commas().len(),
                            get: |owner, index| {
                                let Owner::Scope(item) = owner else {
                                    unreachable!()
                                };
                                range(item.commas().get(index))
                            },
                        },
                        first_in_compound: item.first_in_compound(),
                    }
                },
            },
            compositions: List {
                owner,
                len: facts.compositions().len(),
                get: |owner, index| {
                    let Owner::Facts(facts) = owner else {
                        unreachable!()
                    };
                    let item = facts.compositions().get(index);
                    Composition {
                        span: range(item.span()),
                        selector: item.selector().map(range),
                        nested: item.nested(),
                        names: List {
                            owner: Owner::Composition(item),
                            len: item.names().len(),
                            get: |owner, index| {
                                let Owner::Composition(item) = owner else {
                                    unreachable!()
                                };
                                item.names().get(index).into()
                            },
                        },
                        source: match item.kind() {
                            wire::CompositionKind::Local => CompositionSource::Local,
                            wire::CompositionKind::Global => CompositionSource::Global,
                            _ => CompositionSource::File(
                                item.request().expect("checked file composition").into(),
                            ),
                        },
                    }
                },
            },
            selectors: List {
                owner,
                len: facts.selectors().len(),
                get: |owner, index| {
                    let Owner::Facts(facts) = owner else {
                        unreachable!()
                    };
                    range(facts.selectors().get(index))
                },
            },
            animations: List {
                owner,
                len: facts.animations().len(),
                get: |owner, index| {
                    let Owner::Facts(facts) = owner else {
                        unreachable!()
                    };
                    let item = facts.animations().get(index);
                    Animation {
                        name: item.name().into(),
                        span: range(item.span()),
                        local: match item.local() {
                            wire::Locality::Local => Some(true),
                            wire::Locality::Global => Some(false),
                            _ => None,
                        },
                        definition: item.definition(),
                    }
                },
            },
        }
    }
}
