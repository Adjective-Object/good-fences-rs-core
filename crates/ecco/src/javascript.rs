use std::{collections::HashMap, path::Path};

use eyre::{ensure, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Semantic, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, Span};

use crate::*;

#[derive(Debug, Clone)]
pub struct SourceRoot {
    pub span: Span,
    pub value: ValueId,
    pub effects: Effect,
}

#[derive(Debug, Clone)]
pub struct DefinitionMatch {
    pub span: Span,
    pub binding: Binding,
}

#[derive(Debug, Clone)]
pub struct SourceAnalysis {
    pub context: Context,
    pub roots: Vec<SourceRoot>,
    pub matches: Vec<DefinitionMatch>,
}

/// Parse using the repository's extension-sensitive ESM policy. Import evaluation
/// and cross-file linking remain caller responsibilities.
pub fn analyze_source(source: &str, path: &Path, facts: LocalFacts) -> Result<SourceAnalysis> {
    let allocator = Allocator::default();
    let parsed = oxc_utils_parse::parse_bundler_file(
        &allocator,
        source,
        path,
        oxc_utils_parse::ModuleFormat::Esm,
    )?;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    ensure!(
        semantic.errors.is_empty(),
        "{}: semantic diagnostics: {:?}",
        path.display(),
        semantic.errors
    );
    extract_from_ast(&parsed.program, &semantic.semantic, facts)
}

/// Extract owned roots from a program and its checked semantic model, without
/// reparsing or retaining arena-backed data.
pub fn extract_from_ast<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    facts: LocalFacts,
) -> Result<SourceAnalysis> {
    let mut collector = Bindings {
        context: Context::from_local_facts(facts)?,
        semantic,
        values: HashMap::new(),
        mutation: false,
        missing_reference: false,
    };
    collector.visit_program(program);
    ensure!(
        !collector.missing_reference,
        "effect extraction requires resolved OXC references"
    );
    if collector.mutation {
        collector.context.opaque_effects();
    }
    let mut extractor = Extractor {
        context: collector.context,
        semantic,
        values: collector.values,
        namespaces: HashMap::new(),
        roots: Vec::new(),
        matches: Vec::new(),
    };
    if program.source_type.is_commonjs() {
        let effects = extractor.context.opaque_effects();
        extractor.roots.push(SourceRoot {
            span: program.span,
            value: extractor.context.unknown(),
            effects,
        });
    } else {
        extractor.visit_program(program);
    }
    Ok(SourceAnalysis {
        context: extractor.context,
        roots: extractor.roots,
        matches: extractor.matches,
    })
}

#[derive(Clone)]
struct NamedValue {
    value: ValueId,
    binding: Option<Binding>,
}

struct Bindings<'s, 'a> {
    context: Context,
    semantic: &'s Semantic<'a>,
    values: HashMap<SymbolId, NamedValue>,
    mutation: bool,
    missing_reference: bool,
}

impl<'a> Visit<'a> for Bindings<'_, 'a> {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        if declaration.import_kind.is_type() {
            return;
        }
        if let Some(specifiers) = &declaration.specifiers {
            for specifier in specifiers {
                let path = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(s) if s.import_kind.is_type() => {
                        continue
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        vec![s.imported.name().to_string()]
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => vec!["default".into()],
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => Vec::new(),
                };
                if let Some(symbol) = specifier.local().symbol_id.get() {
                    self.values.insert(
                        symbol,
                        NamedValue {
                            value: self.context.unknown(),
                            binding: Some(Binding::Module {
                                module: declaration.source.value.to_string(),
                                path,
                            }),
                        },
                    );
                }
            }
        }
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if let Some(reference) = identifier.reference_id.get() {
            if self.semantic.scoping().get_reference(reference).is_write() {
                self.mutation = true;
            }
        } else {
            self.missing_reference = true;
        }
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        self.mutation = true;
        walk::walk_assignment_expression(self, expression);
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        self.mutation = true;
        walk::walk_update_expression(self, expression);
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        self.mutation |= expression.operator.as_str() == "delete";
        walk::walk_unary_expression(self, expression);
    }

    fn visit_function(&mut self, function: &Function<'a>, _: ScopeFlags) {
        if let Some(identifier) = &function.id {
            if let Some(symbol) = identifier.symbol_id.get() {
                if self.semantic.scoping().symbol_scope_id(symbol)
                    != self.semantic.scoping().root_scope_id()
                {
                    return;
                }
                let binding = Binding::Local(identifier.name.to_string());
                let value = self
                    .context
                    .definition(&binding)
                    .unwrap_or(self.context.unknown());
                self.values.insert(
                    symbol,
                    NamedValue {
                        value,
                        binding: Some(binding),
                    },
                );
            }
        }
    }

    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
}

struct Extractor<'s, 'a> {
    context: Context,
    semantic: &'s Semantic<'a>,
    values: HashMap<SymbolId, NamedValue>,
    namespaces: HashMap<Binding, ValueId>,
    roots: Vec<SourceRoot>,
    matches: Vec<DefinitionMatch>,
}

#[derive(Clone)]
struct Computed {
    named: NamedValue,
    effects: Effect,
}

impl Extractor<'_, '_> {
    fn computed(&self, value: ValueId, effects: Effect) -> Computed {
        Computed {
            named: NamedValue {
                value,
                binding: None,
            },
            effects,
        }
    }

    fn pure_value(&mut self, hazards: Hazards) -> Computed {
        let value = self.context.value(hazards);
        self.computed(value, self.context.pure())
    }

    fn unknown(&mut self) -> Computed {
        let effects = self.context.opaque_effects();
        self.computed(self.context.unknown(), effects)
    }

    fn resolve(&mut self, binding: &Binding, span: Span) -> ValueId {
        if let Some(value) = self.context.definition(binding) {
            self.matches.push(DefinitionMatch {
                span,
                binding: binding.clone(),
            });
            return value;
        }
        if let Some(value) = self.namespaces.get(binding) {
            return *value;
        }
        let prefix = self.context.facts().definitions.iter().any(|definition| {
            match (binding, &definition.binding) {
                (Binding::Global(prefix), Binding::Global(path)) => path.starts_with(prefix),
                (
                    Binding::Module {
                        module: left,
                        path: prefix,
                    },
                    Binding::Module {
                        module: right,
                        path,
                    },
                ) => left == right && path.starts_with(prefix),
                _ => false,
            }
        });
        if prefix {
            // Trusted environment namespaces have ordinary member reads. Their
            // identities are still poisoned by unknown code or replacement.
            let mut hazards = Hazards::UNKNOWN;
            hazards.0[Hazard::PropertyRead as usize] = false;
            let value = self.context.value(hazards);
            self.namespaces.insert(binding.clone(), value);
            value
        } else {
            self.context.unknown()
        }
    }

    fn member_binding(binding: Option<Binding>, property: &str) -> Option<Binding> {
        match binding? {
            Binding::Global(mut path) => {
                path.push(property.into());
                Some(Binding::Global(path))
            }
            Binding::Module { module, mut path } => {
                path.push(property.into());
                Some(Binding::Module { module, path })
            }
            Binding::Local(_) => None,
        }
    }

    fn member(
        &mut self,
        object: Computed,
        property: Option<&str>,
        key: Option<Computed>,
        span: Span,
    ) -> Computed {
        let binding =
            property.and_then(|property| Self::member_binding(object.named.binding, property));
        let value = binding.as_ref().map_or(self.context.unknown(), |binding| {
            self.resolve(binding, span)
        });
        let mut effects = vec![
            object.effects,
            self.context
                .operation(object.named.value, Hazard::PropertyRead),
        ];
        if let Some(key) = key {
            effects.push(key.effects);
            effects.push(self.context.operation(key.named.value, Hazard::ToPrimitive));
        }
        let effects = self.context.any(effects);
        Computed {
            named: NamedValue { value, binding },
            effects,
        }
    }

    fn expression(&mut self, expression: &Expression<'_>) -> Computed {
        match expression.get_inner_expression() {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::StringLiteral(_) => self.pure_value(Hazards::NONE),
            Expression::Identifier(identifier) => {
                let reference = self
                    .semantic
                    .scoping()
                    .get_reference(identifier.reference_id.get().expect("checked reference"));
                let named = if let Some(symbol) = reference.symbol_id() {
                    let mut named = self.values.get(&symbol).cloned().unwrap_or(NamedValue {
                        value: self.context.unknown(),
                        binding: None,
                    });
                    if let Some(binding) = &named.binding {
                        named.value = self.resolve(binding, identifier.span);
                    }
                    named
                } else {
                    let binding = Binding::Global(vec![identifier.name.to_string()]);
                    let value = if identifier.name == "undefined" {
                        self.context.undefined()
                    } else {
                        self.resolve(&binding, identifier.span)
                    };
                    NamedValue {
                        value,
                        binding: Some(binding),
                    }
                };
                Computed {
                    named,
                    effects: self.context.pure(),
                }
            }
            Expression::StaticMemberExpression(member) => {
                let object = self.expression(&member.object);
                self.member(
                    object,
                    Some(member.property.name.as_str()),
                    None,
                    member.span,
                )
            }
            Expression::ComputedMemberExpression(member) => {
                let object = self.expression(&member.object);
                let key = self.expression(&member.expression);
                let property = match member.expression.get_inner_expression() {
                    Expression::StringLiteral(literal) => Some(literal.value.as_str()),
                    _ => None,
                };
                self.member(object, property, Some(key), member.span)
            }
            Expression::CallExpression(call) => {
                // Evaluate a member receiver once, not once for matching and again
                // for `this`: either evaluation can allocate or mutate identities.
                let (callee, receiver) = match call.callee.get_inner_expression() {
                    Expression::StaticMemberExpression(member) => {
                        let object = self.expression(&member.object);
                        let receiver = object.named.value;
                        (
                            self.member(
                                object,
                                Some(member.property.name.as_str()),
                                None,
                                member.span,
                            ),
                            Some(receiver),
                        )
                    }
                    Expression::ComputedMemberExpression(member) => {
                        let object = self.expression(&member.object);
                        let receiver = object.named.value;
                        let key = self.expression(&member.expression);
                        let property = match member.expression.get_inner_expression() {
                            Expression::StringLiteral(literal) => Some(literal.value.as_str()),
                            _ => None,
                        };
                        (
                            self.member(object, property, Some(key), member.span),
                            Some(receiver),
                        )
                    }
                    _ => (self.expression(&call.callee), None),
                };
                let mut effects = vec![callee.effects];
                let mut arguments = Vec::new();
                let mut spread = false;
                for argument in &call.arguments {
                    if let Argument::SpreadElement(element) = argument {
                        let value = self.expression(&element.argument);
                        effects.push(value.effects);
                        effects.push(self.context.operation(value.named.value, Hazard::Iterate));
                        spread = true;
                    } else {
                        let value =
                            self.expression(argument.as_expression().expect("expression argument"));
                        effects.push(value.effects);
                        arguments.push(value.named.value);
                    }
                }
                let result = if spread {
                    CallResult {
                        value: self.context.unknown(),
                        effects: self.context.opaque_effects(),
                    }
                } else {
                    self.context.call(callee.named.value, &arguments, receiver)
                };
                effects.push(result.effects);
                let effects = self.context.any(effects);
                self.computed(result.value, effects)
            }
            Expression::BinaryExpression(binary) => {
                let left = self.expression(&binary.left);
                let right = self.expression(&binary.right);
                let mut effects = vec![left.effects, right.effects];
                match binary.operator.as_str() {
                    "===" | "!==" => {}
                    "in" => {
                        effects.push(
                            self.context
                                .operation(left.named.value, Hazard::ToPrimitive),
                        );
                        effects.push(self.context.operation(right.named.value, Hazard::In));
                    }
                    "instanceof" => {
                        effects.push(
                            self.context
                                .operation(right.named.value, Hazard::Instanceof),
                        );
                        effects.push(
                            self.context
                                .operation(left.named.value, Hazard::PropertyRead),
                        );
                    }
                    _ => {
                        effects.push(
                            self.context
                                .operation(left.named.value, Hazard::ToPrimitive),
                        );
                        effects.push(
                            self.context
                                .operation(right.named.value, Hazard::ToPrimitive),
                        );
                    }
                }
                let effects = self.context.any(effects);
                let value = self.context.value(Hazards::NONE);
                self.computed(value, effects)
            }
            Expression::UnaryExpression(unary) if unary.operator.as_str() != "delete" => {
                let argument = self.expression(&unary.argument);
                let effects = if matches!(unary.operator.as_str(), "!" | "void" | "typeof") {
                    argument.effects
                } else {
                    let coercion = self
                        .context
                        .operation(argument.named.value, Hazard::ToPrimitive);
                    self.context.any([argument.effects, coercion])
                };
                let value = self.context.value(Hazards::NONE);
                self.computed(value, effects)
            }
            Expression::LogicalExpression(logical) => {
                let left = self.expression(&logical.left);
                let right = self.expression(&logical.right);
                let value = self
                    .context
                    .join_values(&[left.named.value, right.named.value]);
                let effects = self.context.any([left.effects, right.effects]);
                self.computed(value, effects)
            }
            Expression::ConditionalExpression(conditional) => {
                let test = self.expression(&conditional.test);
                let left = self.expression(&conditional.consequent);
                let right = self.expression(&conditional.alternate);
                let value = self
                    .context
                    .join_values(&[left.named.value, right.named.value]);
                let effects = self
                    .context
                    .any([test.effects, left.effects, right.effects]);
                self.computed(value, effects)
            }
            Expression::SequenceExpression(sequence) => {
                let mut effects = Vec::new();
                let mut last = self.computed(self.context.undefined(), self.context.pure());
                for expression in &sequence.expressions {
                    last = self.expression(expression);
                    effects.push(last.effects);
                }
                last.effects = self.context.any(effects);
                last
            }
            Expression::TemplateLiteral(template) => {
                let mut effects = Vec::new();
                for expression in &template.expressions {
                    let value = self.expression(expression);
                    effects.push(value.effects);
                    effects.push(
                        self.context
                            .operation(value.named.value, Hazard::ToPrimitive),
                    );
                }
                let effects = self.context.any(effects);
                let value = self.context.value(Hazards::NONE);
                self.computed(value, effects)
            }
            Expression::ObjectExpression(object) => {
                let mut effects = Vec::new();
                let mut getter = false;
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            getter |= property.kind == PropertyKind::Get;
                            getter |= !property.computed
                                && property
                                    .key
                                    .static_name()
                                    .is_some_and(|name| name == "__proto__");
                            if property.computed {
                                if let Some(key) = property.key.as_expression() {
                                    let key = self.expression(key);
                                    effects.push(key.effects);
                                    effects.push(
                                        self.context
                                            .operation(key.named.value, Hazard::ToPrimitive),
                                    );
                                }
                            }
                            let value = self.expression(&property.value);
                            effects.push(value.effects);
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            let value = self.expression(&spread.argument);
                            effects.push(value.effects);
                            // Enumeration and property getter execution are not
                            // represented by the iterable hazard.
                            effects.push(self.context.opaque_effects());
                            getter = true;
                        }
                    }
                }
                let mut hazards = Hazards::UNKNOWN;
                hazards.0[Hazard::PropertyRead as usize] = getter;
                hazards.0[Hazard::ToPrimitive as usize] = !object.properties.is_empty();
                let value = self.context.value(hazards);
                let effects = self.context.any(effects);
                self.computed(value, effects)
            }
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                self.pure_value(Hazards::UNKNOWN)
            }
            _ => self.unknown(),
        }
    }
}

impl<'a> Visit<'a> for Extractor<'_, 'a> {
    fn visit_declaration(&mut self, declaration: &Declaration<'a>) {
        match declaration {
            Declaration::VariableDeclaration(declaration) => {
                self.visit_variable_declaration(declaration)
            }
            Declaration::FunctionDeclaration(_) => {
                self.roots.push(SourceRoot {
                    span: declaration.span(),
                    value: self.context.undefined(),
                    effects: self.context.pure(),
                });
            }
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {}
            _ => {
                let value = self.unknown();
                self.roots.push(SourceRoot {
                    span: declaration.span(),
                    value: value.named.value,
                    effects: value.effects,
                });
            }
        }
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::ExpressionStatement(statement) => {
                let value = self.expression(&statement.expression);
                self.roots.push(SourceRoot {
                    span: statement.span,
                    value: value.named.value,
                    effects: value.effects,
                });
            }
            Statement::VariableDeclaration(declaration) => {
                self.visit_variable_declaration(declaration)
            }
            Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {
                self.roots.push(SourceRoot {
                    span: statement.span(),
                    value: self.context.undefined(),
                    effects: self.context.pure(),
                });
            }
            Statement::ImportDeclaration(_) | Statement::ExportAllDeclaration(_) => {}
            Statement::ExportNamedDeclaration(declaration) => {
                if let Some(declaration) = &declaration.declaration {
                    self.visit_declaration(declaration);
                }
            }
            Statement::ExportDefaultDeclaration(declaration) => {
                if let Some(expression) = declaration.declaration.as_expression() {
                    let value = self.expression(expression);
                    self.roots.push(SourceRoot {
                        span: declaration.span,
                        value: value.named.value,
                        effects: value.effects,
                    });
                } else if matches!(
                    declaration.declaration,
                    ExportDefaultDeclarationKind::FunctionDeclaration(_)
                ) {
                    self.roots.push(SourceRoot {
                        span: declaration.span,
                        value: self.context.undefined(),
                        effects: self.context.pure(),
                    });
                } else {
                    let value = self.unknown();
                    self.roots.push(SourceRoot {
                        span: declaration.span,
                        value: value.named.value,
                        effects: value.effects,
                    });
                }
            }
            Statement::TSTypeAliasDeclaration(_) | Statement::TSInterfaceDeclaration(_) => {}
            _ => {
                let value = self.unknown();
                self.roots.push(SourceRoot {
                    span: statement.span(),
                    value: value.named.value,
                    effects: value.effects,
                });
            }
        }
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        if matches!(
            declaration.kind,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        ) {
            let value = self.unknown();
            self.roots.push(SourceRoot {
                span: declaration.span,
                value: value.named.value,
                effects: value.effects,
            });
        } else {
            for declarator in &declaration.declarations {
                self.visit_variable_declarator(declarator);
            }
        }
    }

    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        let value = if let Some(initializer) = &declaration.init {
            self.expression(initializer)
        } else {
            self.computed(self.context.undefined(), self.context.pure())
        };
        let effects = if let BindingPattern::BindingIdentifier(identifier) = &declaration.id {
            if let Some(symbol) = identifier.symbol_id.get() {
                self.values.insert(symbol, value.named.clone());
            }
            value.effects
        } else {
            self.context.opaque_effects()
        };
        self.roots.push(SourceRoot {
            span: declaration.span,
            value: value.named.value,
            effects,
        });
    }

    fn visit_function(&mut self, _: &Function<'a>, _: ScopeFlags) {}
    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
}
