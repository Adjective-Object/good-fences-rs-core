//! Source-local facts for executable consumers, independent of the legacy name sets.
#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;

use std::collections::HashMap;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Semantic, SymbolFlags, SymbolId};
use oxc_span::{GetSpan, Span};
pub use records::Directive;
use serde::{Deserialize, Serialize};

/// Canonical parsed contract identity; never hashes raw declaration text.
pub fn declarations_hash(
    declarations: &ecco::LocalFacts,
) -> eyre::Result<crate::DeclarationFingerprint> {
    crate::fingerprint::declarations_hash(declarations)
}

mod records;
pub use records::*;

/// Stable byte-pair representation, independent of OXC's serde features.
pub mod span_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(span: &Span, serializer: S) -> Result<S::Ok, S::Error> {
        (span.start, span.end).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Span, D::Error> {
        let (start, end) = Deserialize::deserialize(deserializer)?;
        Ok(Span::new(start, end))
    }

    pub mod optional {
        use super::*;

        pub fn serialize<S: Serializer>(span: &Option<Span>, s: S) -> Result<S::Ok, S::Error> {
            span.map(|s| (s.start, s.end)).serialize(s)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Span>, D::Error> {
            Ok(Option::<(u32, u32)>::deserialize(d)?.map(|(a, b)| Span::new(a, b)))
        }
    }

    pub mod list {
        use super::*;

        pub fn serialize<S: Serializer>(spans: &[Span], s: S) -> Result<S::Ok, S::Error> {
            spans
                .iter()
                .map(|s| (s.start, s.end))
                .collect::<Vec<_>>()
                .serialize(s)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Span>, D::Error> {
            Ok(Vec::<(u32, u32)>::deserialize(d)?
                .into_iter()
                .map(|(a, b)| Span::new(a, b))
                .collect())
        }
    }
}

pub(crate) fn extract(program: &Program<'_>, semantic: &Semantic<'_>) -> eyre::Result<ModuleFacts> {
    let scoping = semantic.scoping();
    let ids: HashMap<_, _> = scoping
        .symbol_ids()
        .map(|symbol| (symbol, BindingId(scoping.symbol_span(symbol).start)))
        .collect();
    let bindings: Vec<Binding> = scoping
        .symbol_ids()
        .map(|symbol| {
            let flags = scoping.symbol_flags(symbol);
            Binding::new(
                ids[&symbol],
                scoping.symbol_name(symbol).to_owned(),
                Vec::new(),
                scoping.symbol_scope_id(symbol) == scoping.root_scope_id(),
                flags.contains(SymbolFlags::FunctionScopedVariable),
                flags.contains(SymbolFlags::Function),
                flags.intersects(SymbolFlags::Import | SymbolFlags::TypeImport),
                flags.contains(SymbolFlags::ConstVariable),
                flags.is_type() && !flags.is_value(),
                flags.contains(SymbolFlags::Ambient),
            )
        })
        .collect();
    let mut visitor = Extractor {
        semantic,
        program,
        ids,
        binding_indices: bindings
            .iter()
            .enumerate()
            .map(|(i, b): (_, &Binding)| (b.id(), i))
            .collect(),
        facts: ModuleFacts::new(
            Default::default(),
            Default::default(),
            program.source_type.is_commonjs(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            bindings,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            program
                .directives
                .iter()
                .map(|d| Directive::new(d.expression.value.display().to_string(), d.span))
                .collect(),
            program
                .hashbang
                .as_ref()
                .map(|h| Directive::new(h.value.to_string(), h.span)),
        ),
        errors: Vec::new(),
        exported_declaration: None,
        commonjs: program.source_type.is_commonjs(),
        enclosing_function: None,
        async_deferred: false,
        function_body_async: false,
    };
    visitor.visit_program(program);
    eyre::ensure!(visitor.errors.is_empty(), "{}", visitor.errors.join("; "));
    let mut facts = visitor.facts;
    facts.bindings_mut().sort_by_key(|b| b.id().0);
    for binding in facts.bindings_mut() {
        binding.declarations_mut().sort_by_key(|s| (s.start, s.end));
        binding.declarations_mut().dedup();
    }
    facts
        .references_mut()
        .sort_by_key(|r| (r.span().start, r.span().end));
    facts.dependencies_mut().sort_by_key(|d| d.id().0);
    facts
        .top_level_await_mut()
        .sort_by_key(|s| (s.start, s.end));
    facts.imports_mut().sort_by_key(|i| i.span().start);
    facts
        .exports_mut()
        .sort_by_key(|e| (e.span().start, e.name_span().map(|s| s.start)));
    let (effects, invocations) = crate::effects::extract(program, &facts);
    *facts.effects_mut() = effects;
    *facts.invocations_mut() = invocations;
    let segments = if facts.commonjs() {
        let span = Span::new(
            program.hashbang.as_ref().map_or(0, |h| h.span.end),
            program.source_text.len() as u32,
        );
        vec![(span, span.into())]
    } else {
        program
            .body
            .iter()
            .flat_map(crate::multi_span::statement_sources)
            .collect()
    };
    if !facts.commonjs() {
        for export in facts.exports_mut() {
            if let Some(name) = export.name_span() {
                let index = segments.partition_point(|(span, _)| span.end <= name.start);
                if let Some((span, source)) = segments.get(index) {
                    if source.as_slice().len() > 1
                        && span.start <= name.start
                        && name.end <= span.end
                    {
                        *export.span_mut() = *span;
                    }
                }
            }
        }
    }
    *facts.emission_mut() = crate::emission::extract(program, &facts, &segments);
    Ok(facts)
}

fn webpack_ignored(program: &Program<'_>, expression: &ImportExpression<'_>) -> bool {
    let mut ignored = false;
    // Match webpack's parseCommentOptions(expr.range)/getComments(range):
    // nested import/body comments also belong to the enclosing import range.
    let start = program
        .comments
        .partition_point(|comment| comment.span.start < expression.span.start);
    let end = program
        .comments
        .partition_point(|comment| comment.span.end <= expression.span.end);
    for comment in &program.comments[start..end] {
        let text = &program.source_text[comment.span.start as usize..comment.span.end as usize];
        let body = if text.starts_with("/**") {
            continue;
        } else if let Some(body) = text.strip_prefix("/*").and_then(|s| s.strip_suffix("*/")) {
            body
        } else if let Some(body) = text.strip_prefix("//") {
            body
        } else {
            continue;
        };
        if !body.contains("webpackIgnore") {
            continue;
        }
        // Parse comment data as an object; never evaluate comment expressions.
        let source = format!("({{{body}\n}})");
        let allocator = oxc_allocator::Allocator::default();
        let Ok(parsed) = oxc_utils_parse::parse_with_source_type(
            &allocator,
            &source,
            oxc_span::SourceType::mjs(),
            std::path::Path::new("import-magic-comment.mjs"),
        ) else {
            continue;
        };
        if parsed.fatal_error || !parsed.diagnostics.is_empty() || parsed.program.body.len() != 1 {
            continue;
        }
        let Statement::ExpressionStatement(statement) = &parsed.program.body[0] else {
            continue;
        };
        let Expression::ObjectExpression(object) = statement.expression.get_inner_expression()
        else {
            continue;
        };
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if !property.computed
                && !property.method
                && property
                    .key
                    .static_name()
                    .is_some_and(|name| name.as_str() == Some("webpackIgnore"))
            {
                ignored =
                    matches!(&property.value, Expression::BooleanLiteral(value) if value.value);
            }
        }
    }
    ignored
}

struct Extractor<'s, 'a> {
    semantic: &'s Semantic<'a>,
    program: &'s Program<'a>,
    ids: HashMap<SymbolId, BindingId>,
    binding_indices: HashMap<BindingId, usize>,
    facts: ModuleFacts,
    errors: Vec<String>,
    exported_declaration: Option<(Span, bool)>,
    commonjs: bool,
    enclosing_function: Option<Span>,
    async_deferred: bool,
    function_body_async: bool,
}

impl Extractor<'_, '_> {
    fn context_dependency(&mut self, kind: DependencyKind, span: Span, source: &Expression<'_>) {
        fn pieces(source: &Expression<'_>, parts: &mut Vec<String>) -> Option<()> {
            match source.get_inner_expression() {
                Expression::StringLiteral(s) => {
                    parts.last_mut().unwrap().push_str(s.value.as_str()?)
                }
                Expression::TemplateLiteral(t) => {
                    for (index, quasi) in t.quasis.iter().enumerate() {
                        if index > 0 {
                            parts.push(String::new());
                        }
                        if let Some(cooked) = &quasi.value.cooked {
                            parts.last_mut().unwrap().push_str(cooked.as_str()?);
                        }
                    }
                }
                Expression::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
                    pieces(&b.left, parts)?;
                    pieces(&b.right, parts)?;
                }
                _ => parts.push(String::new()),
            }
            Some(())
        }
        let mut pattern = vec![String::new()];
        if pieces(source, &mut pattern).is_none() {
            self.reject(span, "module request contains a lone UTF-16 surrogate");
            return;
        }
        if !pattern[0].starts_with("./") && !pattern[0].starts_with("../") {
            if kind == DependencyKind::Require {
                self.reject(
                    span,
                    "nonliteral require requires a relative literal prefix",
                );
                return;
            }
            // The empty-prefix wildcard denotes the importing directory, not
            // an unrestricted filesystem scope. Runtime arguments stay intact.
            pattern = vec![String::new(), String::new()];
        }
        self.facts.dependencies_mut().push(Dependency::new(
            DependencyId(span.start),
            kind,
            span,
            pattern.join("*"),
            Some(pattern),
            source.span(),
            false,
            self.enclosing_function,
        ));
    }

    fn binding(&self, id: &BindingIdentifier<'_>) -> BindingId {
        self.ids[&id.symbol_id.get().expect("validated binding")]
    }

    fn root_binding(&self, name: &str) -> Option<BindingId> {
        self.facts
            .bindings()
            .iter()
            .find(|b| b.top_level() && b.name() == name)
            .map(|b| b.id())
    }

    fn dependency(
        &mut self,
        kind: DependencyKind,
        span: Span,
        source: &StringLiteral<'_>,
        type_only: bool,
    ) -> DependencyId {
        let id = DependencyId(span.start);
        let Some(request) = source.value.as_str() else {
            self.reject(
                source.span,
                "module request contains a lone UTF-16 surrogate",
            );
            return id;
        };
        self.facts.dependencies_mut().push(Dependency::new(
            id,
            kind,
            span,
            request.to_owned(),
            None,
            source.span,
            type_only,
            self.enclosing_function,
        ));
        id
    }

    fn reject(&mut self, span: Span, reason: &str) {
        self.errors
            .push(format!("{reason} at bytes {}..{}", span.start, span.end));
    }
}

fn literal_worker_url<'b, 'a>(
    expression: &'b NewExpression<'a>,
) -> Option<(&'b NewExpression<'a>, &'b StringLiteral<'a>)> {
    let Expression::Identifier(worker) = &expression.callee else {
        return None;
    };
    if worker.name != "Worker" || expression.type_arguments.is_some() {
        return None;
    }
    let Argument::NewExpression(expression) = expression.arguments.first()? else {
        return None;
    };
    if expression.arguments.len() != 2 || expression.type_arguments.is_some() {
        return None;
    }
    let Expression::Identifier(callee) = &expression.callee else {
        return None;
    };
    if callee.name != "URL" {
        return None;
    }
    let Argument::StringLiteral(source) = &expression.arguments[0] else {
        return None;
    };
    let request = source.value.as_str()?;
    if !(request.starts_with("./") || request.starts_with("../"))
        || request.contains(['?', '#', '%', '\\'])
    {
        return None;
    }
    let Argument::StaticMemberExpression(base) = &expression.arguments[1] else {
        return None;
    };
    if base.optional || base.property.name != "url" {
        return None;
    }
    matches!(&base.object, Expression::ImportMeta(_)).then_some((expression, source))
}

impl<'a> Visit<'a> for Extractor<'_, 'a> {
    fn visit_new_expression(&mut self, expression: &NewExpression<'a>) {
        if !self.commonjs {
            if let Some((url, source)) = literal_worker_url(expression) {
                if let Expression::Identifier(callee) = url.callee.get_inner_expression() {
                    let reference = self
                        .semantic
                        .scoping()
                        .get_reference(callee.reference_id.get().expect("validated URL reference"));
                    if reference.symbol_id().is_none() {
                        self.dependency(DependencyKind::Url, url.span, source, false);
                    }
                }
            }
        }
        walk::walk_new_expression(self, expression);
    }

    fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
        if self.enclosing_function.is_none() {
            self.facts.top_level_await_mut().push(expression.span);
        }

        walk::walk_await_expression(self, expression);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        if statement.r#await && self.enclosing_function.is_none() {
            self.facts.top_level_await_mut().push(statement.span);
        }
        walk::walk_for_of_statement(self, statement);
    }

    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        if let Some(symbol) = id.symbol_id.get() {
            let binding = self.ids[&symbol];
            self.facts.bindings_mut()[self.binding_indices[&binding]]
                .declarations_mut()
                .push(id.span);
            if let Some((span, type_only)) = self.exported_declaration {
                let info = &self.facts.bindings()[self.binding_indices[&binding]];
                if info.top_level() {
                    let type_only = type_only || info.type_only();
                    self.facts.exports_mut().push(Export::new(
                        Some(id.name.to_string()),
                        ExportValue::Binding(binding),
                        span,
                        Some(id.span),
                        type_only,
                    ));
                }
            }
        }
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        let reference = self
            .semantic
            .scoping()
            .get_reference(id.reference_id.get().expect("validated reference"));
        let type_only = reference.is_type() && !reference.is_value();
        let commonjs = if self.commonjs && reference.symbol_id().is_none() && !type_only {
            match id.name.as_str() {
                "require" => Some(CommonJsGlobal::Require),
                "module" => Some(CommonJsGlobal::Module),
                "exports" => Some(CommonJsGlobal::Exports),
                _ => None,
            }
        } else {
            None
        };
        let reference = Reference::new(
            id.name.to_string(),
            id.span,
            reference.symbol_id().map(|s| self.ids[&s]),
            reference.is_read(),
            reference.is_write(),
            type_only,
            self.async_deferred,
            commonjs,
        );
        self.facts.references_mut().push(reference);
    }

    fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
        let dependency = self.dependency(
            DependencyKind::Import,
            decl.span,
            &decl.source,
            decl.import_kind.is_type(),
        );
        if let Some(specifiers) = &decl.specifiers {
            for specifier in specifiers {
                let (imported, imported_span, type_only) = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => (
                        ImportedName::Name(s.imported.name().to_string()),
                        s.imported.span(),
                        s.import_kind.is_type(),
                    ),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        (ImportedName::Name("default".into()), s.local.span, false)
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        (ImportedName::Namespace, s.span, false)
                    }
                };
                let binding = self.binding(specifier.local());
                self.facts.imports_mut().push(ImportBinding::new(
                    dependency,
                    binding,
                    imported,
                    specifier.span(),
                    imported_span,
                    decl.import_kind.is_type() || type_only,
                ));
            }
        }
        walk::walk_import_declaration(self, decl);
    }

    fn visit_export_named_declaration(&mut self, decl: &ExportNamedDeclaration<'a>) {
        for specifier in &decl.specifiers {
            let value = if let Some(binding) = self.root_binding(&specifier.local.name()) {
                ExportValue::Binding(binding)
            } else {
                self.reject(specifier.span, "unresolved local export");
                continue;
            };
            let local_type_only = match &value {
                ExportValue::Binding(binding) => {
                    self.facts.bindings()[self.binding_indices[binding]].type_only()
                }
                _ => false,
            };
            self.facts.exports_mut().push(Export::new(
                Some(specifier.exported.name().to_string()),
                value,
                specifier.span,
                Some(specifier.exported.span()),
                decl.export_kind.is_type() || specifier.export_kind.is_type() || local_type_only,
            ));
        }
        walk::walk_export_named_declaration(self, decl);
    }

    fn visit_export_from_declaration(&mut self, decl: &ExportFromDeclaration<'a>) {
        let dependency = self.dependency(
            DependencyKind::ReExport,
            decl.span,
            &decl.source,
            decl.export_kind.is_type(),
        );
        for specifier in &decl.specifiers {
            self.facts.exports_mut().push(Export::new(
                Some(specifier.exported.name().to_string()),
                ExportValue::ReExport {
                    dependency,
                    imported: ImportedName::Name(specifier.local.name().to_string()),
                },
                specifier.span,
                Some(specifier.exported.span()),
                decl.export_kind.is_type() || specifier.export_kind.is_type(),
            ));
        }
        walk::walk_export_from_declaration(self, decl);
    }

    fn visit_export_declaration(&mut self, decl: &ExportDeclaration<'a>) {
        let previous = self.exported_declaration;
        self.exported_declaration = Some((
            decl.declaration.span(),
            matches!(
                decl.declaration,
                Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_)
            ),
        ));
        walk::walk_export_declaration(self, decl);
        self.exported_declaration = previous;
    }

    fn visit_export_default_declaration(&mut self, decl: &ExportDefaultDeclaration<'a>) {
        let binding = match &decl.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                f.id.as_ref().map(|id| self.binding(id))
            }
            ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                c.id.as_ref().map(|id| self.binding(id))
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => Some(self.binding(&i.id)),
            _ => None,
        };
        let type_only = match &decl.declaration {
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => true,
            ExportDefaultDeclarationKind::Identifier(id) => {
                let reference = self
                    .semantic
                    .scoping()
                    .get_reference(id.reference_id.get().expect("validated default reference"));
                reference.is_type() && !reference.is_value()
            }
            _ => false,
        };
        self.facts.exports_mut().push(Export::new(
            Some("default".into()),
            binding.map_or(
                ExportValue::Expression(decl.declaration.span()),
                ExportValue::Binding,
            ),
            decl.span,
            None,
            type_only,
        ));
        walk::walk_export_default_declaration(self, decl);
    }

    fn visit_export_all_declaration(&mut self, decl: &ExportAllDeclaration<'a>) {
        let dependency = self.dependency(
            DependencyKind::ReExport,
            decl.span,
            &decl.source,
            decl.export_kind.is_type(),
        );
        self.facts.exports_mut().push(Export::new(
            decl.exported.as_ref().map(|n| n.name().to_string()),
            ExportValue::ReExport {
                dependency,
                imported: ImportedName::Namespace,
            },
            decl.span,
            decl.exported.as_ref().map(GetSpan::span),
            decl.export_kind.is_type(),
        ));
        walk::walk_export_all_declaration(self, decl);
    }

    fn visit_import_expression(&mut self, expr: &ImportExpression<'a>) {
        if webpack_ignored(self.program, expr) {
            // Still walk inputs: ignoring a dependency never makes it pure.
        } else if let Expression::StringLiteral(source) = expr.source.get_inner_expression() {
            self.dependency(DependencyKind::DynamicImport, expr.span, source, false);
        } else {
            self.context_dependency(DependencyKind::DynamicImport, expr.span, &expr.source);
        }

        walk::walk_import_expression(self, expr);
    }

    fn visit_call_expression(&mut self, expr: &CallExpression<'a>) {
        if let Expression::Identifier(id) = expr.callee.get_inner_expression() {
            let reference = self
                .semantic
                .scoping()
                .get_reference(id.reference_id.get().expect("validated reference"));
            if id.name == "require" && reference.symbol_id().is_none() {
                if expr.optional || expr.arguments.len() != 1 {
                    self.reject(
                        expr.span,
                        "unsupported require call (expected one argument, no optional call)",
                    );
                } else if let Some(Expression::StringLiteral(source)) = expr
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .map(Expression::get_inner_expression)
                {
                    self.dependency(DependencyKind::Require, expr.span, source, false);
                } else if let Some(source) =
                    expr.arguments.first().and_then(Argument::as_expression)
                {
                    self.context_dependency(DependencyKind::Require, expr.span, source);
                } else {
                    self.reject(expr.span, "nonliteral require");
                }
            }
        }
        walk::walk_call_expression(self, expr);
    }

    fn visit_ts_import_type(&mut self, expr: &TSImportType<'a>) {
        self.dependency(DependencyKind::ImportType, expr.span, &expr.source, true);
        walk::walk_ts_import_type(self, expr);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let previous = self.enclosing_function.replace(function.span);
        let deferred = std::mem::replace(&mut self.async_deferred, false);
        let body_async = std::mem::replace(&mut self.function_body_async, function.r#async);
        walk::walk_function(self, function, flags);
        self.function_body_async = body_async;
        self.async_deferred = deferred;
        self.enclosing_function = previous;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let previous = self.enclosing_function.replace(function.span);
        let deferred = std::mem::replace(&mut self.async_deferred, false);
        let body_async = std::mem::replace(&mut self.function_body_async, function.r#async);
        walk::walk_arrow_function_expression(self, function);
        self.function_body_async = body_async;
        self.async_deferred = deferred;
        self.enclosing_function = previous;
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        let previous = std::mem::replace(&mut self.async_deferred, self.function_body_async);
        walk::walk_function_body(self, body);
        self.async_deferred = previous;
    }

    fn visit_arrow_function_body(&mut self, body: &ArrowFunctionBody<'a>) {
        let previous = std::mem::replace(&mut self.async_deferred, self.function_body_async);
        walk::walk_arrow_function_body(self, body);
        self.async_deferred = previous;
    }

    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        self.visit_decorators(&property.decorators);
        self.visit_property_key(&property.key);
        let previous = std::mem::replace(&mut self.async_deferred, false);
        if let Some(annotation) = &property.type_annotation {
            self.visit_ts_type_annotation(annotation);
        }
        if let Some(value) = &property.value {
            self.visit_expression(value);
        }
        self.async_deferred = previous;
    }

    fn visit_accessor_property(&mut self, property: &AccessorProperty<'a>) {
        self.visit_decorators(&property.decorators);
        self.visit_property_key(&property.key);
        let previous = std::mem::replace(&mut self.async_deferred, false);
        if let Some(annotation) = &property.type_annotation {
            self.visit_ts_type_annotation(annotation);
        }
        if let Some(value) = &property.value {
            self.visit_expression(value);
        }
        self.async_deferred = previous;
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        let previous = std::mem::replace(&mut self.async_deferred, false);
        walk::walk_static_block(self, block);
        self.async_deferred = previous;
    }
}
