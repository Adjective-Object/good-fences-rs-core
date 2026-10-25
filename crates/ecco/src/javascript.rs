use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use eyre::{ensure, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{ScopeFlags, Semantic, SemanticBuilder, SymbolId as OxcSymbolId};
use oxc_span::{GetSpan, Span};

use crate::frontier::{FlatVisit, Mode, Op, Plan, Role, Step};
use crate::solver::FunctionSignature;
use crate::*;

#[derive(Debug, Clone)]
pub struct SourceRoot {
    pub span: Span,
    pub value: ValueId,
    pub effects: Effect,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DefinitionMatch {
    #[serde(with = "crate::analysis::span")]
    span: Span,
    binding: Binding,
}

impl DefinitionMatch {
    pub(crate) fn new(span: Span, binding: Binding) -> Self {
        Self { span, binding }
    }
}

impl DefinitionMatch {
    pub fn span(&self) -> Span {
        self.span
    }

    pub fn binding(&self) -> &Binding {
        &self.binding
    }
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
    crate::analysis::analyze_parsed(parse_source(source, path, facts)?)
}

/// Parse source into saveable lazy operations, without analyzing source calls.
pub fn parse_source(source: &str, path: &Path, facts: LocalFacts) -> Result<ParsedFile> {
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
        semantic.diagnostics.is_empty(),
        "{}: semantic diagnostics: {:?}",
        path.display(),
        semantic.diagnostics
    );
    parse_from_ast(&parsed.program, &semantic.semantic, facts)
}

/// Analyze a single checked program without reparsing. Use `parse_from_ast`
/// instead when extracting precursors for a connected graph.
pub fn extract_from_ast<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    facts: LocalFacts,
) -> Result<SourceAnalysis> {
    crate::analysis::analyze_parsed(parse_from_ast(program, semantic, facts)?)
}

/// Extract owned lazy definitions using an externally supplied checked AST.
pub fn parse_from_ast<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    mut facts: LocalFacts,
) -> Result<ParsedFile> {
    let mut contracts: HashSet<_> = facts
        .definitions()
        .iter()
        .map(|definition| definition.binding().clone())
        .collect();
    let (inferred, usable) = crate::inference::infer_contracts(program, semantic, &facts)?;
    contracts.extend(usable);
    facts.definitions_mut().extend(inferred.into_parts().0);
    let mut context = Context::recording_context(facts)?;
    let values = semantic
        .scoping()
        .symbol_ids()
        .map(|symbol| {
            let value = context.value(Hazards::NONE);
            context.set_binding(value, context.undefined());
            (
                symbol,
                NamedValue {
                    value,
                    binding: None,
                },
            )
        })
        .collect();
    let mut collector = Bindings {
        contracts,
        context,
        semantic,
        values,
        mutation: false,
        missing_reference: false,
        functions: HashMap::new(),
        stack: Vec::new(),
        expressions: None,
    };
    collector.visit_program(program);
    ensure!(
        !collector.missing_reference,
        "effect extraction requires resolved OXC references"
    );
    let this_value = collector.context.undefined();
    let slots = collector
        .values
        .iter()
        .map(|(symbol, named)| (*symbol, named.value))
        .collect();
    let mut extractor = Extractor {
        contracts: collector.contracts,
        context: collector.context,
        semantic,
        values: collector.values,
        roots: Vec::new(),
        matches: Vec::new(),
        functions: collector.functions,
        this_value,
        arguments_value: None,
        deferred: false,
        slots,
        parsed_roots: Vec::new(),
        externals: Vec::new(),
    };
    let mut symbols: Vec<_> = semantic
        .scoping()
        .symbol_ids()
        .filter(|symbol| {
            semantic.scoping().symbol_scope_id(*symbol) == semantic.scoping().root_scope_id()
                && !matches!(
                    extractor.values[symbol].binding,
                    Some(Binding::Module { .. })
                )
        })
        .map(|symbol| {
            let span = semantic.scoping().symbol_span(symbol);
            ParsedSymbol::new(
                SymbolId(span.start),
                semantic.scoping().symbol_name(symbol).to_owned(),
                span,
                extractor.slots[&symbol],
            )
        })
        .collect();
    symbols.sort_by_key(|symbol| symbol.id().0);
    let values = extractor.context.parsed_values();
    Compiler {
        extractor: &mut extractor,
        expressions: None,
    }
    .visit_program(program);
    extractor.context.start_function();
    if collector.mutation {
        extractor.context.opaque_effects();
    }
    if !program.source_type.is_commonjs() {
        extractor.hoist(&program.body);
    }
    let setup = extractor.context.finish_recording(
        FunctionSignature::default(),
        Vec::new(),
        extractor.context.pure(),
    );
    extractor.context.start_function();
    if program.source_type.is_commonjs() {
        let effects = extractor.context.opaque_effects();
        extractor.push_root(SourceRoot {
            span: program.span,
            value: extractor.context.unknown(),
            effects,
        });
    } else {
        extractor.visit_program(program);
    }
    extractor.context.finish_recording(
        FunctionSignature::default(),
        Vec::new(),
        extractor.context.pure(),
    );
    Ok(ParsedFile::new(
        1,
        extractor.context.facts().clone(),
        values,
        extractor.context.owned_functions(),
        setup,
        symbols,
        extractor.parsed_roots,
        extractor.externals,
        extractor.matches,
    ))
}

/// Extract from the same checked AST as a segmenter, preserving declarator
/// boundaries without reconstructing or reparsing source.
pub fn parse_from_ast_with_segments<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    facts: LocalFacts,
    segments: &[Span],
) -> Result<ParsedFile> {
    let parsed = parse_from_ast(program, semantic, facts)?;
    let mut occupied = std::collections::HashSet::new();
    for root in parsed.roots() {
        let segment = segments
            .iter()
            .position(|span| span.start <= root.span().start && root.span().end <= span.end)
            .ok_or_else(|| eyre::eyre!("Ecco root outside segment ownership"))?;
        ensure!(
            occupied.insert(segment),
            "multiple Ecco roots in one segment"
        );
    }
    parsed.validate()?;
    Ok(parsed)
}

#[derive(Clone)]
struct NamedValue {
    value: ValueId,
    binding: Option<Binding>,
}

struct Bindings<'s, 'a> {
    contracts: HashSet<Binding>,
    context: Context,
    semantic: &'s Semantic<'a>,
    values: HashMap<OxcSymbolId, NamedValue>,
    mutation: bool,
    missing_reference: bool,
    functions: HashMap<Span, FunctionInfo>,
    stack: Vec<Span>,
    expressions: Option<Vec<&'a Expression<'a>>>,
}

#[derive(Clone)]
struct FunctionInfo {
    index: usize,
    locals: Vec<OxcSymbolId>,
    captures: Vec<ValueId>,
    receiver: ValueId,
    arrow: bool,
    self_binding: Option<ValueId>,
    arguments: Option<ValueId>,
}

impl<'a> FlatVisit<'a> for Bindings<'_, 'a> {
    fn expression_queue(&mut self) -> &mut Option<Vec<&'a Expression<'a>>> {
        &mut self.expressions
    }
}

impl<'a> Visit<'a> for Bindings<'_, 'a> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        self.flat_expression(expression);
    }
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        if declaration.import_kind.is_type() {
            return;
        }
        let Some(module) = declaration.source.value.as_str() else {
            return;
        };
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
                                module: module.to_owned(),
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
        let expressions = self.expressions.take();
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
                    if self.contracts.contains(&binding) {
                        self.values.get_mut(&symbol).expect("symbol").binding = Some(binding);
                    }
                }
            }
        }
        let receiver = self.context.value(Hazards::NONE);
        let arguments = self.context.value(Hazards::UNKNOWN);
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
                arguments: Some(arguments),
            },
        );
        self.stack.push(function.span);
        self.visit_formal_parameters(&function.params);
        if let Some(body) = &function.body {
            self.visit_function_body(body);
        }
        self.stack.pop();
        self.expressions = expressions;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let expressions = self.expressions.take();
        let receiver = self.stack.last().map_or(self.context.undefined(), |span| {
            self.functions[span].receiver
        });
        let arguments = self
            .stack
            .last()
            .and_then(|span| self.functions[span].arguments);
        let mut captures = vec![receiver];
        captures.extend(arguments);
        self.functions.insert(
            function.span,
            FunctionInfo {
                index: self.context.reserve_function(),
                locals: Vec::new(),
                captures,
                receiver,
                arrow: true,
                self_binding: None,
                arguments,
            },
        );
        self.stack.push(function.span);
        walk::walk_arrow_function_expression(self, function);
        self.stack.pop();
        self.expressions = expressions;
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
    contracts: HashSet<Binding>,
    context: Context,
    semantic: &'s Semantic<'a>,
    values: HashMap<OxcSymbolId, NamedValue>,
    roots: Vec<SourceRoot>,
    matches: Vec<DefinitionMatch>,
    functions: HashMap<Span, FunctionInfo>,
    this_value: ValueId,
    arguments_value: Option<ValueId>,
    deferred: bool,
    slots: HashMap<OxcSymbolId, ValueId>,
    parsed_roots: Vec<ParsedRoot>,
    externals: Vec<ExternalReference>,
}

#[derive(Clone)]
struct Computed {
    named: NamedValue,
    effects: Effect,
}

struct Operand {
    computed: Computed,
    receiver: Option<ValueId>,
    extra: Option<Effect>,
}

impl<'a> Extractor<'_, 'a> {
    fn push_root(&mut self, root: SourceRoot) {
        if self.deferred {
            self.roots.push(root);
        } else {
            let function = self.context.finish_recording(
                FunctionSignature::default(),
                vec![root.value],
                root.effects,
            );
            self.parsed_roots.push(ParsedRoot::new(root.span, function));
            self.context.start_function();
        }
    }

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
                Statement::ExportDeclaration(export) => match &export.declaration {
                    Declaration::FunctionDeclaration(function) => Some(function.as_ref()),
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
                self.context.retain_captures(value.named.value, locals);
                returns.push(value.named.value);
                self.push_root(SourceRoot {
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
                self.push_root(SourceRoot {
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
                self.push_root(SourceRoot {
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

    fn global_read(&mut self, name: &str, span: Span) -> Effect {
        let Some(symbol) = self
            .context
            .facts()
            .global_symbols()
            .iter()
            .find(|symbol| symbol.name() == name)
        else {
            return self.context.pure();
        };
        let read_effects = symbol.read_effects();
        self.matches.push(DefinitionMatch::new(
            span,
            Binding::Global(vec![name.to_owned()]),
        ));
        if read_effects {
            self.context.opaque_effects()
        } else {
            self.context.pure()
        }
    }

    fn resolve(&mut self, binding: &Binding, span: Span) -> ValueId {
        if let Some(value) = self.context.definition(binding) {
            self.matches
                .push(DefinitionMatch::new(span, binding.clone()));
            if matches!(binding, Binding::Local(_)) {
                return value;
            }
        }
        let reference = self.externals.len();
        self.externals
            .push(ExternalReference::new(binding.clone(), span));
        self.context.external_value(reference)
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
        let limit = (expression.span().size() as usize)
            .saturating_mul(2)
            .saturating_add(1);
        let Some(plan) = Plan::lower(expression, self.semantic, Mode::Recording, limit) else {
            return self.unknown();
        };
        let mut results: Vec<Option<Operand>> = Vec::new();
        results.resize_with(plan.nodes.len(), || None);
        for step in &plan.steps {
            match *step {
                Step::Enter(_) => {}
                Step::Input(input) => {
                    let input = &plan.inputs[input];
                    let value = results[input.node].as_mut().expect("completed input");
                    value.extra = Some(match input.role {
                        Role::Coerce => self
                            .context
                            .operation(value.computed.named.value, Hazard::ToPrimitive),
                        Role::Iterate => self
                            .context
                            .operation(value.computed.named.value, Hazard::Iterate),
                        Role::Enumerate => self.context.opaque_effects(),
                        Role::Value => unreachable!("undecorated input"),
                    });
                }
                Step::Ready(id) => {
                    let inputs = plan.inputs[plan.nodes[id].inputs.clone()]
                        .iter()
                        .map(|input| results[input.node].take().expect("ready expression input"));
                    let value = self.expression_transfer(&plan.nodes[id].op, inputs);
                    results[id] = Some(value);
                }
            }
        }
        results[0].take().expect("root summary").computed
    }

    fn expression_transfer(
        &mut self,
        op: &Op<'_>,
        mut values: impl ExactSizeIterator<Item = Operand>,
    ) -> Operand {
        let mut receiver = None;
        let computed = match op {
            Op::Primitive => self.pure_value(Hazards::NONE),
            Op::This => self.computed(self.this_value, self.context.pure()),
            Op::Identifier { symbol, name, span } => {
                if symbol.is_none() && *name == "arguments" {
                    if let Some(value) = self.arguments_value {
                        let value = self.context.read_binding(value);
                        return Operand {
                            computed: self.computed(value, self.context.pure()),
                            receiver: None,
                            extra: None,
                        };
                    }
                }
                let named = if let Some(symbol) = symbol {
                    let mut named = self.values.get(symbol).cloned().unwrap_or(NamedValue {
                        value: self.context.unknown(),
                        binding: None,
                    });
                    if let Some(binding) = &named.binding {
                        named.value = self.resolve(binding, *span);
                    } else {
                        named.value = self.context.read_binding(named.value);
                    }
                    named
                } else {
                    let binding = Binding::Global(vec![(*name).to_owned()]);
                    let value = if *name == "undefined" {
                        self.context.undefined()
                    } else {
                        self.resolve(&binding, *span)
                    };
                    NamedValue {
                        value,
                        binding: Some(binding),
                    }
                };
                let effects = if symbol.is_none() {
                    self.global_read(name, *span)
                } else {
                    self.context.pure()
                };
                Computed { named, effects }
            }
            Op::Member {
                property,
                span,
                reference,
            } => {
                let object = values.next().expect("member object").computed;
                if *reference {
                    receiver = Some(object.named.value);
                }
                let key = values.next().map(|value| value.computed);
                self.member(object, *property, key, *span)
            }
            Op::Call { pure, spread } => {
                let callee = values.next().expect("callee");
                let mut effects = vec![callee.computed.effects];
                let mut arguments = Vec::new();
                for value in values {
                    effects.push(value.computed.effects);
                    if let Some(extra) = value.extra {
                        effects.push(extra);
                    } else {
                        arguments.push(value.computed.named.value);
                    }
                }
                let result = if *spread {
                    CallResult::new(
                        self.context.unknown(),
                        if *pure {
                            self.context.pure()
                        } else {
                            self.context.opaque_effects()
                        },
                    )
                } else {
                    self.context.call_with_annotation(
                        callee.computed.named.value,
                        &arguments,
                        callee.receiver,
                        *pure,
                    )
                };
                effects.push(result.effects());
                let effects = self.context.any(effects);
                self.computed(result.value(), effects)
            }
            Op::New { pure, .. } => {
                let callee = values.next().expect("constructor").computed;
                let mut effects = Vec::new();
                for value in values {
                    effects.push(value.computed.effects);
                    effects.extend(value.extra);
                }
                effects.push(callee.effects);
                if !pure {
                    effects.push(self.context.opaque_effects());
                }
                let value = self.context.value(Hazards::UNKNOWN);
                let effects = self.context.any(effects);
                self.computed(value, effects)
            }
            Op::Binary(operator) => {
                let left = values.next().expect("left").computed;
                let right = values.next().expect("right").computed;
                let mut effects = vec![left.effects, right.effects];
                match *operator {
                    "===" | "!==" => {}
                    "==" | "!=" => {
                        effects.push(
                            self.context
                                .loose_equality(left.named.value, right.named.value),
                        );
                    }
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
                                .operation(left.named.value, Hazard::PrototypeRead),
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
            Op::Unary(operator) => {
                let argument = values.next().expect("argument").computed;
                let effects = if matches!(*operator, "!" | "void" | "typeof") {
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
            Op::Logical => {
                let left = values.next().expect("left").computed;
                let right = values.next().expect("right").computed;
                let value = self
                    .context
                    .join_values(&[left.named.value, right.named.value]);
                let effects = self.context.any([left.effects, right.effects]);
                self.computed(value, effects)
            }
            Op::Conditional => {
                let test = values.next().expect("test").computed;
                let left = values.next().expect("left").computed;
                let right = values.next().expect("right").computed;
                let value = self
                    .context
                    .join_values(&[left.named.value, right.named.value]);
                let effects = self
                    .context
                    .any([test.effects, left.effects, right.effects]);
                self.computed(value, effects)
            }
            Op::Sequence => {
                let mut effects = Vec::new();
                let mut last = self.computed(self.context.undefined(), self.context.pure());
                for value in values {
                    last = value.computed;
                    effects.push(last.effects);
                }
                last.effects = self.context.any(effects);
                last
            }
            Op::Template => {
                let mut effects = Vec::new();
                for value in values {
                    effects.push(value.computed.effects);
                    effects.extend(value.extra);
                }
                let effects = self.context.any(effects);
                let value = self.context.value(Hazards::NONE);
                self.computed(value, effects)
            }
            Op::Object {
                empty,
                getter,
                prototype,
                ..
            } => {
                let mut effects = Vec::new();
                for value in values {
                    effects.push(value.computed.effects);
                    effects.extend(value.extra);
                }
                let mut hazards = Hazards::UNKNOWN;
                hazards.0[Hazard::PropertyRead as usize] = *getter || *prototype;
                hazards.0[Hazard::PrototypeRead as usize] = *prototype;
                hazards.0[Hazard::ToPrimitive as usize] = !empty;
                let value = self.context.value(hazards);
                let effects = self.context.any(effects);
                self.computed(value, effects)
            }
            Op::Function {
                symbol,
                span,
                arrow,
            } => {
                let contract = if !arrow {
                    symbol
                        .and_then(|symbol| self.values[&symbol].binding.as_ref())
                        .and_then(|binding| self.context.definition(binding))
                } else {
                    None
                };
                let value = contract
                    .unwrap_or_else(|| self.context.function_value(self.functions[span].index));
                self.computed(value, self.context.pure())
            }
            _ => self.unknown(),
        };
        Operand {
            computed,
            receiver,
            extra: None,
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
                self.push_root(SourceRoot {
                    span: declaration.span(),
                    value: self.context.undefined(),
                    effects: self.context.pure(),
                });
            }
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_) => {}
            _ => {
                let value = self.unknown();
                self.push_root(SourceRoot {
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
                self.push_root(SourceRoot {
                    span: statement.span,
                    value: value.named.value,
                    effects: value.effects,
                });
            }
            Statement::VariableDeclaration(declaration) => {
                self.visit_variable_declaration(declaration)
            }
            Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {
                self.push_root(SourceRoot {
                    span: statement.span(),
                    value: self.context.undefined(),
                    effects: self.context.pure(),
                });
            }
            Statement::ImportDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportNamedDeclaration(_) => {}
            Statement::ExportDeclaration(declaration) => {
                self.visit_declaration(&declaration.declaration);
            }
            Statement::ExportDefaultDeclaration(declaration) => {
                if let Some(expression) = declaration.declaration.as_expression() {
                    let value = self.expression(expression);
                    self.push_root(SourceRoot {
                        span: declaration.span,
                        value: value.named.value,
                        effects: value.effects,
                    });
                } else if let ExportDefaultDeclarationKind::FunctionDeclaration(function) =
                    &declaration.declaration
                {
                    let value = if let Some(symbol) =
                        function.id.as_ref().and_then(|id| id.symbol_id.get())
                    {
                        self.context.read_binding(self.slots[&symbol])
                    } else {
                        self.function(function)
                    };
                    self.push_root(SourceRoot {
                        span: declaration.span,
                        value,
                        effects: self.context.pure(),
                    });
                } else {
                    let value = self.unknown();
                    self.push_root(SourceRoot {
                        span: declaration.span,
                        value: value.named.value,
                        effects: value.effects,
                    });
                }
            }
            Statement::TSTypeAliasDeclaration(_) | Statement::TSInterfaceDeclaration(_) => {}
            _ => {
                let value = self.unknown();
                self.push_root(SourceRoot {
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
            self.push_root(SourceRoot {
                span: declaration.span,
                value: value.named.value,
                effects: value.effects,
            });
        } else {
            for declarator in &declaration.declarations {
                if declaration.kind == VariableDeclarationKind::Var && declarator.init.is_none() {
                    if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                        let symbol = identifier.symbol_id.get().expect("binding symbol");
                        let value = self.context.read_binding(self.values[&symbol].value);
                        self.push_root(SourceRoot {
                            span: declarator.span,
                            value,
                            effects: self.context.pure(),
                        });
                        continue;
                    }
                }
                self.visit_variable_declarator(declarator);
            }
        }
    }

    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        let value = if let Some(initializer) = &declaration.init {
            let contract = match (&declaration.id, initializer.get_inner_expression()) {
                (
                    BindingPattern::BindingIdentifier(identifier),
                    Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_),
                ) if identifier.symbol_id.get().is_some_and(|symbol| {
                    self.semantic.scoping().symbol_scope_id(symbol)
                        == self.semantic.scoping().root_scope_id()
                }) =>
                {
                    let binding = Binding::Local(identifier.name.to_string());
                    self.contracts
                        .contains(&binding)
                        .then(|| self.context.definition(&binding))
                        .flatten()
                        .map(|value| Computed {
                            named: NamedValue {
                                value,
                                binding: Some(binding),
                            },
                            effects: self.context.pure(),
                        })
                }
                _ => None,
            };
            contract.unwrap_or_else(|| self.expression(initializer))
        } else {
            self.computed(self.context.undefined(), self.context.pure())
        };
        let effects = if let BindingPattern::BindingIdentifier(identifier) = &declaration.id {
            if let Some(symbol) = identifier.symbol_id.get() {
                let slot = self.slots[&symbol];
                self.context.set_binding(slot, value.named.value);
                let mut named = value.named.clone();
                named.value = slot;
                self.values.insert(symbol, named);
            }

            value.effects
        } else {
            self.context.opaque_effects()
        };
        self.push_root(SourceRoot {
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
    expressions: Option<Vec<&'a Expression<'a>>>,
}

impl<'a> Compiler<'_, '_, 'a> {
    fn compile(
        &mut self,
        span: Span,
        parameters: &FormalParameters<'a>,
        body: Option<&FunctionBody<'a>>,
        unsupported: bool,
        expression: Option<&Expression<'a>>,
    ) {
        let extractor = &mut self.extractor;
        let info = extractor.functions[&span].clone();
        let values: Vec<_> = info
            .locals
            .iter()
            .map(|symbol| (*symbol, extractor.values[symbol].clone()))
            .collect();
        let roots = std::mem::take(&mut extractor.roots);
        let receiver = extractor.this_value;
        let arguments_value = extractor.arguments_value;
        extractor.this_value = info.receiver;
        extractor.arguments_value = info.arguments;
        extractor.deferred = true;
        extractor.context.start_function();
        let mut arguments = Vec::new();
        let mut unsupported =
            unsupported || parameters.rest.is_some() || (body.is_none() && expression.is_none());
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
        } else if let Some(expression) = expression {
            let value = extractor.expression(expression);
            returns.push(value.named.value);
            extractor.roots.push(SourceRoot {
                span,
                value: value.named.value,
                effects: value.effects,
            });
        } else if let Some(body) = body {
            if !extractor.function_statements(
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
            FunctionSignature::new(
                captures,
                arguments,
                (!info.arrow).then_some(info.receiver),
                info.self_binding,
                !info.arrow,
                if info.arrow { None } else { info.arguments },
            ),
            returns,
            effects,
        );
        for (symbol, named) in values {
            extractor.values.insert(symbol, named);
        }
        extractor.roots = roots;
        extractor.this_value = receiver;
        extractor.arguments_value = arguments_value;
        extractor.deferred = false;
    }
}

impl<'a> FlatVisit<'a> for Compiler<'_, '_, 'a> {
    fn expression_queue(&mut self) -> &mut Option<Vec<&'a Expression<'a>>> {
        &mut self.expressions
    }
}

impl<'a> Visit<'a> for Compiler<'_, '_, 'a> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        self.flat_expression(expression);
    }
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let expressions = self.expressions.take();
        self.compile(
            function.span,
            &function.params,
            function.body.as_deref(),
            function.r#async || function.generator,
            None,
        );
        walk::walk_function(self, function, flags);
        self.expressions = expressions;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let expressions = self.expressions.take();
        self.compile(
            function.span,
            &function.params,
            function.body.as_function_body(),
            function.r#async,
            function.get_expression(),
        );
        walk::walk_arrow_function_expression(self, function);
        self.expressions = expressions;
    }
}
