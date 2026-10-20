use std::{collections::HashMap, path::Path};

use eyre::{ensure, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Semantic, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, Span};

use crate::solver::FunctionSignature;
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
    let mut context = Context::from_local_facts(facts)?;
    let values = semantic
        .scoping()
        .symbol_ids()
        .map(|symbol| {
            (
                symbol,
                NamedValue {
                    value: context.value(Hazards::NONE),
                    binding: None,
                },
            )
        })
        .collect();
    let mut collector = Bindings {
        context,
        semantic,
        values,
        mutation: false,
        missing_reference: false,
        functions: HashMap::new(),
        stack: Vec::new(),
    };
    collector.visit_program(program);
    ensure!(
        !collector.missing_reference,
        "effect extraction requires resolved OXC references"
    );
    if collector.mutation {
        collector.context.opaque_effects();
    }
    let this_value = collector.context.undefined();
    let slots = collector
        .values
        .iter()
        .map(|(symbol, named)| (*symbol, named.value))
        .collect();
    let mut extractor = Extractor {
        context: collector.context,
        semantic,
        values: collector.values,
        namespaces: HashMap::new(),
        roots: Vec::new(),
        matches: Vec::new(),
        functions: collector.functions,
        this_value,
        deferred: false,
        slots,
    };
    Compiler {
        extractor: &mut extractor,
    }
    .visit_program(program);
    if program.source_type.is_commonjs() {
        let effects = extractor.context.opaque_effects();
        extractor.roots.push(SourceRoot {
            span: program.span,
            value: extractor.context.unknown(),
            effects,
        });
    } else {
        extractor.hoist(&program.body);
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
    functions: HashMap<Span, FunctionInfo>,
    stack: Vec<Span>,
}

#[derive(Clone)]
struct FunctionInfo {
    index: usize,
    locals: Vec<SymbolId>,
    captures: Vec<ValueId>,
    receiver: ValueId,
    arrow: bool,
    self_binding: Option<ValueId>,
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
                            value: self.values[&symbol].value,
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
            let reference = self.semantic.scoping().get_reference(reference);
            if reference.is_write() && self.stack.is_empty() {
                self.mutation = true;
            }
            if let Some(symbol) = reference.symbol_id() {
                for span in &self.stack {
                    self.functions
                        .get_mut(span)
                        .expect("function")
                        .captures
                        .push(self.values[&symbol].value);
                }
            }
        } else {
            self.missing_reference = true;
        }
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        self.mutation |= self.stack.is_empty();
        walk::walk_assignment_expression(self, expression);
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        self.mutation |= self.stack.is_empty();
        walk::walk_update_expression(self, expression);
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        self.mutation |= self.stack.is_empty() && expression.operator.as_str() == "delete";
        walk::walk_unary_expression(self, expression);
    }

    fn visit_function(&mut self, function: &Function<'a>, _: ScopeFlags) {
        let declaration = function.r#type == FunctionType::FunctionDeclaration;
        if let Some(identifier) = &function.id {
            if let Some(symbol) = identifier.symbol_id.get() {
                if declaration {
                    self.visit_binding_identifier(identifier);
                }
                if declaration
                    && self.semantic.scoping().symbol_scope_id(symbol)
                        == self.semantic.scoping().root_scope_id()
                {
                    let binding = Binding::Local(identifier.name.to_string());
                    if self.context.definition(&binding).is_some() {
                        self.values.get_mut(&symbol).expect("symbol").binding = Some(binding);
                    }
                }
            }
        }
        let receiver = self.context.value(Hazards::NONE);
        self.functions.insert(
            function.span,
            FunctionInfo {
                index: self.context.reserve_function(),
                locals: Vec::new(),
                captures: Vec::new(),
                receiver,
                arrow: false,
                self_binding: (!declaration)
                    .then(|| {
                        function
                            .id
                            .as_ref()
                            .and_then(|id| id.symbol_id.get())
                            .map(|symbol| self.values[&symbol].value)
                    })
                    .flatten(),
            },
        );
        self.stack.push(function.span);
        self.visit_formal_parameters(&function.params);
        if let Some(body) = &function.body {
            self.visit_function_body(body);
        }
        self.stack.pop();
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let receiver = self.stack.last().map_or(self.context.undefined(), |span| {
            self.functions[span].receiver
        });
        self.functions.insert(
            function.span,
            FunctionInfo {
                index: self.context.reserve_function(),
                locals: Vec::new(),
                captures: vec![receiver],
                receiver,
                arrow: true,
                self_binding: None,
            },
        );
        self.stack.push(function.span);
        walk::walk_arrow_function_expression(self, function);
        self.stack.pop();
    }

    fn visit_binding_identifier(&mut self, identifier: &BindingIdentifier<'a>) {
        if let (Some(span), Some(symbol)) = (self.stack.last(), identifier.symbol_id.get()) {
            self.functions
                .get_mut(span)
                .expect("function")
                .locals
                .push(symbol);
        }
    }
}

struct Extractor<'s, 'a> {
    context: Context,
    semantic: &'s Semantic<'a>,
    values: HashMap<SymbolId, NamedValue>,
    namespaces: HashMap<Binding, ValueId>,
    roots: Vec<SourceRoot>,
    matches: Vec<DefinitionMatch>,
    functions: HashMap<Span, FunctionInfo>,
    this_value: ValueId,
    deferred: bool,
    slots: HashMap<SymbolId, ValueId>,
}

#[derive(Clone)]
struct Computed {
    named: NamedValue,
    effects: Effect,
}

impl<'a> Extractor<'_, 'a> {
    fn function(&mut self, function: &Function<'_>) -> ValueId {
        if let Some(id) = &function.id {
            if let Some(symbol) = id.symbol_id.get() {
                if let Some(binding) = &self.values[&symbol].binding {
                    if let Some(value) = self.context.definition(binding) {
                        return value;
                    }
                }
            }
        }
        self.context
            .function_value(self.functions[&function.span].index)
    }

    fn hoist(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            let function = match statement {
                Statement::FunctionDeclaration(function) => Some(function.as_ref()),
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::FunctionDeclaration(function)) => Some(function.as_ref()),
                    _ => None,
                },
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        Some(function.as_ref())
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(function) = function {
                if let Some(symbol) = function.id.as_ref().and_then(|id| id.symbol_id.get()) {
                    let value = self.function(function);
                    self.context.set_binding(self.slots[&symbol], value);
                }
            }
        }
    }

    fn function_statements(
        &mut self,
        statements: &[Statement<'a>],
        returns: &mut Vec<ValueId>,
        locals: &[ValueId],
    ) -> bool {
        self.hoist(statements);
        for statement in statements {
            if self.function_statement(statement, returns, locals) {
                return true;
            }
        }
        false
    }

    fn function_statement(
        &mut self,
        statement: &Statement<'a>,
        returns: &mut Vec<ValueId>,
        locals: &[ValueId],
    ) -> bool {
        match statement {
            Statement::ReturnStatement(statement) => {
                let value = if let Some(expression) = &statement.argument {
                    self.expression(expression)
                } else {
                    self.computed(self.context.undefined(), self.context.pure())
                };
                returns.push(value.named.value);
                self.roots.push(SourceRoot {
                    span: statement.span,
                    value: value.named.value,
                    effects: value.effects,
                });
                true
            }
            Statement::BlockStatement(block) => {
                self.function_statements(&block.body, returns, locals)
            }
            Statement::IfStatement(statement) => {
                let test = self.expression(&statement.test);
                self.roots.push(SourceRoot {
                    span: statement.test.span(),
                    value: test.named.value,
                    effects: test.effects,
                });
                let before: Vec<_> = locals
                    .iter()
                    .map(|slot| self.context.read_binding(*slot))
                    .collect();
                let names = self.values.clone();
                let consequent = self.function_statement(&statement.consequent, returns, locals);
                let left: Vec<_> = locals
                    .iter()
                    .map(|slot| self.context.read_binding(*slot))
                    .collect();
                let left_names = std::mem::replace(&mut self.values, names);
                for (slot, value) in locals.iter().zip(&before) {
                    self.context.set_binding(*slot, *value);
                }
                let alternate = statement
                    .alternate
                    .as_ref()
                    .is_some_and(|statement| self.function_statement(statement, returns, locals));
                for (slot, left) in locals.iter().zip(left) {
                    let right = self.context.read_binding(*slot);
                    let value = match (consequent, alternate) {
                        (false, false) => self.context.join_values(&[left, right]),
                        (false, true) => left,
                        _ => right,
                    };
                    self.context.set_binding(*slot, value);
                }
                for (symbol, named) in &mut self.values {
                    if !consequent && (alternate || named.binding != left_names[symbol].binding) {
                        named.binding = if alternate {
                            left_names[symbol].binding.clone()
                        } else {
                            None
                        };
                    }
                }
                consequent && alternate
            }
            Statement::ThrowStatement(statement) => {
                let value = self.expression(&statement.argument);
                self.roots.push(SourceRoot {
                    span: statement.span,
                    value: value.named.value,
                    effects: value.effects,
                });
                true
            }
            _ => {
                self.visit_statement(statement);
                false
            }
        }
    }

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
            Expression::ThisExpression(_) => self.computed(self.this_value, self.context.pure()),
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
                    } else if self.deferred {
                        named.value = self.context.read_binding(named.value);
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
            Expression::FunctionExpression(function) => {
                let value = self.function(function);
                self.computed(value, self.context.pure())
            }
            Expression::ArrowFunctionExpression(function) => {
                let value = self
                    .context
                    .function_value(self.functions[&function.span].index);
                self.computed(value, self.context.pure())
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
                let slot = self.slots[&symbol];
                self.context.set_binding(slot, value.named.value);
                let mut named = value.named.clone();
                if self.deferred {
                    named.value = slot;
                }
                self.values.insert(symbol, named);
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

struct Compiler<'e, 's, 'a> {
    extractor: &'e mut Extractor<'s, 'a>,
}

impl<'a> Compiler<'_, '_, 'a> {
    fn compile(
        &mut self,
        span: Span,
        parameters: &FormalParameters<'a>,
        body: Option<&FunctionBody<'a>>,
        unsupported: bool,
        expression: bool,
    ) {
        let extractor = &mut self.extractor;
        let info = extractor.functions[&span].clone();
        let values: Vec<_> = info
            .locals
            .iter()
            .map(|symbol| (*symbol, extractor.values[symbol].clone()))
            .collect();
        let roots = std::mem::take(&mut extractor.roots);
        let namespaces = std::mem::take(&mut extractor.namespaces);
        let receiver = extractor.this_value;
        extractor.this_value = info.receiver;
        extractor.deferred = true;
        extractor.context.start_function();
        let mut arguments = Vec::new();
        let mut unsupported = unsupported || parameters.rest.is_some() || body.is_none();
        for parameter in &parameters.items {
            if let BindingPattern::BindingIdentifier(id) = &parameter.pattern {
                arguments.push(extractor.slots[&id.symbol_id.get().expect("parameter symbol")]);
            } else {
                unsupported = true;
            }
            unsupported |= parameter.initializer.is_some();
        }
        for local in &info.locals {
            let slot = extractor.slots[local];
            if !arguments.contains(&slot) {
                extractor.context.function_local(slot);
            }
        }
        let mut returns = Vec::new();
        if unsupported {
            let value = extractor.unknown();
            returns.push(value.named.value);
            extractor.roots.push(SourceRoot {
                span,
                value: value.named.value,
                effects: value.effects,
            });
        } else if let Some(body) = body {
            if expression {
                if let Some(Statement::ExpressionStatement(statement)) = body.statements.first() {
                    let value = extractor.expression(&statement.expression);
                    returns.push(value.named.value);
                    extractor.roots.push(SourceRoot {
                        span,
                        value: value.named.value,
                        effects: value.effects,
                    });
                }
            } else if !extractor.function_statements(
                &body.statements,
                &mut returns,
                &info
                    .locals
                    .iter()
                    .map(|symbol| extractor.slots[symbol])
                    .collect::<Vec<_>>(),
            ) {
                returns.push(extractor.context.undefined());
            }
        }
        let effects = extractor
            .context
            .any(extractor.roots.iter().map(|root| root.effects));
        let mut captures = info.captures;
        let locals: std::collections::HashSet<_> = info
            .locals
            .iter()
            .map(|symbol| extractor.slots[symbol])
            .collect();
        captures.retain(|value| !locals.contains(value) && Some(*value) != info.self_binding);
        captures.sort_unstable();
        captures.dedup();
        extractor.context.finish_function(
            info.index,
            FunctionSignature {
                parameters: arguments,
                receiver: (!info.arrow).then_some(info.receiver),
                self_binding: info.self_binding,
                captures,
            },
            returns,
            effects,
        );
        for (symbol, named) in values {
            extractor.values.insert(symbol, named);
        }
        extractor.roots = roots;
        extractor.namespaces = namespaces;
        extractor.this_value = receiver;
        extractor.deferred = false;
    }
}

impl<'a> Visit<'a> for Compiler<'_, '_, 'a> {
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        self.compile(
            function.span,
            &function.params,
            function.body.as_deref(),
            function.r#async || function.generator,
            false,
        );
        walk::walk_function(self, function, flags);
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.compile(
            function.span,
            &function.params,
            Some(&function.body),
            function.r#async,
            function.expression,
        );
        walk::walk_arrow_function_expression(self, function);
    }
}
