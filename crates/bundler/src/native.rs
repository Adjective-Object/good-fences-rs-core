//! First staged native slice: one synchronous acyclic ESM evaluation tree.
use super::*;
use ast_segmenter::owned::ImportedName;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use source_graph::ExportIdentity;
use std::collections::{HashMap, HashSet};

use crate::rewrite::{self, Edit};
use crate::symbols::Symbols;

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
) -> eyre::Result<Option<Artifacts>> {
    // shortcut: keep unported boundaries on the legacy emitter until their
    // native semantics are implemented; remove this staging gate with that emitter.
    if plan.roots.len() != 1 || plan.chunks.len() != 1 {
        return Ok(None);
    }
    let (RootId::Entry(entry), seed) = plan.roots.first_key_value().unwrap() else {
        return Ok(None);
    };
    let [file] = seed.evaluation_roots.as_slice() else {
        return Ok(None);
    };
    if seed.binding_roots != [GraphNode::Module(*file)] {
        return Ok(None);
    }
    for module in modules.values() {
        let facts = &module.parsed.facts;
        let imports: HashSet<_> = facts.imports.iter().map(|i| i.binding).collect();
        if facts.commonjs
            || !facts.top_level_await.is_empty()
            || facts.dependencies.iter().any(|d| {
                !d.type_only && !matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            })
            || facts
                .imports
                .iter()
                .any(|i| i.imported == ImportedName::Namespace)
            || facts
                .exports
                .iter()
                .any(|e| matches!(e.value, ExportValue::Expression(_)))
            || facts
                .references
                .iter()
                .any(|r| r.write && r.binding.is_some_and(|b| imports.contains(&b)))
        {
            return Ok(None);
        }
    }
    if symbols
        .imports
        .values()
        .chain(symbols.exports.values().flat_map(|e| e.values()))
        .any(|e| !matches!(e.identity, ExportIdentity::Binding(..)))
    {
        return Ok(None);
    }

    // Static dependency occurrences, not path order or the scalar effect DAG,
    // define synchronous ESM evaluation. Gray nodes defer real binding cycles.
    let mut state = BTreeMap::new();
    let mut order = Vec::new();
    let mut stack = vec![(*file, false)];
    while let Some((file, finish)) = stack.pop() {
        if finish {
            state.insert(file, 2);
            order.push(file);
            continue;
        }
        match state.get(&file) {
            Some(1) => return Ok(None),
            Some(2) => continue,
            _ => {}
        }
        state.insert(file, 1);
        stack.push((file, true));
        let module = modules.get(&file).ok_or_eyre("missing native dependency")?;
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .rev()
            .filter(|d| !d.type_only)
        {
            stack.push((target_file(graph, file, dependency)?, false));
        }
    }
    ensure!(
        order.len() == modules.len(),
        "native root does not cover retained modules"
    );
    let mut output = String::new();
    let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
    let recipes: BTreeMap<_, _> = plan
        .retention
        .effects
        .iter()
        .filter(|e| !e.summary.rewrites.is_empty())
        .map(|e| (e.key, e.summary.rewrites.as_slice()))
        .collect();
    if let Some(hashbang) = &modules[file].parsed.facts.hashbang {
        output.push_str(
            &modules[file].source[hashbang.span.start as usize..hashbang.span.end as usize],
        );
        output.push('\n');
    }
    for file in order {
        let Some(body) = body(graph, file, &modules[&file], symbols, &live, &recipes)? else {
            return Ok(None);
        };
        output.push_str(&body);
        // Separate statements even when a source lacks a final semicolon/newline.
        output.push_str("\n;\n");
    }
    output.push_str("export {\n");
    for (public, linked) in &symbols.exports[file] {
        let ExportIdentity::Binding(file, binding) = linked.identity else {
            unreachable!()
        };
        output.push_str(&format!(
            "{} as {},\n",
            symbols.definitions[&(file, binding)].name,
            quote(public)
        ));
    }
    output.push_str("};\n");
    let path = PathBuf::from("native.mjs");
    output = rewrite::apply(&output, &rewrite::source_hint_edits(&output))?;
    rewrite::validate(&path, &output)?;
    let (chunk, nodes) = plan.chunks.first_key_value().unwrap();
    Ok(Some(Artifacts {
        files: BTreeMap::from([(path.clone(), output)]),
        entries: BTreeMap::from([(entry.clone(), path.clone())]),
        // Only the public entry namespace is executable as an independent module.
        modules: BTreeMap::from([(graph.file_path(*file).unwrap().to_owned(), path.clone())]),
        members: nodes
            .iter()
            .map(|node| (node.clone(), path.clone()))
            .collect(),
        chunk_artifacts: BTreeMap::from([(chunk.clone(), BTreeSet::from([path]))]),
        manifest: manifest.clone(),
        cjs_proxies: BTreeMap::new(),
        cjs_namespaces: BTreeMap::new(),
        async_facades: BTreeMap::new(),
        async_modules: BTreeSet::new(),
    }))
}

fn body(
    graph: &SourceGraph,
    file: u32,
    module: &Module<'_>,
    symbols: &Symbols,
    live: &BTreeSet<SegmentKey>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
) -> eyre::Result<Option<String>> {
    // Compose PURE recipes before re-extracting occurrences. Generated spans
    // never address original text; source binding names reconnect to owned IDs.
    let mut source = String::new();
    for (index, segment) in module.parsed.segments.iter().enumerate() {
        let key = SegmentKey::new(file, index as u32);
        if !live.contains(&key) {
            continue;
        }
        if let Some(recipes) = recipes.get(&key) {
            source.push_str(&crate::emit::render_segment(
                module.source,
                segment.span,
                recipes,
                &module.parsed.facts,
            )?);
        } else {
            source.push_str(&module.source[segment.span.start as usize..segment.span.end as usize]);
        }
        // Retention and PURE rewrites can expose a new ASI continuation.
        source.push_str("\n;\n");
    }
    rewrite::count_output_parse();
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        &source,
        graph.file_path(file).unwrap(),
        ast_segmenter::ModuleFormat::Esm,
    )?;
    let mut names = BTreeMap::new();
    for binding in module.parsed.facts.bindings.iter().filter(|b| b.top_level) {
        let target = if let Some(definition) = symbols.definitions.get(&(file, binding.id)) {
            Some(&definition.name)
        } else if let Some(import) = symbols.imports.get(&(file, binding.id)) {
            let ExportIdentity::Binding(target, binding) = import.identity else {
                unreachable!()
            };
            Some(&symbols.definitions[&(target, binding)].name)
        } else {
            None
        };
        if let Some(target) = target {
            names.insert(binding.name.as_str(), target.as_str());
        }
    }
    let mut replacements = BTreeMap::new();
    let mut targets = HashMap::new();
    for binding in parsed.facts.bindings.iter().filter(|b| b.top_level) {
        let Some(&target) = names.get(binding.name.as_str()) else {
            continue;
        };
        if target == binding.name {
            continue;
        }
        targets.insert(binding.id, target);
        for span in &binding.declarations {
            replacements.insert(span.start, (*span, target.to_owned()));
        }
    }
    for reference in &parsed.facts.references {
        if let Some(target) = reference.binding.and_then(|b| targets.get(&b)) {
            replacements.insert(reference.span.start, (reference.span, (*target).to_owned()));
        }
    }
    let allocator = Allocator::default();
    rewrite::count_output_parse();
    let ast = Parser::new(&allocator, &source, SourceType::mjs()).parse();
    let mut syntax = Syntax {
        replacements: &replacements,
        edits: Vec::new(),
        name_sensitive: false,
    };
    syntax.visit_program(&ast.program);
    if syntax.name_sensitive {
        return Ok(None);
    }
    let mut shorthand = rewrite::Shorthand::default();
    shorthand.visit_program(&ast.program);
    let removed: BTreeMap<_, _> = syntax
        .edits
        .iter()
        .map(|e| (e.span.start, e.span))
        .collect();
    for (span, name) in replacements.values() {
        if removed
            .range(..=span.start)
            .next_back()
            .is_some_and(|(_, &range)| contains(range, *span))
        {
            continue;
        }
        syntax.edits.push(Edit {
            span: *span,
            replacement: shorthand.replacement(&source, *span, name),
        });
    }
    rewrite::apply(&source, &syntax.edits).map(Some)
}

struct Syntax<'s> {
    replacements: &'s BTreeMap<u32, (Span, String)>,
    edits: Vec<Edit>,
    name_sensitive: bool,
}

impl<'a> Visit<'a> for Syntax<'_> {
    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        self.edits.push(Edit {
            span: import.span,
            replacement: ";".into(),
        });
    }

    fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
        self.edits.push(Edit {
            span: export.declaration.as_ref().map_or(export.span, |d| {
                Span::new(export.span.start, d.span().start)
            }),
            replacement: if export.declaration.is_some() {
                String::new()
            } else {
                ";".into()
            },
        });
        walk::walk_export_named_declaration(self, export);
    }

    fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
        self.edits.push(Edit {
            span: export.span,
            replacement: ";".into(),
        });
    }

    fn visit_export_default_declaration(&mut self, export: &ExportDefaultDeclaration<'a>) {
        self.edits.push(Edit {
            span: Span::new(export.span.start, export.declaration.span().start),
            replacement: String::new(),
        });
        walk::walk_export_default_declaration(self, export);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.name_sensitive |= function
            .id
            .as_ref()
            .map_or(!self.replacements.is_empty(), |id| {
                self.replacements.contains_key(&id.span.start)
            });
        walk::walk_function(self, function, flags);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.name_sensitive |= class
            .id
            .as_ref()
            .map_or(!self.replacements.is_empty(), |id| {
                self.replacements.contains_key(&id.span.start)
            });
        walk::walk_class(self, class);
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        // Anonymous inferred names also occur in assignments/destructuring.
        self.name_sensitive |= !self.replacements.is_empty();
        walk::walk_arrow_function_expression(self, arrow);
    }
}
