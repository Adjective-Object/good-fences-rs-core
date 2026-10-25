//! Interim source fragments keep generated requests typed until artifact naming.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use eyre::{ensure, OptionExt};
use oxc_span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    Import,
    ReExport,
    DynamicImport,
    Require,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Source(Arc<crate::sourcemap::Source>, Span),
    Named(String, Arc<crate::sourcemap::Source>, u32, String),
    Request(PathBuf, RequestKind),
    External(String, RequestKind),
    Evaluation(PathBuf),
    BuildRootUrl(String),
    ResourceUrl(PathBuf, String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Code {
    parts: Vec<Part>,
    pub default_export: bool,
}

impl From<String> for Code {
    fn from(text: String) -> Self {
        Self {
            parts: vec![Part::Text(text)],
            ..Self::default()
        }
    }
}

impl From<&str> for Code {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl Code {
    pub(crate) fn resource_url(target: PathBuf, base: String) -> Self {
        Self {
            parts: vec![Part::ResourceUrl(target, base)],
            ..Self::default()
        }
    }

    pub(crate) fn build_root_url(public_url: String) -> Self {
        Self {
            parts: vec![Part::BuildRootUrl(public_url)],
            ..Self::default()
        }
    }

    pub fn external(specifier: String, kind: RequestKind) -> Self {
        Self {
            parts: vec![Part::External(specifier, kind)],
            ..Self::default()
        }
    }
    pub fn request(target: PathBuf, kind: RequestKind) -> Self {
        Self {
            parts: vec![Part::Request(target, kind)],
            ..Self::default()
        }
    }

    pub fn evaluation(target: PathBuf) -> Self {
        Self {
            parts: vec![Part::Evaluation(target)],
            ..Self::default()
        }
    }

    pub(crate) fn prune(
        files: &mut BTreeMap<PathBuf, Code>,
        candidates: &BTreeSet<PathBuf>,
        members: &mut BTreeMap<crate::chunks::NodeId, PathBuf>,
    ) -> eyre::Result<()> {
        for path in candidates {
            ensure!(files.contains_key(path), "missing planned empty chunk");
        }
        // Binding/namespace requests carry an identity, not merely evaluation.
        // External clauses likewise retain their native linking contract.
        let mut candidates = candidates.clone();
        for (path, code) in files.iter() {
            for part in &code.parts {
                match part {
                    Part::Request(target, _) | Part::ResourceUrl(target, _) => {
                        candidates.remove(target);
                    }
                    Part::External(_, _) => {
                        candidates.remove(path);
                    }
                    _ => {}
                }
            }
        }
        let paths: Vec<_> = files.keys().collect();
        let indices: BTreeMap<_, _> = paths
            .iter()
            .enumerate()
            .map(|(index, &path)| (path, index))
            .collect();
        let dependencies: Vec<BTreeSet<_>> = paths
            .iter()
            .map(|path| {
                files[*path]
                    .static_dependencies()
                    .filter_map(|(target, _)| indices.get(target).copied())
                    .collect()
            })
            .collect();
        let mut users = vec![Vec::new(); paths.len()];
        for (user, targets) in dependencies.iter().enumerate() {
            for &target in targets {
                users[target].push(user);
            }
        }
        let mut reaches_retained: Vec<_> = paths
            .iter()
            .map(|path| !candidates.contains(*path))
            .collect();
        let mut pending: Vec<_> = reaches_retained
            .iter()
            .enumerate()
            .filter_map(|(index, &retained)| retained.then_some(index))
            .collect();
        while let Some(target) = pending.pop() {
            for &user in &users[target] {
                if !reaches_retained[user] {
                    reaches_retained[user] = true;
                    pending.push(user);
                }
            }
        }
        for component in crate::effect_dependencies::components(&dependencies) {
            let cyclic = component.len() > 1 || dependencies[component[0]].contains(&component[0]);
            if component
                .iter()
                .any(|&index| !candidates.contains(paths[index]))
                || (cyclic && component.iter().any(|&index| reaches_retained[index]))
            {
                // An evaluating retained job is a barrier: replacing a revisit
                // with its connector's remaining requests could run them early.
                // Cyclic connectors also cache a descendant's failure together;
                // another entrance must not execute its remaining siblings.
                for index in component {
                    candidates.remove(paths[index]);
                }
            }
        }
        // Removed cycles have no reachable retained execution/identity jobs.
        // Acyclic connectors splice their outgoing obligations in request order.
        let empty: BTreeMap<_, _> = candidates
            .iter()
            .map(|path| {
                let code = files.get(path).ok_or_eyre("missing planned empty chunk")?;
                Ok((
                    path.clone(),
                    code.static_dependencies()
                        .map(|(target, _)| target.clone())
                        .collect::<Vec<_>>(),
                ))
            })
            .collect::<eyre::Result<_>>()?;
        let mut users = BTreeMap::<_, Vec<_>>::new();
        for (path, code) in files.iter() {
            for (target, _) in code.static_dependencies() {
                users.entry(target.clone()).or_default().push(path.clone());
            }
        }
        for (path, code) in files.iter_mut() {
            if !empty.contains_key(path) {
                code.splice(&empty)?;
            }
        }
        let mut replacements = BTreeMap::new();
        for path in empty.keys() {
            let mut pending = vec![path.clone()];
            let mut seen = BTreeSet::new();
            let mut owner = None;
            while let Some(candidate) = pending.pop() {
                if !seen.insert(candidate.clone()) {
                    continue;
                }
                if !empty.contains_key(&candidate) {
                    owner = Some(candidate);
                    break;
                }
                pending.extend(users.get(&candidate).into_iter().flatten().cloned());
            }
            if owner.is_none() {
                owner = expand(path, &empty).into_iter().next();
            }
            replacements.insert(
                path.clone(),
                owner.ok_or_eyre("empty private module has no retained physical consumer")?,
            );
        }
        for owner in members.values_mut() {
            if let Some(replacement) = replacements.get(owner) {
                *owner = replacement.clone();
            }
        }
        for path in empty.keys() {
            files.remove(path);
        }
        Ok(())
    }

    pub fn push(&mut self, code: impl Into<Self>) {
        let code = code.into();
        self.default_export |= code.default_export;
        self.parts.extend(code.parts);
    }

    pub fn push_str(&mut self, text: &str) {
        if !text.is_empty() {
            self.parts.push(Part::Text(text.to_owned()));
        }
    }

    pub fn surrounded(self, prefix: impl Into<String>, suffix: impl Into<String>) -> Self {
        let mut code = Self::from(prefix.into());
        code.push(self);
        code.push(suffix.into());
        code
    }

    pub fn prepend_after_hashbang(&mut self, header: Self) {
        let mut prefix = Self::default();
        if let Some(Part::Text(text)) = self.parts.first_mut() {
            if text.starts_with("#!") {
                let end = text.find('\n').map_or(text.len(), |index| index + 1);
                prefix.push(text[..end].to_owned());
                *text = text[end..].to_owned();
            }
        }
        if let Some(Part::Source(source, span)) = self.parts.first_mut() {
            let text = source.slice(*span);
            if text.starts_with("#!") {
                let end = text.find('\n').map_or(text.len(), |index| index + 1);
                prefix.parts.push(Part::Source(
                    source.clone(),
                    Span::new(span.start, span.start + end as u32),
                ));
                span.start += end as u32;
            }
        }
        prefix.push(header);
        prefix.push(std::mem::take(self));
        *self = prefix;
    }

    pub fn static_dependencies(&self) -> impl Iterator<Item = (&PathBuf, bool)> {
        self.parts.iter().filter_map(|part| match part {
            Part::Evaluation(target) => Some((target, false)),
            Part::Request(target, RequestKind::Import | RequestKind::ReExport) => {
                Some((target, true))
            }
            _ => None,
        })
    }

    pub fn splice(&mut self, empty: &BTreeMap<PathBuf, Vec<PathBuf>>) -> eyre::Result<()> {
        let mut parts = Vec::new();
        for part in std::mem::take(&mut self.parts) {
            match part {
                Part::Evaluation(target) if empty.contains_key(&target) => {
                    parts.extend(expand(&target, empty).into_iter().map(Part::Evaluation));
                }
                Part::Request(ref target, _) if empty.contains_key(target) => {
                    eyre::bail!("empty native chunk still supplies a binding or namespace");
                }
                part => parts.push(part),
            }
        }
        self.parts = parts;
        Ok(())
    }

    pub fn render(
        &self,
        original: &Path,
        destination: &Path,
        destinations: &BTreeMap<PathBuf, PathBuf>,
    ) -> eyre::Result<String> {
        Ok(self
            .render_with_map(original, destination, destinations, false)?
            .0)
    }

    pub(crate) fn render_with_map(
        &self,
        original: &Path,
        destination: &Path,
        destinations: &BTreeMap<PathBuf, PathBuf>,
        source_maps: bool,
    ) -> eyre::Result<(String, Option<oxc_sourcemap::SourceMap<'static>>)> {
        let mut output = crate::sourcemap::Writer::new(source_maps);
        for part in &self.parts {
            let (target, url, evaluation) = match part {
                Part::Text(text) => {
                    output.generated(text);
                    continue;
                }
                Part::Source(source, span) => {
                    output.source(source, *span);
                    continue;
                }
                Part::Named(text, source, offset, name) => {
                    output.named(text, source, *offset, name);
                    continue;
                }
                Part::External(specifier, _) => {
                    output.generated(&serde_json::to_string(specifier)?);
                    continue;
                }
                Part::BuildRootUrl(public_url) => {
                    let depth = destination
                        .parent()
                        .ok_or_eyre("missing output parent")?
                        .components()
                        .count();
                    let root = if depth == 0 {
                        "./".to_owned()
                    } else {
                        "../".repeat(depth)
                    };
                    output.generated(&format!(
                        "new URL({}, new URL({}, import.meta.url))",
                        serde_json::to_string(public_url)?,
                        serde_json::to_string(&root)?
                    ));
                    continue;
                }
                Part::ResourceUrl(target, base) => {
                    let target = destinations
                        .get(target)
                        .ok_or_eyre("resource URL target has no artifact")?;
                    let base = crate::output_url::PublicBase::new(base)?;
                    let mut request = base.join(target)?;
                    if base.relative() {
                        let depth = destination
                            .parent()
                            .ok_or_eyre("missing output parent")?
                            .components()
                            .count();
                        request = format!("{}{request}", "../".repeat(depth));
                    }
                    output.generated(&serde_json::to_string(&request)?);
                    continue;
                }
                Part::Request(target, kind) => (target, *kind != RequestKind::Require, false),
                Part::Evaluation(target) => (target, true, true),
            };
            let target = destinations.get(target).ok_or_else(|| {
                eyre::eyre!(
                    "generated request in {} has no artifact: {}",
                    original.display(),
                    target.display()
                )
            })?;
            let request = serde_json::to_string(
                &relative_request(destination, target, url)
                    .ok_or_eyre("artifact destinations cannot form a relative request")?,
            )?;
            if evaluation {
                output.generated("import ");
                output.generated(&request);
                output.generated(";\n");
            } else {
                output.generated(&request);
            }
        }
        Ok(output.finish(destination))
    }

    pub(crate) fn name_origin(
        &mut self,
        source: &Arc<crate::sourcemap::Source>,
        offset: u32,
        name: &str,
    ) {
        for part in &mut self.parts {
            if let Part::Text(text) = part {
                *part = Part::Named(
                    std::mem::take(text),
                    source.clone(),
                    offset,
                    name.to_owned(),
                );
            }
        }
    }
}

pub(crate) fn expand(path: &PathBuf, empty: &BTreeMap<PathBuf, Vec<PathBuf>>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut pending = vec![path];
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path) {
            continue;
        }
        if let Some(targets) = empty.get(path) {
            pending.extend(targets.iter().rev());
        } else {
            result.push(path.clone());
        }
    }
    result
}

#[derive(Debug, Clone)]
pub(crate) struct Edit {
    pub span: Span,
    pub replacement: Code,
}

pub(crate) fn apply(source: &str, edits: &[Edit]) -> eyre::Result<Code> {
    apply_source_range(source, Span::new(0, source.len() as u32), edits, None)
}

#[cfg(test)]
pub(crate) fn apply_range(source: &str, range: Span, edits: &[Edit]) -> eyre::Result<Code> {
    apply_source_range(source, range, edits, None)
}

pub(crate) fn apply_source_range(
    source: &str,
    range: Span,
    edits: &[Edit],
    mapping: Option<&Arc<crate::sourcemap::Source>>,
) -> eyre::Result<Code> {
    let text = source
        .get(range.start as usize..range.end as usize)
        .ok_or_eyre("invalid UTF-8 source slot")?;
    let mut edits: Vec<_> = edits.iter().collect();
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    let mut end = range.start as usize;
    let mut previous = None;
    let mut output = Code::default();
    let copied = |output: &mut Code, start: usize, end: usize| {
        if start < end {
            if let Some(mapping) = mapping {
                output.parts.push(Part::Source(
                    mapping.clone(),
                    Span::new(start as u32, end as u32),
                ));
            } else {
                output.push_str(&source[start..end]);
            }
        }
    };
    for edit in edits {
        let start = edit.span.start as usize;
        let next = edit.span.end as usize;
        ensure!(
            start <= next
                && start >= range.start as usize
                && next <= range.end as usize
                && source.is_char_boundary(start)
                && source.is_char_boundary(next),
            "invalid UTF-8 edit range {start}..{next}"
        );
        ensure!(
            start >= end && previous != Some(start),
            "overlapping edits at bytes {start}..{next}"
        );
        copied(&mut output, end, start);
        output.push(edit.replacement.clone());
        end = next;
        previous = Some(start);
    }
    copied(&mut output, end, range.start as usize + text.len());
    Ok(output)
}

pub(crate) fn relative_request(importer: &Path, target: &Path, url: bool) -> Option<String> {
    let parent: Vec<_> = importer.parent()?.components().collect();
    let target: Vec<_> = target.components().collect();
    let shared = parent
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    // Source candidates can be on another Windows volume/share. Neither a
    // prefix nor a root can be crossed by a relative module request.
    if parent[shared..]
        .iter()
        .chain(&target[shared..])
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let mut request = if parent.len() == shared {
        "./".to_owned()
    } else {
        "../".repeat(parent.len() - shared)
    };
    request.push_str(
        &target[shared..]
            .iter()
            .map(|part| match part {
                Component::Normal(value) => value.to_str(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?
            .join("/"),
    );
    if !url {
        return Some(request);
    }
    Some(
        request
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"/.-_~".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_hashbang_and_late_request_lengths_preserve_following_source_positions() {
        let text = "#!/usr/bin/env node\nconst before=import('./dep.mjs'); console.log('AFTER');";
        let source = crate::sourcemap::Source::new(
            &std::env::current_dir().unwrap().join("input.mjs"),
            text,
        )
        .unwrap();
        let start = text.find("'./dep.mjs'").unwrap() as u32;
        let mut code = apply_source_range(
            text,
            Span::new(0, text.len() as u32),
            &[Edit {
                span: Span::new(start, start + "'./dep.mjs'".len() as u32),
                replacement: Code::request("dep.mjs".into(), RequestKind::DynamicImport),
            }],
            Some(&source),
        )
        .unwrap();
        code.prepend_after_hashbang("const helper = 1;\n".into());
        let destinations = BTreeMap::from([("dep.mjs".into(), "some/longer/renamed #.mjs".into())]);
        let (output, map) = code
            .render_with_map(
                Path::new("entry.mjs"),
                Path::new("deep/start.mjs"),
                &destinations,
                true,
            )
            .unwrap();
        assert!(output.starts_with("#!/usr/bin/env node\nconst helper = 1;\n"));
        let output_line = output.lines().nth(2).unwrap();
        assert!(output_line.contains("../some/longer/renamed%20%23.mjs"));
        let map = map.unwrap();
        let lookup = map.generate_lookup_table();
        let token = map
            .lookup_source_view_token(&lookup, 2, output_line.find("'AFTER'").unwrap() as u32)
            .unwrap();
        assert_eq!(token.get_src_line(), 1);
        assert_eq!(
            token.get_src_col(),
            text.lines().nth(1).unwrap().find("'AFTER'").unwrap() as u32
        );
        assert!(map
            .lookup_source_view_token(&lookup, 1, 0)
            .unwrap()
            .get_source()
            .is_none());
    }

    #[test]
    fn requests_follow_destinations_without_rewriting_source_lookalikes() {
        let mut code = Code::from("const text = './dep.mjs'; import(");
        code.push(Code::request("dep.mjs".into(), RequestKind::DynamicImport));
        code.push_str("); require(");
        code.push(Code::request("dep.mjs".into(), RequestKind::Require));
        code.push_str(");");
        let destinations = BTreeMap::from([("dep.mjs".into(), "other/a #é.mjs".into())]);
        let output = code
            .render(
                Path::new("entry.mjs"),
                Path::new("app/start.mjs"),
                &destinations,
            )
            .unwrap();
        assert_eq!(
            output,
            "const text = './dep.mjs'; import(\"../other/a%20%23%C3%A9.mjs\"); require(\"../other/a #é.mjs\");"
        );
        assert!(code
            .render(
                Path::new("entry.mjs"),
                Path::new("entry.mjs"),
                &BTreeMap::new()
            )
            .is_err());
    }

    #[test]
    fn splicing_keeps_dependency_order_and_rejects_binding_targets() {
        let empty = BTreeMap::from([
            ("a.mjs".into(), vec!["b.mjs".into(), "c.mjs".into()]),
            ("b.mjs".into(), vec!["d.mjs".into()]),
        ]);
        let mut code = Code::evaluation("a.mjs".into());
        code.splice(&empty).unwrap();
        assert_eq!(
            code.static_dependencies().collect::<Vec<_>>(),
            vec![
                (&PathBuf::from("d.mjs"), false),
                (&PathBuf::from("c.mjs"), false)
            ]
        );
        assert!(Code::request("a.mjs".into(), RequestKind::Import)
            .splice(&empty)
            .is_err());
    }

    #[test]
    fn pruning_projects_members_without_creating_requests_to_deleted_files() {
        use crate::chunks::NodeId;
        let mut entry = Code::evaluation("empty.mjs".into());
        entry.push_str("console.log('entry');");
        let mut empty = Code::evaluation("first.mjs".into());
        empty.push(Code::evaluation("second.mjs".into()));
        let mut files = BTreeMap::from([
            ("entry.mjs".into(), entry),
            ("empty.mjs".into(), empty),
            ("first.mjs".into(), Code::from("console.log('first');")),
            ("second.mjs".into(), Code::from("console.log('second');")),
        ]);
        let node = NodeId::Module("source.mjs".into());
        let mut members = BTreeMap::from([(node.clone(), "empty.mjs".into())]);
        Code::prune(
            &mut files,
            &BTreeSet::from(["empty.mjs".into()]),
            &mut members,
        )
        .unwrap();
        assert_eq!(members[&node], Path::new("entry.mjs"));
        assert!(!files.contains_key(Path::new("empty.mjs")));
        let destinations = files.keys().map(|p| (p.clone(), p.clone())).collect();
        assert_eq!(
            files[Path::new("entry.mjs")]
                .render(
                    Path::new("entry.mjs"),
                    Path::new("entry.mjs"),
                    &destinations
                )
                .unwrap(),
            "import \"./first.mjs\";\nimport \"./second.mjs\";\nconsole.log('entry');"
        );
    }

    #[test]
    fn pruning_keeps_empty_sccs_with_ordered_outgoing_obligations() {
        let mut a = Code::evaluation("b.mjs".into());
        a.push(Code::evaluation("first.mjs".into()));
        let mut b = Code::evaluation("a.mjs".into());
        b.push(Code::evaluation("second.mjs".into()));
        let mut files = BTreeMap::from([
            ("left.mjs".into(), Code::evaluation("a.mjs".into())),
            ("right.mjs".into(), Code::evaluation("b.mjs".into())),
            ("a.mjs".into(), a),
            ("b.mjs".into(), b),
            ("first.mjs".into(), Code::from("console.log('first');")),
            ("second.mjs".into(), Code::from("console.log('second');")),
        ]);
        let original = files.clone();
        Code::prune(
            &mut files,
            &BTreeSet::from(["a.mjs".into(), "b.mjs".into()]),
            &mut BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(files, original);
    }

    #[test]
    fn pruning_keeps_retained_evaluation_and_namespace_barriers() {
        let mut connector = Code::evaluation("effect.mjs".into());
        connector.push(Code::evaluation("later.mjs".into()));
        let mut effect = Code::evaluation("connector.mjs".into());
        effect.push_str("console.log('effect');");
        let mut files = BTreeMap::from([
            ("entry.mjs".into(), Code::evaluation("connector.mjs".into())),
            ("connector.mjs".into(), connector),
            ("effect.mjs".into(), effect),
            ("later.mjs".into(), Code::from("console.log('later');")),
            ("namespace.mjs".into(), Code::default()),
            (
                "loader.mjs".into(),
                Code::request("namespace.mjs".into(), RequestKind::DynamicImport)
                    .surrounded("import(", ");"),
            ),
        ]);
        let original = files.clone();
        Code::prune(
            &mut files,
            &BTreeSet::from(["connector.mjs".into(), "namespace.mjs".into()]),
            &mut BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(files, original);
    }

    #[test]
    fn composed_replacements_keep_original_utf8_coordinates() {
        let source = "é import('./old.mjs')";
        let output = apply(
            source,
            &[Edit {
                span: Span::new(10, 21),
                replacement: Code::request("new.mjs".into(), RequestKind::DynamicImport),
            }],
        )
        .unwrap();
        let destinations = BTreeMap::from([("new.mjs".into(), "new.mjs".into())]);
        assert_eq!(
            output
                .render(
                    Path::new("entry.mjs"),
                    Path::new("entry.mjs"),
                    &destinations
                )
                .unwrap(),
            "é import(\"./new.mjs\")"
        );
        assert!(apply(
            source,
            &[Edit {
                span: Span::new(1, 2),
                replacement: Code::default(),
            }]
        )
        .is_err());
    }
}
