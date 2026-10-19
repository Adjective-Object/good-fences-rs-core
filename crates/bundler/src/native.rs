//! Scope-hoisted ESM, with native module jobs for observable and async seams.
use super::*;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use source_graph::ExportIdentity;
use std::collections::{HashMap, HashSet};

use crate::rewrite::{self, Edit};
use crate::symbols::Symbols;

#[path = "native_chunks.rs"]
mod chunks;

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
) -> eyre::Result<Option<Artifacts>> {
    if let Some(output) = single(graph, plan, modules, manifest, symbols)? {
        return Ok(Some(output));
    }
    chunks::emit(graph, plan, modules, manifest, symbols)
}

fn single(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
) -> eyre::Result<Option<Artifacts>> {
    // Prefer whole-tree concatenation; the group planner handles native seams.
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
                .references
                .iter()
                .any(|r| r.write && r.binding.is_some_and(|b| imports.contains(&b)))
        {
            return Ok(None);
        }
    }
    if symbols.exports[file]
        .values()
        .any(|e| matches!(e.identity, ExportIdentity::Namespace(_)))
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
    let values = Values::new(graph, modules, symbols);
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
    let mut bodies = String::new();
    for file in order {
        let Some((hoisted, body)) = body(graph, file, &modules[&file], &values, &live, &recipes)?
        else {
            return Ok(None);
        };
        output.push_str(&hoisted);
        bodies.push_str(&body);
        // Separate statements even when a source lacks a final semicolon/newline.
        bodies.push_str("\n;\n");
    }
    output.push_str(&bodies);
    output.push_str("export {\n");
    for (public, linked) in &symbols.exports[file] {
        output.push_str(&format!(
            "{} as {},\n",
            values
                .name(&linked.identity)
                .ok_or_eyre("missing native export value")?,
            quote(public)
        ));
    }
    output.push_str("};\n");
    let path = PathBuf::from("native.mjs");
    output = rewrite::apply(&output, &rewrite::source_hint_edits(&output))?;
    rewrite::validate(&path, &output)?;
    let (chunk, nodes) = plan.chunks.first_key_value().unwrap();
    Ok(Some(Artifacts {
        backend: super::Backend::Native,
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

struct Values<'s> {
    symbols: &'s Symbols,
    defaults: BTreeMap<SegmentKey, String>,
    imports: HashMap<(u32, ast_segmenter::owned::BindingId), String>,
    requests: BTreeMap<u32, String>,
}

impl<'s> Values<'s> {
    fn new(graph: &SourceGraph, modules: &BTreeMap<u32, Module<'_>>, symbols: &'s Symbols) -> Self {
        let mut occupied: BTreeSet<_> = symbols
            .definitions
            .values()
            .map(|d| d.name.clone())
            .collect();
        for module in modules.values() {
            occupied.extend(module.parsed.facts.bindings.iter().map(|b| b.name.clone()));
            occupied.extend(
                module
                    .parsed
                    .facts
                    .references
                    .iter()
                    .map(|r| r.name.clone()),
            );
        }
        let mut files: Vec<_> = modules.keys().copied().collect();
        files.sort_by_key(|&file| graph.file_path(file));
        let mut defaults = BTreeMap::new();
        let mut fresh = 0;
        for file in files {
            let module = &modules[&file];
            for export in &module.parsed.facts.exports {
                if let ExportValue::Expression(span) = export.value {
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|s| s.span.end <= span.start);
                    let name = loop {
                        let name = format!("__gf_default_{fresh}");
                        fresh += 1;
                        if occupied.insert(name.clone()) {
                            break name;
                        }
                    };
                    defaults.insert(SegmentKey::new(file, index as u32), name);
                }
            }
        }
        Self {
            symbols,
            defaults,
            imports: HashMap::new(),
            requests: BTreeMap::new(),
        }
    }

    fn name(&self, identity: &ExportIdentity) -> Option<&str> {
        match identity {
            ExportIdentity::Binding(file, binding) => self
                .symbols
                .definitions
                .get(&(*file, *binding))
                .map(|d| d.name.as_str()),
            ExportIdentity::Expression(key) => self.defaults.get(key).map(String::as_str),
            _ => None,
        }
    }
}

fn body(
    graph: &SourceGraph,
    file: u32,
    module: &Module<'_>,
    values: &Values<'_>,
    live: &BTreeSet<SegmentKey>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
) -> eyre::Result<Option<(String, String)>> {
    let symbols = values.symbols;
    // Compose PURE recipes before re-extracting occurrences. Generated spans
    // never address original text; source binding names reconnect to owned IDs.
    let source = retained_source(file, module, live, recipes)?;
    rewrite::count_output_parse();
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        &source,
        graph.file_path(file).unwrap(),
        ast_segmenter::ModuleFormat::Esm,
    )?;
    let mut names = BTreeMap::new();
    let mut namespaces = BTreeMap::new();
    for binding in module.parsed.facts.bindings.iter().filter(|b| b.top_level) {
        let target = if let Some(name) = values.imports.get(&(file, binding.id)) {
            Some(name.as_str())
        } else if let Some(definition) = symbols.definitions.get(&(file, binding.id)) {
            Some(definition.name.as_str())
        } else if let Some(import) = symbols.imports.get(&(file, binding.id)) {
            if let ExportIdentity::Namespace(target) = import.identity {
                namespaces.insert(binding.name.as_str(), target);
            }
            values.name(&import.identity)
        } else {
            None
        };
        if let Some(target) = target {
            names.insert(binding.name.as_str(), target);
        }
    }
    let mut replacements = BTreeMap::new();
    let mut targets = HashMap::new();
    let mut namespace_bindings = HashMap::new();
    for binding in parsed.facts.bindings.iter().filter(|b| b.top_level) {
        if let Some(&target) = namespaces.get(binding.name.as_str()) {
            namespace_bindings.insert(binding.id, target);
        }
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
        graph,
        values,
        namespaces: parsed
            .facts
            .references
            .iter()
            .filter_map(|r| {
                r.binding
                    .and_then(|b| namespace_bindings.get(&b))
                    .map(|&file| (r.span.start, file))
            })
            .collect(),
        blocked_namespace: false,
        default_name: values
            .defaults
            .iter()
            .find(|(k, _)| k.file_id == file)
            .map(|(_, n)| n.as_str()),
        replacements: &replacements,
        edits: Vec::new(),
        unsupported: false,
        // shortcut: stage anonymous syntax alongside renamed local declarations/
        // writes until AST name-inference sites preserve their original names.
        // Read-only import substitutions cannot change inferred names.
        inference_changes: parsed
            .facts
            .bindings
            .iter()
            .any(|b| !b.import && targets.contains_key(&b.id))
            || parsed
                .facts
                .references
                .iter()
                .any(|r| r.write && r.binding.is_some_and(|b| targets.contains_key(&b))),
        hoists: Vec::new(),
    };
    syntax.visit_program(&ast.program);
    if syntax.unsupported {
        return Ok(None);
    }
    for dependency in parsed
        .facts
        .dependencies
        .iter()
        .filter(|d| !d.type_only && d.kind == DependencyKind::DynamicImport)
    {
        let original = module
            .parsed
            .facts
            .dependencies
            .iter()
            .find(|d| d.kind == dependency.kind && d.specifier == dependency.specifier)
            .ok_or_eyre("missing retained dynamic dependency")?;
        let target = target_file(graph, file, original)?;
        syntax.edits.push(Edit {
            span: dependency.specifier_span,
            replacement: values
                .requests
                .get(&target)
                .ok_or_eyre("missing native dynamic target")?
                .clone(),
        });
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
    let mut hoisted = String::new();
    for (export, function, name) in syntax.hoists {
        let mut edits = Vec::new();
        syntax.edits.retain(|e| {
            if contains(function, e.span) {
                edits.push(Edit {
                    span: Span::new(e.span.start - function.start, e.span.end - function.start),
                    replacement: e.replacement.clone(),
                });
                false
            } else {
                true
            }
        });
        let function = rewrite::apply(
            &source[function.start as usize..function.end as usize],
            &edits,
        )?;
        hoisted.push_str(&format!(
            "var {name} = ({{default: {function}}}).default;\n"
        ));
        syntax.edits.push(Edit {
            span: export,
            replacement: ";".into(),
        });
    }
    Ok(Some((hoisted, rewrite::apply(&source, &syntax.edits)?)))
}

fn retained_source(
    file: u32,
    module: &Module<'_>,
    live: &BTreeSet<SegmentKey>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
) -> eyre::Result<String> {
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
    Ok(source)
}

struct Syntax<'s> {
    graph: &'s SourceGraph,
    values: &'s Values<'s>,
    namespaces: BTreeMap<u32, u32>,
    blocked_namespace: bool,
    default_name: Option<&'s str>,
    replacements: &'s BTreeMap<u32, (Span, String)>,
    edits: Vec<Edit>,
    unsupported: bool,
    inference_changes: bool,
    hoists: Vec<(Span, Span, String)>,
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
        if let Some(name) = self.default_name {
            let span = export.declaration.span();
            if matches!(&export.declaration, ExportDefaultDeclarationKind::FunctionDeclaration(f) if f.id.is_none())
            {
                self.hoists.push((export.span, span, name.to_owned()));
            } else {
                self.edits.push(Edit {
                    span: Span::new(export.span.start, span.start),
                    replacement: format!("const {name} = ({{default: "),
                });
                self.edits.push(Edit {
                    span: Span::new(span.end, export.span.end),
                    replacement: "}).default;".into(),
                });
            }
            walk::walk_export_default_declaration(self, export);
            return;
        }
        self.edits.push(Edit {
            span: Span::new(export.span.start, export.declaration.span().start),
            replacement: String::new(),
        });
        walk::walk_export_default_declaration(self, export);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.unsupported |= function.id.as_ref().map_or(self.inference_changes, |id| {
            self.replacements.contains_key(&id.span.start)
        });
        walk::walk_function(self, function, flags);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        self.unsupported |= class.id.as_ref().map_or(self.inference_changes, |id| {
            self.replacements.contains_key(&id.span.start)
        });
        walk::walk_class(self, class);
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        // Anonymous inferred names also occur in assignments/destructuring.
        self.unsupported |= self.inference_changes;
        walk::walk_arrow_function_expression(self, arrow);
    }

    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        // A namespace that escapes requires a real body-bearing native boundary,
        // not a getter object or an unconditional internal facade.
        self.unsupported |= self.namespaces.contains_key(&reference.span.start);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if let Expression::Identifier(object) = &member.object {
            if let Some(&file) = self.namespaces.get(&object.span.start) {
                if !self.blocked_namespace {
                    if let Ok(linked) = self.graph.resolve_export(
                        file,
                        member.property.name.as_str(),
                        source_graph::Namespace::Value,
                    ) {
                        if let Some(name) = self.values.name(&linked.identity) {
                            self.edits.push(Edit {
                                span: member.span,
                                replacement: name.to_owned(),
                            });
                            return;
                        }
                    }
                }
            }
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let blocked = self.blocked_namespace;
        self.blocked_namespace = true;
        self.visit_expression(&call.callee);
        self.blocked_namespace = blocked;
        for argument in &call.arguments {
            self.visit_argument(argument);
        }
    }

    fn visit_tagged_template_expression(&mut self, tagged: &TaggedTemplateExpression<'a>) {
        let blocked = self.blocked_namespace;
        self.blocked_namespace = true;
        self.visit_expression(&tagged.tag);
        self.blocked_namespace = blocked;
        self.visit_template_literal(&tagged.quasi);
    }

    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        let blocked = self.blocked_namespace;
        self.blocked_namespace = true;
        walk::walk_assignment_target(self, target);
        self.blocked_namespace = blocked;
    }

    fn visit_update_expression(&mut self, update: &UpdateExpression<'a>) {
        let blocked = self.blocked_namespace;
        self.blocked_namespace = true;
        walk::walk_update_expression(self, update);
        self.blocked_namespace = blocked;
    }

    fn visit_unary_expression(&mut self, unary: &UnaryExpression<'a>) {
        let blocked = self.blocked_namespace;
        self.blocked_namespace |= unary.operator == UnaryOperator::Delete;
        walk::walk_unary_expression(self, unary);
        self.blocked_namespace = blocked;
    }
}
