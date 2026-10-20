//! Extract CSS without flattening import conditions, layers or cycles.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use cssparser::{BasicParseErrorKind, ParseError, Parser, ParserInput, Token};
use eyre::{ensure, WrapErr};
use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};
use sha2::{Digest, Sha256};

use crate::{link::Artifacts, resource};

struct Url {
    span: Range<usize>,
    value: String,
    import: bool,
    function: bool,
}

struct Request {
    url: Url,
    path: PathBuf,
    suffix: String,
}

pub(crate) struct Styles<'a> {
    root: &'a Path,
    inventory: &'a BTreeMap<PathBuf, Option<[u8; 32]>>,
    inputs: BTreeMap<PathBuf, Vec<u8>>,
    requests: BTreeMap<PathBuf, Vec<Request>>,
    compositions: BTreeMap<PathBuf, Vec<(crate::css_syntax::Name, PathBuf)>>,
    public_base: &'a crate::output_url::PublicBase,
    selection: &'a crate::build::CssModules,
    names: BTreeMap<PathBuf, crate::css_names::PreparedNames>,
    name_registry: crate::css_names::Registry,
}

impl<'a> Styles<'a> {
    pub(crate) fn new(
        root: &'a Path,
        inventory: &'a BTreeMap<PathBuf, Option<[u8; 32]>>,
        public_base: &'a crate::output_url::PublicBase,
        selection: &'a crate::build::CssModules,
    ) -> Self {
        Self {
            root,
            inventory,
            inputs: BTreeMap::new(),
            requests: BTreeMap::new(),
            compositions: BTreeMap::new(),
            public_base,
            selection,
            names: BTreeMap::new(),
            name_registry: crate::css_names::Registry::default(),
        }
    }

    pub(crate) fn discover(
        &mut self,
        path: &Path,
        input: &[u8],
        resolver: &impl DependencyResolver,
    ) -> eyre::Result<()> {
        if let Some(previous) = self.inputs.get(path) {
            ensure!(
                previous == input,
                "CSS dependency changed during build: {}",
                path.display()
            );
        } else {
            self.inputs.insert(path.to_owned(), input.to_vec());
        }
        let mut queue = VecDeque::from([path.to_owned()]);
        while let Some(path) = queue.pop_front() {
            if self.requests.contains_key(&path) {
                continue;
            }
            let text = std::str::from_utf8(&self.inputs[&path])
                .wrap_err_with(|| format!("{}: CSS is not UTF-8", path.display()))?;
            let urls = urls(text).wrap_err_with(|| format!("{}: invalid CSS", path.display()))?;
            let mut compositions = Vec::new();
            if matches!(
                resource::stylesheet_kind(&path, self.selection),
                resource::ResourceKind::Stylesheet { module: true }
            ) {
                for composition in crate::css_syntax::parse(&path, text)?.compositions {
                    let crate::css_syntax::CompositionSource::File(request) = composition.source
                    else {
                        continue;
                    };
                    let context = || {
                        format!(
                            "{}: CSS Modules composes request {:?} at {:?}, declaration {:?}",
                            path.display(),
                            request.decoded,
                            request.span,
                            composition.span
                        )
                    };
                    ensure!(
                        !request.decoded.contains(['\\', '\0']),
                        "{}: nonportable request",
                        context()
                    );
                    let resolved = resolver
                        .resolve_dependency(&path, &request.decoded, ResolutionMode::Import)
                        .wrap_err_with(context)?;
                    let ResolvedDependency::File(target) = resolved else {
                        eyre::bail!(
                            "{}: composition target must be a local CSS module",
                            context()
                        );
                    };
                    let target = target.canonicalize().wrap_err_with(context)?;
                    ensure!(
                        target.starts_with(self.root)
                            && self.inventory.contains_key(target.strip_prefix(self.root)?),
                        "{}: composition target outside bounded source inventory",
                        context()
                    );
                    ensure!(
                        matches!(
                            resource::kind(&target, self.selection),
                            resource::ResourceKind::Stylesheet { module: true }
                        ),
                        "{}: non-module composition target {}",
                        context(),
                        target.display()
                    );
                    if !self.inputs.contains_key(&target) {
                        self.inputs.insert(target.clone(), fs::read(&target)?);
                    }
                    queue.push_back(target.clone());
                    compositions.push((request, target));
                }
            }
            self.compositions.insert(path.clone(), compositions);
            let mut requests = Vec::new();
            for url in urls {
                let value = url
                    .value
                    .trim_matches(|ch| ch <= '\u{20}')
                    .replace(['\t', '\r', '\n'], "");
                let value = value.as_str();
                let scheme = value.find(':').is_some_and(|end| {
                    value.as_bytes()[0].is_ascii_alphabetic()
                        && value[..end].bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
                        })
                });
                if value.is_empty() || value.starts_with(['/', '#']) || scheme {
                    continue;
                }
                let end = value.find(['?', '#']).unwrap_or(value.len());
                let suffix = value[end..].to_owned();
                if end == 0 {
                    continue;
                }
                let relative = decode_path(&value[..end])?;
                ensure!(
                    !relative.contains('\\'),
                    "{}: nonportable CSS URL",
                    path.display()
                );
                let target = path
                    .parent()
                    .expect("CSS parent")
                    .join(relative)
                    .canonicalize()
                    .wrap_err_with(|| format!("{}: CSS URL {:?}", path.display(), url.value))?;
                ensure!(
                    target.starts_with(self.root)
                        && self.inventory.contains_key(target.strip_prefix(self.root)?),
                    "{}: CSS URL outside bounded source inventory: {:?}",
                    path.display(),
                    url.value
                );
                if !self.inputs.contains_key(&target) {
                    self.inputs.insert(target.clone(), fs::read(&target)?);
                }
                if url.import {
                    queue.push_back(target.clone());
                }
                requests.push(Request {
                    url,
                    path: target,
                    suffix,
                });
            }
            self.requests.insert(path, requests);
        }
        self.solve_compositions()?;
        Ok(())
    }

    fn solve_compositions(&mut self) -> eyre::Result<()> {
        for root in self.compositions.keys() {
            if self.names.contains_key(root) {
                continue;
            }
            let mut stack = vec![(root.clone(), 0)];
            let mut active = BTreeSet::from([root.clone()]);
            while let Some((path, index)) = stack.last_mut() {
                if let Some((request, next)) = self.compositions[path].get(*index) {
                    *index += 1;
                    if self.names.contains_key(next) {
                        continue;
                    }
                    let from = path.clone();
                    ensure!(
                        active.insert(next.clone()),
                        "{}: cross-file composition cycle {} -> {} at {:?}",
                        from.display(),
                        stack
                            .iter()
                            .map(|(path, _)| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join(" -> "),
                        next.display(),
                        request.span
                    );
                    stack.push((next.clone(), 0));
                    continue;
                }
                if matches!(
                    resource::stylesheet_kind(path, self.selection),
                    resource::ResourceKind::Stylesheet { module: true }
                ) {
                    let text = std::str::from_utf8(&self.inputs[path])?;
                    let mut names = self.name_registry.prepare_linked(self.root, path, text, &|request, name| {
                        let target = &self.compositions[path].iter()
                            .find(|(original, _)| original.decoded == request.decoded)
                            .expect("discovered composition request").1;
                        self.names[target].locals.get(&name.decoded).cloned().ok_or_else(|| eyre::eyre!(
                            "missing/private exported local mapping {:?} at {:?} from {} (request {:?} at {:?})",
                            name.decoded, name.span, target.display(), request.decoded, request.span
                        ))
                    })?;
                    self.requests
                        .get_mut(path)
                        .expect("discovered CSS")
                        .retain(|request| {
                            !names.edits.iter().any(|edit| {
                                edit.span.start <= request.url.span.start
                                    && request.url.span.end <= edit.span.end
                            })
                        });
                    for url in urls(text)? {
                        if !url.import
                            && url.function
                            && !names.edits.iter().any(|edit| {
                                edit.span.start <= url.span.start && url.span.end <= edit.span.end
                            })
                        {
                            let value = url
                                .value
                                .trim_matches(|ch| ch <= '\u{20}')
                                .replace(['\t', '\r', '\n'], "");
                            if let Some(scoped) =
                                value.strip_prefix('#').and_then(|id| names.ids.get(id))
                            {
                                names.edits.push(crate::css_syntax::Edit {
                                    span: url.span,
                                    replacement: format!("url(\"#{scoped}\")"),
                                });
                            }
                        }
                    }
                    self.names.insert(path.clone(), names);
                }
                active.remove(path);
                stack.pop();
            }
        }
        Ok(())
    }

    pub(crate) fn evaluation_dependencies(&self, path: &Path) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.compositions[path]
            .iter()
            .filter_map(|(request, target)| seen.insert(target).then_some(request.decoded.as_str()))
            .collect()
    }

    pub(crate) fn identity(&self, path: &Path) -> PathBuf {
        self.output_path(path)
    }

    pub(crate) fn public_url(&self, path: &Path) -> eyre::Result<String> {
        self.public_base.join(&self.output_path(path))
    }

    pub(crate) fn name_mappings(&self, path: &Path) -> Option<&BTreeMap<String, String>> {
        self.names.get(path).map(|names| &names.mappings)
    }

    pub(crate) fn identities(&self) -> BTreeMap<&Path, PathBuf> {
        self.requests
            .keys()
            .map(|path| (path.as_path(), self.output_path(path)))
            .collect()
    }

    fn closure(&self, root: &Path) -> BTreeSet<(PathBuf, bool)> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([(root.to_owned(), true)]);
        while let Some((path, style)) = queue.pop_front() {
            if !seen.insert((path.clone(), style)) || !style {
                continue;
            }
            queue.extend(
                self.requests[&path]
                    .iter()
                    .map(|request| (request.path.clone(), request.url.import)),
            );
            queue.extend(
                self.compositions[&path]
                    .iter()
                    .map(|(_, target)| (target.clone(), true)),
            );
        }
        seen
    }

    fn output_path(&self, path: &Path) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(b"css-resource-v1\0");
        let root = path
            .strip_prefix(self.root)
            .expect("bounded CSS")
            .as_os_str()
            .as_encoded_bytes();
        digest.update((root.len() as u64).to_le_bytes());
        digest.update(root);
        for (path, style) in self.closure(path) {
            let relative = path
                .strip_prefix(self.root)
                .expect("bounded CSS dependency")
                .as_os_str()
                .as_encoded_bytes();
            let input = &self.inputs[&path];
            digest.update([u8::from(style)]);
            digest.update((relative.len() as u64).to_le_bytes());
            digest.update(relative);
            digest.update((input.len() as u64).to_le_bytes());
            digest.update(input);
            if style && self.names.contains_key(&path) {
                digest.update(crate::css_names::REVISION.as_bytes());
                digest.update(crate::css_names::TRANSFORM_REVISION.as_bytes());
                digest.update(crate::css_syntax::PARSER_REVISION.as_bytes());
                for (request, target) in &self.compositions[&path] {
                    let target = target
                        .strip_prefix(self.root)
                        .expect("bounded composition")
                        .as_os_str()
                        .as_encoded_bytes();
                    digest.update(b"composition\0");
                    digest.update((request.decoded.len() as u64).to_le_bytes());
                    digest.update(request.decoded.as_bytes());
                    digest.update((target.len() as u64).to_le_bytes());
                    digest.update(target);
                }
            }
        }
        PathBuf::from(format!("assets/{:x}.css", digest.finalize()))
    }

    fn render(&self, path: &Path) -> eyre::Result<Vec<u8>> {
        let text = std::str::from_utf8(&self.inputs[path])?;
        let stylesheet = self.output_path(path);
        let mut edits = self
            .names
            .get(path)
            .map_or_else(Vec::new, |names| names.edits.clone());
        for request in &self.requests[path] {
            let target = if request.url.import {
                self.output_path(&request.path)
            } else {
                resource::asset_path(&request.path, &self.inputs[&request.path])
            };
            let url = format!(
                "{}{}",
                self.public_base.css_fetch(&stylesheet, &target)?,
                request.suffix
            );
            let mut replacement = String::new();
            if request.url.function {
                replacement.push_str("url(");
            }
            cssparser::serialize_string(&url, &mut replacement)?;
            if request.url.function {
                replacement.push(')');
            }
            edits.push(crate::css_syntax::Edit {
                span: request.url.span.clone(),
                replacement,
            });
        }
        Ok(crate::css_syntax::apply_edits(text, edits)?.into_bytes())
    }

    pub(crate) fn emit<C>(&self, root: &Path, artifacts: &mut Artifacts<C>) -> eyre::Result<()> {
        artifacts
            .stylesheets
            .insert(root.to_owned(), self.output_path(root));
        for (path, style) in self.closure(root) {
            let (output, bytes) = if style {
                (self.output_path(&path), self.render(&path)?)
            } else {
                (
                    resource::asset_path(&path, &self.inputs[&path]),
                    self.inputs[&path].clone(),
                )
            };
            artifacts.resources.insert(output, bytes);
        }
        Ok(())
    }

    pub(crate) fn recheck(&self) -> eyre::Result<()> {
        for (path, input) in &self.inputs {
            ensure!(
                fs::read(path)? == *input,
                "CSS dependency changed during build: {}",
                path.display()
            );
        }
        Ok(())
    }
}

fn decode_path(value: &str) -> eyre::Result<String> {
    let mut bytes = value.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            let low = bytes.next().and_then(|byte| (byte as char).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                eyre::bail!("invalid percent-encoded CSS URL: {value:?}");
            };
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    String::from_utf8(decoded).wrap_err("CSS URL is not UTF-8")
}

fn urls(source: &str) -> eyre::Result<Vec<Url>> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut urls = Vec::new();
    scan(&mut parser, &mut urls, false, 0)
        .map_err(|error| eyre::eyre!("CSS token error: {error:?}"))?;
    Ok(urls)
}

fn scan<'i>(
    parser: &mut Parser<'i, '_>,
    urls: &mut Vec<Url>,
    string_urls: bool,
    depth: usize,
) -> Result<(), ParseError<'i, ()>> {
    if depth > 128 {
        return Err(parser.new_custom_error(()));
    }
    let mut import = false;
    let mut identifier_urls = false;
    loop {
        let start = parser.position().byte_index();
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(error) if matches!(error.kind, BasicParseErrorKind::EndOfInput) => break,
            Err(error) => return Err(error.into()),
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::AtKeyword(name) => {
                import = name.eq_ignore_ascii_case("import");
                identifier_urls = true;
                continue;
            }
            Token::Semicolon => identifier_urls = false,
            Token::QuotedString(value) if import || (string_urls && !identifier_urls) => {
                urls.push(Url {
                    span: start..parser.position().byte_index(),
                    value: value.to_string(),
                    import,
                    function: false,
                })
            }
            Token::UnquotedUrl(value) if import || !identifier_urls => urls.push(Url {
                span: start..parser.position().byte_index(),
                value: value.to_string(),
                import,
                function: true,
            }),
            Token::Function(name)
                if name.eq_ignore_ascii_case("url") && (import || !identifier_urls) =>
            {
                let value = parser.parse_nested_block(|parser| {
                    let value = parser.expect_string_cloned()?;
                    parser.expect_exhausted()?;
                    Ok::<_, ParseError<'i, ()>>(value.to_string())
                })?;
                urls.push(Url {
                    span: start..parser.position().byte_index(),
                    value,
                    import,
                    function: true,
                });
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock
                if identifier_urls => {}
            Token::Function(name) => {
                let strings = name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set");
                parser.parse_nested_block(|parser| scan(parser, urls, strings, depth + 1))?;
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                parser.parse_nested_block(|parser| scan(parser, urls, false, depth + 1))?;
                identifier_urls = false;
            }
            Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket => {
                return Err(parser.new_custom_error(()));
            }
            _ => {}
        }
        import = false;
    }
    Ok(())
}
