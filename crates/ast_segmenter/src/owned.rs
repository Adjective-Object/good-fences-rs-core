//! Source-local facts for executable consumers, independent of the legacy name sets.
#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;

use std::collections::HashMap;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Semantic, SymbolFlags, SymbolId};
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Serialize};

/// The byte offset of a binding's first declaration, not an OXC symbol ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindingId(pub u32);

/// The byte offset of a dependency expression/declaration. Repeated paths stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DependencyId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub id: BindingId,
    pub name: String,
    #[serde(with = "span_serde::list")]
    pub declarations: Vec<Span>,
    pub top_level: bool,
    pub function_scoped: bool,
    pub function: bool,
    pub import: bool,
    pub constant: bool,
    pub type_only: bool,
    pub ambient: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub name: String,
    #[serde(with = "span_serde")]
    pub span: Span,
    pub binding: Option<BindingId>,
    pub read: bool,
    pub write: bool,
    pub type_only: bool,
    /// Wrapper parameter in CommonJS; never set for a lexical shadow.
    pub commonjs: Option<CommonJsGlobal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommonJsGlobal {
    Require,
    Module,
    Exports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyKind {
    Import,
    ReExport,
    DynamicImport,
    Require,
    ImportType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub id: DependencyId,
    pub kind: DependencyKind,
    #[serde(with = "span_serde")]
    pub span: Span,
    pub specifier: String,
    #[serde(with = "span_serde")]
    pub specifier_span: Span,
    pub type_only: bool,
    /// Lexical containment only, not a claim that a module-level site runs eagerly.
    #[serde(with = "span_serde::optional")]
    pub enclosing_function: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportedName {
    Name(String),
    Namespace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportBinding {
    pub dependency: DependencyId,
    pub binding: BindingId,
    pub imported: ImportedName,
    #[serde(with = "span_serde")]
    pub span: Span,
    #[serde(with = "span_serde")]
    pub imported_span: Span,
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportValue {
    Binding(BindingId),
    ReExport {
        dependency: DependencyId,
        imported: ImportedName,
    },
    /// Includes anonymous default functions/classes and default expressions.
    Expression(#[serde(with = "span_serde")] Span),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    /// `None` means export-star, not a default export.
    pub name: Option<String>,
    pub value: ExportValue,
    #[serde(with = "span_serde")]
    pub span: Span,
    /// Absent for the implicit `default` name and export-star.
    #[serde(with = "span_serde::optional")]
    pub name_span: Option<Span>,
    pub type_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directive {
    pub value: String,
    #[serde(with = "span_serde")]
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFacts {
    pub effects: Vec<crate::effects::EffectSite>,
    pub bindings: Vec<Binding>,
    pub references: Vec<Reference>,
    pub dependencies: Vec<Dependency>,
    pub imports: Vec<ImportBinding>,
    pub exports: Vec<Export>,
    pub directives: Vec<Directive>,
    pub hashbang: Option<Directive>,
}

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
            Binding {
                id: ids[&symbol],
                name: scoping.symbol_name(symbol).to_owned(),
                declarations: Vec::new(),
                top_level: scoping.symbol_scope_id(symbol) == scoping.root_scope_id(),
                function_scoped: flags.contains(SymbolFlags::FunctionScopedVariable),
                function: flags.contains(SymbolFlags::Function),
                import: flags.intersects(SymbolFlags::Import | SymbolFlags::TypeImport),
                constant: flags.contains(SymbolFlags::ConstVariable),
                type_only: flags.is_type() && !flags.is_value(),
                ambient: flags.contains(SymbolFlags::Ambient),
            }
        })
        .collect();
    let mut visitor = Extractor {
        semantic,
        ids,
        binding_indices: bindings
            .iter()
            .enumerate()
            .map(|(i, b): (_, &Binding)| (b.id, i))
            .collect(),
        facts: ModuleFacts {
            bindings,
            directives: program
                .directives
                .iter()
                .map(|d| Directive {
                    value: d.expression.value.to_string(),
                    span: d.span,
                })
                .collect(),
            hashbang: program.hashbang.as_ref().map(|h| Directive {
                value: h.value.to_string(),
                span: h.span,
            }),
            ..ModuleFacts::default()
        },
        errors: Vec::new(),
        exported_declaration: None,
        commonjs: program.source_type.is_commonjs(),
        enclosing_function: None,
    };
    visitor.visit_program(program);
    eyre::ensure!(visitor.errors.is_empty(), "{}", visitor.errors.join("; "));
    let mut facts = visitor.facts;
    facts.bindings.sort_by_key(|b| b.id.0);
    for binding in &mut facts.bindings {
        binding.declarations.sort_by_key(|s| (s.start, s.end));
        binding.declarations.dedup();
    }
    facts.references.sort_by_key(|r| (r.span.start, r.span.end));
    facts.dependencies.sort_by_key(|d| d.id.0);
    facts.imports.sort_by_key(|i| i.span.start);
    facts
        .exports
        .sort_by_key(|e| (e.span.start, e.name_span.map(|s| s.start)));
    facts.effects = crate::effects::extract(program, &facts);
    Ok(facts)
}

struct Extractor<'s, 'a> {
    semantic: &'s Semantic<'a>,
    ids: HashMap<SymbolId, BindingId>,
    binding_indices: HashMap<BindingId, usize>,
    facts: ModuleFacts,
    errors: Vec<String>,
    exported_declaration: Option<(Span, bool)>,
    commonjs: bool,
    enclosing_function: Option<Span>,
}

impl Extractor<'_, '_> {
    fn binding(&self, id: &BindingIdentifier<'_>) -> BindingId {
        self.ids[&id.symbol_id.get().expect("validated binding")]
    }

    fn root_binding(&self, name: &str) -> Option<BindingId> {
        self.facts
            .bindings
            .iter()
            .find(|b| b.top_level && b.name == name)
            .map(|b| b.id)
    }

    fn dependency(
        &mut self,
        kind: DependencyKind,
        span: Span,
        source: &StringLiteral<'_>,
        type_only: bool,
    ) -> DependencyId {
        let id = DependencyId(span.start);
        self.facts.dependencies.push(Dependency {
            id,
            kind,
            span,
            specifier: source.value.to_string(),
            specifier_span: source.span,
            type_only,
            enclosing_function: self.enclosing_function,
        });
        id
    }

    fn reject(&mut self, span: Span, reason: &str) {
        self.errors
            .push(format!("{reason} at bytes {}..{}", span.start, span.end));
    }
}

impl<'a> Visit<'a> for Extractor<'_, 'a> {
    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        if let Some(symbol) = id.symbol_id.get() {
            let binding = self.ids[&symbol];
            self.facts.bindings[self.binding_indices[&binding]]
                .declarations
                .push(id.span);
            if let Some((span, type_only)) = self.exported_declaration {
                let info = &self.facts.bindings[self.binding_indices[&binding]];
                if info.top_level {
                    self.facts.exports.push(Export {
                        name: Some(id.name.to_string()),
                        value: ExportValue::Binding(binding),
                        span,
                        name_span: Some(id.span),
                        type_only: type_only || info.type_only,
                    });
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
        self.facts.references.push(Reference {
            name: id.name.to_string(),
            span: id.span,
            binding: reference.symbol_id().map(|s| self.ids[&s]),
            read: reference.is_read(),
            write: reference.is_write(),
            type_only,
            commonjs,
        });
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
                self.facts.imports.push(ImportBinding {
                    dependency,
                    binding: self.binding(specifier.local()),
                    imported,
                    span: specifier.span(),
                    imported_span,
                    type_only: decl.import_kind.is_type() || type_only,
                });
            }
        }
        walk::walk_import_declaration(self, decl);
    }

    fn visit_export_named_declaration(&mut self, decl: &ExportNamedDeclaration<'a>) {
        let dependency = decl.source.as_ref().map(|source| {
            self.dependency(
                DependencyKind::ReExport,
                decl.span,
                source,
                decl.export_kind.is_type(),
            )
        });
        for specifier in &decl.specifiers {
            let value = if let Some(dependency) = dependency {
                ExportValue::ReExport {
                    dependency,
                    imported: ImportedName::Name(specifier.local.name().to_string()),
                }
            } else if let Some(binding) = self.root_binding(&specifier.local.name()) {
                ExportValue::Binding(binding)
            } else {
                self.reject(specifier.span, "unresolved local export");
                continue;
            };
            let local_type_only = match &value {
                ExportValue::Binding(binding) => {
                    self.facts.bindings[self.binding_indices[binding]].type_only
                }
                _ => false,
            };
            self.facts.exports.push(Export {
                name: Some(specifier.exported.name().to_string()),
                value,
                span: specifier.span,
                name_span: Some(specifier.exported.span()),
                type_only: decl.export_kind.is_type()
                    || specifier.export_kind.is_type()
                    || local_type_only,
            });
        }
        let previous = self.exported_declaration;
        self.exported_declaration = decl.declaration.as_ref().map(|d| {
            (
                d.span(),
                decl.export_kind.is_type()
                    || matches!(
                        d,
                        Declaration::TSInterfaceDeclaration(_)
                            | Declaration::TSTypeAliasDeclaration(_)
                    ),
            )
        });
        walk::walk_export_named_declaration(self, decl);
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
        self.facts.exports.push(Export {
            name: Some("default".into()),
            value: binding.map_or(
                ExportValue::Expression(decl.declaration.span()),
                ExportValue::Binding,
            ),
            span: decl.span,
            name_span: None,
            type_only,
        });
        walk::walk_export_default_declaration(self, decl);
    }

    fn visit_export_all_declaration(&mut self, decl: &ExportAllDeclaration<'a>) {
        let dependency = self.dependency(
            DependencyKind::ReExport,
            decl.span,
            &decl.source,
            decl.export_kind.is_type(),
        );
        self.facts.exports.push(Export {
            name: decl.exported.as_ref().map(|n| n.name().to_string()),
            value: ExportValue::ReExport {
                dependency,
                imported: ImportedName::Namespace,
            },
            span: decl.span,
            name_span: decl.exported.as_ref().map(GetSpan::span),
            type_only: decl.export_kind.is_type(),
        });
        walk::walk_export_all_declaration(self, decl);
    }

    fn visit_import_expression(&mut self, expr: &ImportExpression<'a>) {
        if let Expression::StringLiteral(source) = expr.source.get_inner_expression() {
            self.dependency(DependencyKind::DynamicImport, expr.span, source, false);
        } else {
            self.reject(expr.span, "nonliteral dynamic import");
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
                    self.reject(expr.span, "unsupported require call (expected one literal argument, no optional call)");
                } else if let Some(Expression::StringLiteral(source)) = expr
                    .arguments
                    .first()
                    .and_then(Argument::as_expression)
                    .map(Expression::get_inner_expression)
                {
                    self.dependency(DependencyKind::Require, expr.span, source, false);
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
        walk::walk_function(self, function, flags);
        self.enclosing_function = previous;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let previous = self.enclosing_function.replace(function.span);
        walk::walk_arrow_function_expression(self, function);
        self.enclosing_function = previous;
    }
}
