//! Interim source fragments keep generated requests typed until artifact naming.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
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
    Request(PathBuf, RequestKind),
    External(String, RequestKind),
    Evaluation(PathBuf),
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
        let mut output = String::new();
        for part in &self.parts {
            let (target, url, evaluation) = match part {
                Part::Text(text) => {
                    output.push_str(text);
                    continue;
                }
                Part::External(specifier, _) => {
                    output.push_str(&serde_json::to_string(specifier)?);
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
            let request = serde_json::to_string(&relative_request(destination, target, url))?;
            if evaluation {
                output.push_str("import ");
                output.push_str(&request);
                output.push_str(";\n");
            } else {
                output.push_str(&request);
            }
        }
        Ok(output)
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
    let mut edits: Vec<_> = edits.iter().collect();
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    let mut end = 0;
    let mut previous = None;
    let mut output = Code::default();
    for edit in edits {
        let start = edit.span.start as usize;
        let next = edit.span.end as usize;
        ensure!(
            start <= next
                && next <= source.len()
                && source.is_char_boundary(start)
                && source.is_char_boundary(next),
            "invalid UTF-8 edit range {start}..{next}"
        );
        ensure!(
            start >= end && previous != Some(start),
            "overlapping edits at bytes {start}..{next}"
        );
        output.push_str(&source[end..start]);
        output.push(edit.replacement.clone());
        end = next;
        previous = Some(start);
    }
    output.push_str(source.get(end..).ok_or_eyre("invalid final edit range")?);
    Ok(output)
}

pub(crate) fn apply_range(source: &str, range: Span, edits: &[Edit]) -> eyre::Result<Code> {
    let text = source
        .get(range.start as usize..range.end as usize)
        .ok_or_eyre("invalid UTF-8 source slot")?;
    let edits = edits
        .iter()
        .map(|edit| {
            ensure!(
                range.start <= edit.span.start && edit.span.end <= range.end,
                "edit outside original source slot"
            );
            Ok(Edit {
                span: Span::new(edit.span.start - range.start, edit.span.end - range.start),
                replacement: edit.replacement.clone(),
            })
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    apply(text, &edits)
}

pub(crate) fn relative_request(importer: &Path, target: &Path, url: bool) -> String {
    let parent: Vec<_> = importer.parent().unwrap().components().collect();
    let target: Vec<_> = target.components().collect();
    let shared = parent
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    let mut request = if parent.len() == shared {
        "./".to_owned()
    } else {
        "../".repeat(parent.len() - shared)
    };
    request.push_str(
        &target[shared..]
            .iter()
            .map(|part| match part {
                Component::Normal(value) => value.to_str().unwrap(),
                _ => unreachable!("validated artifact paths are relative"),
            })
            .collect::<Vec<_>>()
            .join("/"),
    );
    if !url {
        return request;
    }
    request
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"/.-_~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
