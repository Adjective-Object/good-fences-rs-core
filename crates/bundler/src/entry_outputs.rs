//! Public output names project canonical native files, not low-level root IDs.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use eyre::{ensure, OptionExt, WrapErr};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::BuildOptions;
use crate::{
    link::Artifacts,
    rewrite::{self, Edit},
};

pub(super) fn apply(
    root: &Path,
    options: &BuildOptions,
    artifacts: &mut Artifacts,
) -> eyre::Result<()> {
    let mut names = BTreeMap::new();
    let mut renames: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut aliases = BTreeSet::new();
    let mut packages = BTreeMap::new();
    for (name, entry) in &options.entries {
        let original = artifacts
            .entries
            .get(name)
            .ok_or_eyre("missing emitted public entry")?;
        let commonjs = original.extension().is_some_and(|e| e == "cjs");
        let requested = match &entry.output_name {
            Some(output) => output.clone(),
            None => root
                .join(&entry.path)
                .canonicalize()?
                .strip_prefix(root)?
                .to_owned(),
        };
        let mut path = output_path(
            requested
                .to_str()
                .ok_or_eyre("non-UTF-8 entry output path")?,
        )
        .wrap_err_with(|| format!("entry {name:?} output {}", requested.display()))?;
        if path.extension().is_none() {
            let extension = entry
                .path
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| matches!(*s, "mjs" | "cjs" | "js"))
                .unwrap_or(if commonjs { "cjs" } else { "mjs" });
            path.set_extension(extension);
        }
        let extension = path.extension().and_then(|s| s.to_str());
        ensure!(
            extension == Some("js") || extension == Some(if commonjs { "cjs" } else { "mjs" }),
            "entry output {name:?}: incompatible output extension for {}",
            if commonjs { "CommonJS" } else { "ESM" }
        );
        if extension == Some("js") {
            let package = path.parent().unwrap().join("package.json");
            let format = if commonjs { "commonjs" } else { "module" };
            if let Some(previous) = packages.insert(package.clone(), format) {
                ensure!(
                    previous == format,
                    "entry output {name:?}: incompatible .js module formats in {}",
                    package.display()
                );
            }
        }
        names.insert(name.clone(), path.clone());
        if let Some(canonical) = renames.get(original) {
            if &path != canonical {
                aliases.insert((path, canonical.clone(), original.clone(), commonjs));
            }
        } else {
            renames.insert(original.clone(), path);
        }
    }
    let project = |path: &PathBuf| renames.get(path).cloned().unwrap_or_else(|| path.clone());
    let mut destinations = BTreeSet::new();
    let mut portable_destinations = BTreeSet::new();
    for path in artifacts.files.keys().map(&project).chain(
        aliases
            .iter()
            .map(|(alias, _, _, _)| alias.clone())
            .chain(packages.keys().cloned()),
    ) {
        ensure!(
            portable_destinations.insert(PathBuf::from(
                path.to_str()
                    .ok_or_eyre("non-UTF-8 output path")?
                    .to_lowercase()
            )),
            "entry output collides with generated artifact: {}",
            path.display()
        );
        destinations.insert(path);
    }
    for path in &destinations {
        let portable = PathBuf::from(path.to_str().unwrap().to_lowercase());
        ensure!(
            !portable
                .ancestors()
                .skip(1)
                .any(|p| portable_destinations.contains(p)),
            "entry output file/directory collision: {}",
            path.display()
        );
    }
    let mut files = BTreeMap::new();
    for (original, source) in &artifacts.files {
        let destination = project(original);
        let mut requests = OwnedRequests::parse(original, source)?;
        let mut edits = Vec::new();
        for request in requests.literals.drain(..) {
            let value = request.value.as_str();
            if !value.starts_with("./") && !value.starts_with("../") {
                continue;
            }
            let target = resolve(original, value)?;
            ensure!(
                artifacts.files.contains_key(&target),
                "generated request {value:?} in {} has no artifact",
                original.display()
            );
            let replacement = relative_request(&destination, &project(&target), request.url);
            if replacement != value {
                edits.push(Edit {
                    span: request.span,
                    replacement: serde_json::to_string(&replacement)?,
                });
            }
        }
        let source = rewrite::apply(source, &edits)?;
        if !edits.is_empty() {
            rewrite::validate(&destination, &source)?;
        }
        files.insert(destination, source);
    }
    for (alias, canonical, original, commonjs) in &aliases {
        let request = serde_json::to_string(&relative_request(alias, canonical, !commonjs))?;
        let source = if *commonjs {
            format!(
                "Object.defineProperty(module, 'exports', {{get: () => require({request})}});\nrequire({request});\n"
            )
        } else {
            let parsed = OwnedRequests::parse(original, &artifacts.files[original])?;
            let default = if parsed.default {
                format!("export {{default}} from {request};\n")
            } else {
                String::new()
            };
            format!("export * from {request};\n{default}")
        };
        rewrite::validate(alias, &source)?;
        files.insert(alias.clone(), source);
    }
    for (package, format) in packages {
        files.insert(package, format!("{{\"type\":\"{format}\"}}\n"));
    }
    artifacts.files = files;
    artifacts.entries = names;
    for path in artifacts
        .modules
        .values_mut()
        .chain(artifacts.members.values_mut())
        .chain(artifacts.cjs_proxies.values_mut().map(|(path, _)| path))
        .chain(artifacts.cjs_namespaces.values_mut())
        .chain(artifacts.async_facades.values_mut())
    {
        *path = project(path);
    }
    for paths in artifacts.chunk_artifacts.values_mut() {
        *paths = paths.iter().map(&project).collect();
        for (alias, canonical, _, _) in &aliases {
            if paths.contains(canonical) {
                paths.insert(alias.clone());
            }
        }
    }
    Ok(())
}

fn output_path(name: &str) -> eyre::Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && !name.ends_with('/')
            && !name
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c)),
        "unsafe or nonportable output path"
    );
    let path = Path::new(name);
    ensure!(!path.is_absolute(), "absolute output path");
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir if result.as_os_str().is_empty() => continue,
            Component::Normal(value) => {
                let value = value.to_str().ok_or_eyre("non-UTF-8 output path")?;
                let stem = value.split('.').next().unwrap().to_ascii_lowercase();
                ensure!(
                    !value.starts_with(".bundler-")
                        && !value.ends_with(['.', ' '])
                        && !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
                        && !(stem.len() == 4
                            && (stem.starts_with("com") || stem.starts_with("lpt"))
                            && matches!(stem.as_bytes()[3], b'1'..=b'9')),
                    "reserved output path component: {value}"
                );
                result.push(value);
            }
            _ => eyre::bail!("output path contains traversal or non-normal components"),
        }
    }
    ensure!(!result.as_os_str().is_empty(), "empty output path");
    ensure!(
        !segment_cache::is_cache_artifact(&result),
        "reserved source-cache output path"
    );
    Ok(result)
}

fn resolve(importer: &Path, request: &str) -> eyre::Result<PathBuf> {
    let mut target = importer.parent().unwrap().to_owned();
    for component in Path::new(request).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => ensure!(target.pop(), "generated request escapes output"),
            Component::Normal(name) => target.push(name),
            _ => eyre::bail!("invalid generated relative request"),
        }
    }
    Ok(target)
}

fn relative_request(importer: &Path, target: &Path, url: bool) -> String {
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
            .map(|part| part.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/"),
    );
    if !url {
        return request;
    }
    // Native imports use URLs; CommonJS require uses filesystem paths.
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

struct Requests<'s, 'a> {
    literals: Vec<Literal>,
    default: bool,
    semantic: &'s oxc_semantic::Semantic<'a>,
}

impl OwnedRequests {
    fn parse(path: &Path, source: &str) -> eyre::Result<Self> {
        rewrite::count_output_parse();
        let allocator = Allocator::default();
        let source_type = if path.extension().is_some_and(|e| e == "cjs") {
            SourceType::cjs()
        } else {
            SourceType::mjs()
        };
        let parsed = Parser::new(&allocator, source, source_type).parse();
        ensure!(
            !parsed.panicked && parsed.errors.is_empty(),
            "{}: invalid generated output: {:?}",
            path.display(),
            parsed.errors
        );
        let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
        ensure!(
            semantic.errors.is_empty(),
            "invalid generated output semantics"
        );
        let mut requests = Requests {
            literals: Vec::new(),
            default: false,
            semantic: &semantic.semantic,
        };
        requests.visit_program(&parsed.program);
        Ok(OwnedRequests {
            literals: requests.literals,
            default: requests.default,
        })
    }
}

struct Literal {
    value: String,
    span: oxc_span::Span,
    url: bool,
}

struct OwnedRequests {
    literals: Vec<Literal>,
    default: bool,
}

impl Requests<'_, '_> {
    fn literal(&mut self, literal: &StringLiteral<'_>, url: bool) {
        self.literals.push(Literal {
            value: literal.value.to_string(),
            span: literal.span,
            url,
        });
    }
}

impl<'a> Visit<'a> for Requests<'_, 'a> {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        self.literal(&declaration.source, true);
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        self.literal(&declaration.source, true);
        self.default |= declaration
            .exported
            .as_ref()
            .is_some_and(|name| name.name() == "default");
    }

    fn visit_export_named_declaration(&mut self, declaration: &ExportNamedDeclaration<'a>) {
        if let Some(source) = &declaration.source {
            self.literal(source, true);
        }
        self.default |= declaration
            .specifiers
            .iter()
            .any(|specifier| specifier.exported.name() == "default");
        walk::walk_export_named_declaration(self, declaration);
    }

    fn visit_export_default_declaration(&mut self, declaration: &ExportDefaultDeclaration<'a>) {
        self.default = true;
        walk::walk_export_default_declaration(self, declaration);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if let Expression::StringLiteral(literal) = &expression.source {
            self.literal(literal, true);
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_call_expression(&mut self, expression: &CallExpression<'a>) {
        if matches!(&expression.callee, Expression::Identifier(id)
            if id.name == "require" && id.reference_id.get().is_some_and(|reference|
                self.semantic.scoping().get_reference(reference).symbol_id().is_none()))
        {
            if let Some(Argument::StringLiteral(literal)) = expression.arguments.first() {
                self.literal(literal, false);
            }
        }
        walk::walk_call_expression(self, expression);
    }
}
