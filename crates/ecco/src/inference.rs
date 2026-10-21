use std::collections::{HashMap, HashSet, VecDeque};

use eyre::{ensure, Result};
use oxc_ast::ast::*;
use oxc_ast_visit::Visit;
use oxc_semantic::{ScopeFlags, Semantic, SymbolId};

use crate::*;

// Bound substitutions and copied expressions, not just source visits.
const CONTRACT_BUDGET: usize = 4096;

struct Function<'s, 'a> {
    symbol: SymbolId,
    name: String,
    params: &'s FormalParameters<'a>,
    body: Option<&'s FunctionBody<'a>>,
    arrow: bool,
    expression: bool,
    unsupported: bool,
}

/// Infer owned contracts once, without evaluating callers or retaining AST data.
/// Supplied local contracts override inference; cyclic dependencies widen.
pub fn infer_function_contracts<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    facts: &LocalFacts,
) -> Result<LocalFacts> {
    Ok(infer_contracts(program, semantic, facts)?.0)
}

pub(crate) fn infer_contracts<'a>(
    program: &Program<'a>,
    semantic: &Semantic<'a>,
    facts: &LocalFacts,
) -> Result<(LocalFacts, HashSet<Binding>)> {
    crate::parser::validate_facts(facts)?;
    let mut functions = Vec::new();
    let mut imports = HashMap::new();
    for statement in &program.body {
        match statement {
            Statement::FunctionDeclaration(node) => collect_function(node, &mut functions),
            Statement::VariableDeclaration(node) => collect_variables(node, &mut functions),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::FunctionDeclaration(node)) => {
                    collect_function(node, &mut functions)
                }
                Some(Declaration::VariableDeclaration(node)) => {
                    collect_variables(node, &mut functions)
                }
                _ => {}
            },
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(node) = &export.declaration
                {
                    collect_function(node, &mut functions);
                }
            }
            Statement::ImportDeclaration(import) if !import.import_kind.is_type() => {
                for specifier in import.specifiers.iter().flatten() {
                    let path = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(node)
                            if node.import_kind.is_type() =>
                        {
                            continue
                        }
                        ImportDeclarationSpecifier::ImportSpecifier(node) => {
                            vec![node.imported.name().to_string()]
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                            vec!["default".into()]
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => Vec::new(),
                    };
                    if let Some(symbol) = specifier.local().symbol_id.get() {
                        imports.insert(
                            symbol,
                            Binding::Module {
                                module: import.source.value.to_string(),
                                path,
                            },
                        );
                    }
                }
            }
            _ => {}
        }
    }
    let mut seen = HashSet::new();
    let mut redeclared = HashSet::new();
    for function in &functions {
        if !seen.insert(function.symbol) {
            redeclared.insert(function.symbol);
        }
    }
    for function in &mut functions {
        function.unsupported |= redeclared.contains(&function.symbol);
    }
    let supplied = facts.definitions().len();
    let mut contracts = facts.definitions().to_vec();
    let mut bindings: HashMap<_, _> = contracts
        .iter()
        .enumerate()
        .map(|(index, definition)| (definition.binding().clone(), index))
        .collect();
    let mut symbols = HashMap::new();
    functions.retain(|function| {
        let binding = Binding::Local(function.name.clone());
        if let Some(index) = bindings.get(&binding) {
            symbols.insert(function.symbol, *index);
            false
        } else {
            let index = contracts.len();
            symbols.insert(function.symbol, index);
            bindings.insert(binding.clone(), index);
            contracts.push(unknown_contract(binding, Vec::new()));
            true
        }
    });
    let mut remaining = vec![0; functions.len()];
    let mut closed = vec![false; functions.len()];
    let mut dependents = vec![Vec::new(); functions.len()];
    for (index, function) in functions.iter().enumerate() {
        let mut dependencies = Dependencies {
            semantic,
            symbols: &symbols,
            indices: HashSet::new(),
            missing: false,
            external: false,
        };
        if let Some(body) = function.body {
            dependencies.visit_function_body(body);
        }
        ensure!(
            !dependencies.missing,
            "function inference requires resolved OXC references"
        );
        closed[index] = !dependencies.external;
        for dependency in dependencies
            .indices
            .into_iter()
            .filter(|dependency| *dependency >= supplied)
        {
            remaining[index] += 1;
            dependents[dependency - supplied].push(index);
        }
    }
    let prefixes: HashSet<_> = bindings
        .keys()
        .flat_map(|binding| match binding {
            Binding::Global(path) => (1..path.len())
                .map(|length| Binding::Global(path[..length].to_vec()))
                .collect::<Vec<_>>(),
            Binding::Module { module, path } => (0..path.len())
                .map(|length| Binding::Module {
                    module: module.clone(),
                    path: path[..length].to_vec(),
                })
                .collect(),
            Binding::Local(_) => Vec::new(),
        })
        .collect();
    let mut ready: VecDeque<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect();
    while let Some(index) = ready.pop_front() {
        let function = &functions[index];
        let mut analyzer = Analyzer {
            semantic,
            contracts: &contracts,
            symbols: &symbols,
            bindings: &bindings,
            imports: &imports,
            prefixes: &prefixes,
            global_symbols: &facts.global_symbols(),
            locals: HashMap::new(),
            effects: Vec::new(),
            returns: Vec::new(),
            updates: Vec::new(),
            remaining: CONTRACT_BUDGET,
            opaque: function.unsupported,
            arrow: function.arrow,
            expanding: HashSet::new(),
        };
        let mut parameters = Vec::new();
        for parameter in &function.params.items {
            if let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern {
                let name = identifier.name.to_string();
                if name == "this" || parameters.contains(&name) {
                    analyzer.opaque = true;
                    continue;
                }
                parameters.push(name.clone());
                analyzer.locals.insert(
                    identifier.symbol_id.get().expect("parameter symbol"),
                    Value::input(name),
                );
            } else {
                analyzer.opaque = true;
            }
            analyzer.opaque |= parameter.initializer.is_some();
        }
        analyzer.opaque |= function.params.rest.is_some();
        if !analyzer.opaque {
            if let Some(body) = function.body {
                if function.expression {
                    if let Some(Statement::ExpressionStatement(statement)) = body.statements.first()
                    {
                        let value = analyzer.expression(&statement.expression);
                        analyzer.effects.push(value.effects);
                        analyzer.returns.push(value.value);
                    }
                } else if !analyzer.statements(&body.statements) {
                    analyzer.returns.push(Value::void());
                }
            }
        }
        let definition = analyzer.finish(Binding::Local(function.name.clone()), parameters);
        contracts[supplied + index] = definition;
        for dependent in &dependents[index] {
            remaining[*dependent] -= 1;
            if remaining[*dependent] == 0 {
                ready.push_back(*dependent);
            }
        }
    }
    let inferred = LocalFacts::new(
        contracts.into_iter().skip(supplied).collect(),
        Default::default(),
    );
    crate::parser::validate_facts(&inferred)?;
    // Linked captures and alias joins need the richer owned operation summaries.
    let usable = inferred
        .definitions()
        .iter()
        .zip(closed)
        .filter_map(|(definition, closed)| {
            fn known(expression: &HazardExpr) -> bool {
                match expression {
                    HazardExpr::Unknown => false,
                    HazardExpr::Or(values) => values.iter().all(known),
                    _ => true,
                }
            }
            (closed
                && !matches!(definition.returns(), Returns::Unknown)
                && [definition.hazards(), definition.return_hazards()]
                    .into_iter()
                    .all(|fields| {
                        fields
                            .default_expression()
                            .into_iter()
                            .chain(fields.fields().iter().map(|(_, value)| value))
                            .all(known)
                    })
                && definition
                    .updates()
                    .iter()
                    .all(|update| known(update.expression())))
            .then(|| definition.binding().clone())
        })
        .collect();
    Ok((inferred, usable))
}

fn collect_function<'s, 'a>(
    node: &'s oxc_ast::ast::Function<'a>,
    functions: &mut Vec<Function<'s, 'a>>,
) {
    if let Some(identifier) = &node.id {
        if let Some(symbol) = identifier.symbol_id.get() {
            functions.push(Function {
                symbol,
                name: identifier.name.to_string(),
                params: &node.params,
                body: node.body.as_deref(),
                arrow: false,
                expression: false,
                unsupported: node.r#async || node.generator || node.body.is_none(),
            });
        }
    }
}

fn collect_variables<'s, 'a>(
    node: &'s VariableDeclaration<'a>,
    functions: &mut Vec<Function<'s, 'a>>,
) {
    if node.kind != VariableDeclarationKind::Const {
        return;
    }
    for declaration in &node.declarations {
        let BindingPattern::BindingIdentifier(identifier) = &declaration.id else {
            continue;
        };
        let Some(symbol) = identifier.symbol_id.get() else {
            continue;
        };
        let Some(initializer) = &declaration.init else {
            continue;
        };
        let candidate = match initializer.get_inner_expression() {
            Expression::FunctionExpression(node) => Function {
                symbol,
                name: identifier.name.to_string(),
                params: &node.params,
                body: node.body.as_deref(),
                arrow: false,
                expression: false,
                unsupported: node.r#async || node.generator || node.body.is_none(),
            },
            Expression::ArrowFunctionExpression(node) => Function {
                symbol,
                name: identifier.name.to_string(),
                params: &node.params,
                body: Some(&node.body),
                arrow: true,
                expression: node.expression,
                unsupported: node.r#async,
            },
            _ => continue,
        };
        functions.push(candidate);
    }
}

struct Dependencies<'s, 'a> {
    semantic: &'s Semantic<'a>,
    symbols: &'s HashMap<SymbolId, usize>,
    indices: HashSet<usize>,
    missing: bool,
    external: bool,
}

impl<'a> Visit<'a> for Dependencies<'_, 'a> {
    fn visit_binary_expression(&mut self, expression: &BinaryExpression<'a>) {
        // Equal argument identities are known only after graph linking.
        self.external |= matches!(expression.operator.as_str(), "==" | "!=");
        oxc_ast_visit::walk::walk_binary_expression(self, expression);
    }
    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if let Some(reference) = identifier.reference_id.get() {
            let symbol = self.semantic.scoping().get_reference(reference).symbol_id();
            self.external |= symbol.map_or(identifier.name != "undefined", |symbol| {
                self.semantic.scoping().symbol_scope_id(symbol)
                    == self.semantic.scoping().root_scope_id()
            });
            if let Some(index) = self
                .semantic
                .scoping()
                .get_reference(reference)
                .symbol_id()
                .and_then(|symbol| self.symbols.get(&symbol))
            {
                self.indices.insert(*index);
            }
        } else {
            self.missing = true;
        }
    }
    fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: ScopeFlags) {}
    fn visit_arrow_function_expression(&mut self, _: &ArrowFunctionExpression<'a>) {}
}

#[derive(Clone, PartialEq, Eq)]
enum Origin {
    Input(String),
    Fresh,
    Void,
    Unknown,
}

#[derive(Clone)]
struct Value {
    origin: Origin,
    hazards: [HazardExpr; Hazard::ALL.len()],
    callable: Option<usize>,
    binding: Option<Binding>,
}

impl Value {
    fn size(&self) -> usize {
        self.hazards.iter().map(expression_size).sum()
    }

    fn unknown() -> Self {
        Self {
            origin: Origin::Unknown,
            hazards: std::array::from_fn(|_| HazardExpr::Unknown),
            callable: None,
            binding: None,
        }
    }
    fn primitive() -> Self {
        Self {
            origin: Origin::Fresh,
            hazards: std::array::from_fn(|_| HazardExpr::Bool(false)),
            callable: None,
            binding: None,
        }
    }
    fn void() -> Self {
        Self {
            origin: Origin::Void,
            ..Self::primitive()
        }
    }
    fn input(name: String) -> Self {
        Self {
            origin: Origin::Input(name.clone()),
            hazards: std::array::from_fn(|index| HazardExpr::Read {
                value: name.clone(),
                slot: Hazard::ALL[index],
            }),
            callable: None,
            binding: None,
        }
    }
    fn hazard(&self, slot: Hazard) -> HazardExpr {
        self.hazards[slot as usize].clone()
    }
}

fn expression_size(expression: &HazardExpr) -> usize {
    match expression {
        HazardExpr::Or(expressions) => 1 + expressions.iter().map(expression_size).sum::<usize>(),
        HazardExpr::Call { arguments, .. } => 1 + arguments.len(),
        _ => 1,
    }
}

struct Computed {
    value: Value,
    effects: HazardExpr,
}

impl Computed {
    fn pure(value: Value) -> Self {
        Self {
            value,
            effects: HazardExpr::Bool(false),
        }
    }
}

fn union(expressions: impl IntoIterator<Item = HazardExpr>) -> HazardExpr {
    let mut pending: Vec<_> = expressions.into_iter().collect();
    pending.reverse();
    let mut unique = HashSet::new();
    let mut output = Vec::new();
    while let Some(expression) = pending.pop() {
        match expression {
            HazardExpr::Bool(false) => {}
            HazardExpr::Unknown => return HazardExpr::Unknown,
            HazardExpr::Or(mut inputs) => {
                inputs.reverse();
                pending.extend(inputs);
            }
            expression if unique.insert(expression.clone()) => output.push(expression),
            _ => {}
        }
    }
    match output.len() {
        0 => HazardExpr::Bool(false),
        1 => output.pop().expect("one expression"),
        _ => HazardExpr::Or(output),
    }
}

fn unknown_contract(binding: Binding, parameters: Vec<String>) -> Definition {
    Definition::new(
        binding,
        parameters,
        None,
        true,
        HazardFields::new(None, vec![(Hazard::Call, HazardExpr::Unknown)]),
        Returns::Unknown,
        HazardFields::default(),
        Vec::new(),
    )
}

struct Analyzer<'s, 'a> {
    semantic: &'s Semantic<'a>,
    contracts: &'s [Definition],
    symbols: &'s HashMap<SymbolId, usize>,
    bindings: &'s HashMap<Binding, usize>,
    imports: &'s HashMap<SymbolId, Binding>,
    prefixes: &'s HashSet<Binding>,
    global_symbols: &'s [GlobalSymbol],
    locals: HashMap<SymbolId, Value>,
    effects: Vec<HazardExpr>,
    returns: Vec<Value>,
    updates: Vec<Update>,
    remaining: usize,
    opaque: bool,
    arrow: bool,
    expanding: HashSet<usize>,
}

impl<'a> Analyzer<'_, 'a> {
    fn spend(&mut self) -> bool {
        self.spend_size(1)
    }

    fn spend_size(&mut self, size: usize) -> bool {
        if size > self.remaining {
            self.remaining = 0;
            self.opaque = true;
            false
        } else {
            self.remaining -= size;
            true
        }
    }

    fn unknown(&mut self) -> Computed {
        self.opaque = true;
        Computed {
            value: Value::unknown(),
            effects: HazardExpr::Unknown,
        }
    }

    fn bound(&self, binding: Binding) -> Value {
        let mut value = Value::unknown();
        if let Some(index) = self.bindings.get(&binding) {
            value.callable = Some(*index);
            let definition = &self.contracts[*index];
            value.hazards =
                std::array::from_fn(|index| match definition.hazards().get(Hazard::ALL[index]) {
                    HazardExpr::Bool(false) => HazardExpr::Bool(false),
                    _ => HazardExpr::Unknown,
                });
        } else if self.prefixes.contains(&binding) {
            value.hazards[Hazard::PropertyRead as usize] = HazardExpr::Bool(false);
        }
        value.binding = Some(binding);
        value
    }

    fn expression(&mut self, expression: &Expression<'a>) -> Computed {
        if self.opaque || !self.spend() {
            return self.unknown();
        }
        match expression.get_inner_expression() {
            Expression::BooleanLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::NullLiteral(_) => Computed::pure(Value::primitive()),
            Expression::ThisExpression(_) => Computed::pure(if self.arrow {
                Value::unknown()
            } else {
                Value::input("this".into())
            }),
            Expression::Identifier(identifier) => {
                let reference = self
                    .semantic
                    .scoping()
                    .get_reference(identifier.reference_id.get().expect("checked references"));
                let value = if let Some(symbol) = reference.symbol_id() {
                    if let Some(size) = self.locals.get(&symbol).map(Value::size) {
                        if !self.spend_size(size) {
                            return self.unknown();
                        }
                        self.locals[&symbol].clone()
                    } else if let Some(index) = self.symbols.get(&symbol) {
                        self.bound(self.contracts[*index].binding().clone())
                    } else if let Some(binding) = self.imports.get(&symbol) {
                        self.bound(binding.clone())
                    } else {
                        Value::unknown()
                    }
                } else if identifier.name == "undefined" {
                    Value::void()
                } else {
                    self.bound(Binding::Global(vec![identifier.name.to_string()]))
                };
                Computed {
                    value,
                    effects: if reference.symbol_id().is_none()
                        && self.global_symbols.iter().any(|symbol| {
                            symbol.name() == identifier.name.as_str() && symbol.read_effects()
                        }) {
                        HazardExpr::Unknown
                    } else {
                        HazardExpr::Bool(false)
                    },
                }
            }
            Expression::StaticMemberExpression(member) => {
                let object = self.expression(&member.object);
                self.member(object, Some(member.property.name.as_str()), None)
            }
            Expression::ComputedMemberExpression(member) => {
                let object = self.expression(&member.object);
                let key = self.expression(&member.expression);
                let property = match member.expression.get_inner_expression() {
                    Expression::StringLiteral(node) => Some(node.value.as_str()),
                    _ => None,
                };
                self.member(object, property, Some(key))
            }
            Expression::CallExpression(call) => {
                let (callee, receiver) = match call.callee.get_inner_expression() {
                    Expression::StaticMemberExpression(member) => {
                        let object = self.expression(&member.object);
                        let receiver = object.value.clone();
                        (
                            self.member(object, Some(member.property.name.as_str()), None),
                            receiver,
                        )
                    }
                    Expression::ComputedMemberExpression(member) => {
                        let object = self.expression(&member.object);
                        let receiver = object.value.clone();
                        let key = self.expression(&member.expression);
                        let property = match member.expression.get_inner_expression() {
                            Expression::StringLiteral(node) => Some(node.value.as_str()),
                            _ => None,
                        };
                        (self.member(object, property, Some(key)), receiver)
                    }
                    _ => (self.expression(&call.callee), Value::void()),
                };
                let mut effects = vec![callee.effects];
                let mut arguments = Vec::new();
                for argument in &call.arguments {
                    let Some(expression) = argument.as_expression() else {
                        return self.unknown();
                    };
                    let argument = self.expression(expression);
                    if self.opaque {
                        return self.unknown();
                    }
                    effects.push(argument.effects);
                    arguments.push(argument.value);
                }
                let opaque = self.opaque;
                let result = self.invoke(callee.value, arguments, receiver, !call.pure);
                if call.pure {
                    self.opaque = opaque;
                } else {
                    effects.push(result.effects);
                }
                Computed {
                    value: result.value,
                    effects: union(effects),
                }
            }
            Expression::BinaryExpression(binary) => {
                let left = self.expression(&binary.left);
                let right = self.expression(&binary.right);
                let mut effects = vec![left.effects, right.effects];
                match binary.operator.as_str() {
                    "===" | "!==" => {}
                    "in" => effects.extend([
                        left.value.hazard(Hazard::ToPrimitive),
                        right.value.hazard(Hazard::In),
                    ]),
                    "instanceof" => effects.extend([
                        left.value.hazard(Hazard::PrototypeRead),
                        right.value.hazard(Hazard::Instanceof),
                    ]),
                    "==" | "!="
                        if matches!((&left.value.origin, &right.value.origin),
                        (Origin::Input(left), Origin::Input(right)) if left == right) => {}
                    _ => effects.extend([
                        left.value.hazard(Hazard::ToPrimitive),
                        right.value.hazard(Hazard::ToPrimitive),
                    ]),
                }
                Computed {
                    value: Value::primitive(),
                    effects: union(effects),
                }
            }
            Expression::UnaryExpression(unary) if unary.operator.as_str() != "delete" => {
                let argument = self.expression(&unary.argument);
                let effects = if matches!(unary.operator.as_str(), "!" | "void" | "typeof") {
                    argument.effects
                } else {
                    union([argument.effects, argument.value.hazard(Hazard::ToPrimitive)])
                };
                Computed {
                    value: if unary.operator.as_str() == "void" {
                        Value::void()
                    } else {
                        Value::primitive()
                    },
                    effects,
                }
            }
            Expression::LogicalExpression(logical) => {
                let left = self.expression(&logical.left);
                let right = self.expression(&logical.right);
                Computed {
                    value: Self::join(&[left.value, right.value]),
                    effects: union([left.effects, right.effects]),
                }
            }
            Expression::ConditionalExpression(conditional) => {
                let test = self.expression(&conditional.test);
                let left = self.expression(&conditional.consequent);
                let right = self.expression(&conditional.alternate);
                Computed {
                    value: Self::join(&[left.value, right.value]),
                    effects: union([test.effects, left.effects, right.effects]),
                }
            }
            Expression::SequenceExpression(sequence) => {
                let mut effects = Vec::new();
                let mut last = Value::void();
                for expression in &sequence.expressions {
                    let value = self.expression(expression);
                    effects.push(value.effects);
                    last = value.value;
                }
                Computed {
                    value: last,
                    effects: union(effects),
                }
            }
            Expression::TemplateLiteral(template) => {
                let mut effects = Vec::new();
                for expression in &template.expressions {
                    let value = self.expression(expression);
                    effects.extend([value.effects, value.value.hazard(Hazard::ToPrimitive)]);
                }
                Computed {
                    value: Value::primitive(),
                    effects: union(effects),
                }
            }
            Expression::ObjectExpression(object) => {
                let mut effects = Vec::new();
                let mut value = Value::primitive();
                value.hazards = std::array::from_fn(|_| HazardExpr::Unknown);
                value.hazards[Hazard::PropertyRead as usize] = HazardExpr::Bool(false);
                value.hazards[Hazard::ToPrimitive as usize] = if object.properties.is_empty() {
                    HazardExpr::Bool(false)
                } else {
                    HazardExpr::Unknown
                };
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        return self.unknown();
                    };
                    if property.computed {
                        if let Some(key) = property.key.as_expression() {
                            let key = self.expression(key);
                            effects.extend([key.effects, key.value.hazard(Hazard::ToPrimitive)]);
                        }
                    }
                    if property.kind == PropertyKind::Get
                        || property
                            .key
                            .static_name()
                            .is_some_and(|name| name == "__proto__")
                    {
                        value.hazards[Hazard::PropertyRead as usize] = HazardExpr::Unknown;
                    }
                    effects.push(self.expression(&property.value).effects);
                }
                Computed {
                    value,
                    effects: union(effects),
                }
            }
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => {
                Computed::pure(Value::unknown())
            }
            _ => self.unknown(),
        }
    }

    fn member(&self, object: Computed, property: Option<&str>, key: Option<Computed>) -> Computed {
        let binding = match (object.value.binding.clone(), property) {
            (Some(Binding::Global(mut path)), Some(property)) => {
                path.push(property.into());
                Some(Binding::Global(path))
            }
            (Some(Binding::Module { module, mut path }), Some(property)) => {
                path.push(property.into());
                Some(Binding::Module { module, path })
            }
            _ => None,
        };
        let mut effects = vec![object.effects, object.value.hazard(Hazard::PropertyRead)];
        if let Some(key) = key {
            effects.extend([key.effects, key.value.hazard(Hazard::ToPrimitive)]);
        }
        Computed {
            value: binding.map_or_else(Value::unknown, |binding| self.bound(binding)),
            effects: union(effects),
        }
    }

    fn statements(&mut self, statements: &[Statement<'a>]) -> bool {
        for statement in statements {
            if self.opaque {
                return false;
            }
            if self.statement(statement) {
                return true;
            }
        }
        false
    }

    fn statement(&mut self, statement: &Statement<'a>) -> bool {
        match statement {
            Statement::ReturnStatement(statement) => {
                let value = statement.argument.as_ref().map_or_else(
                    || Computed::pure(Value::void()),
                    |expression| self.expression(expression),
                );
                self.effects.push(value.effects);
                self.returns.push(value.value);
                true
            }
            Statement::ThrowStatement(statement) => {
                let value = self.expression(&statement.argument);
                self.effects.push(value.effects);
                true
            }
            Statement::ExpressionStatement(statement) => {
                let value = self.expression(&statement.expression);
                self.effects.push(value.effects);
                false
            }
            Statement::VariableDeclaration(declaration)
                if declaration.kind == VariableDeclarationKind::Const =>
            {
                for declaration in &declaration.declarations {
                    let BindingPattern::BindingIdentifier(identifier) = &declaration.id else {
                        self.opaque = true;
                        return false;
                    };
                    let value = declaration.init.as_ref().map_or_else(
                        || Computed::pure(Value::void()),
                        |expression| self.expression(expression),
                    );
                    self.effects.push(value.effects);
                    self.locals.insert(
                        identifier.symbol_id.get().expect("binding symbol"),
                        value.value,
                    );
                }
                false
            }
            Statement::BlockStatement(block) => self.statements(&block.body),
            Statement::IfStatement(statement) => {
                let test = self.expression(&statement.test);
                self.effects.push(test.effects);
                let consequent = self.statement(&statement.consequent);
                let alternate = statement
                    .alternate
                    .as_ref()
                    .is_some_and(|statement| self.statement(statement));
                consequent && alternate
            }
            Statement::EmptyStatement(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_) => false,
            _ => {
                self.opaque = true;
                false
            }
        }
    }

    fn join(values: &[Value]) -> Value {
        let first = values.first().cloned().unwrap_or_else(Value::void);
        let origin = if values.iter().all(|value| value.origin == first.origin) {
            first.origin
        } else if values
            .iter()
            .all(|value| matches!(value.origin, Origin::Fresh | Origin::Void))
        {
            Origin::Fresh
        } else {
            Origin::Unknown
        };
        Value {
            origin,
            hazards: std::array::from_fn(|index| {
                union(values.iter().map(|value| value.hazards[index].clone()))
            }),
            callable: None,
            binding: None,
        }
    }

    fn invoke(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        receiver: Value,
        execute: bool,
    ) -> Computed {
        if !self.spend() {
            return self.unknown();
        }
        if let Some(index) = function.callable {
            if !self.expanding.insert(index) {
                return self.unknown();
            }
            let definition = self.contracts[index].clone();
            if !definition.variadic() && arguments.len() > definition.parameters().len() {
                self.expanding.remove(&index);
                return self.unknown();
            }
            let mut frame: HashMap<_, _> = definition
                .parameters()
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        name.clone(),
                        arguments.get(index).cloned().unwrap_or_else(Value::void),
                    )
                })
                .collect();
            frame.insert("this".into(), receiver);
            let effects = if !execute && !definition.updates().is_empty() {
                HazardExpr::Bool(true)
            } else {
                self.substitute(
                    &definition.hazards().get(Hazard::Call),
                    &frame,
                    &arguments,
                    &definition,
                )
            };
            if execute {
                self.execute_calls(
                    &definition.hazards().get(Hazard::Call),
                    &frame,
                    &arguments,
                    &definition,
                );
                for update in definition.updates() {
                    let expression =
                        self.substitute(update.expression(), &frame, &arguments, &definition);
                    if let Some(Value {
                        origin: Origin::Input(target),
                        ..
                    }) = frame.get(update.target())
                    {
                        let update = Update::new(target.clone(), update.slot(), expression);
                        if !self.updates.contains(&update) {
                            self.updates.push(update);
                        }
                    } else {
                        self.opaque = true;
                    }
                }
            }
            let value = match definition.returns() {
                Returns::Argument(name) => frame.get(name).cloned().unwrap_or_else(Value::unknown),
                Returns::Void => Value::void(),
                Returns::Unknown => Value::unknown(),
                Returns::Fresh => Value {
                    origin: Origin::Fresh,
                    hazards: std::array::from_fn(|index| {
                        self.substitute(
                            &definition.return_hazards().get(Hazard::ALL[index]),
                            &frame,
                            &arguments,
                            &definition,
                        )
                    }),
                    callable: None,
                    binding: None,
                },
            };
            self.expanding.remove(&index);
            Computed { value, effects }
        } else if let Origin::Input(name) = function.origin {
            let arguments: Option<Vec<_>> = arguments
                .into_iter()
                .map(|value| {
                    if let Origin::Input(name) = value.origin {
                        Some(CallArgument::Value(name))
                    } else {
                        None
                    }
                })
                .collect();
            let receiver = match receiver.origin {
                Origin::Input(name) => Some(Some(name)),
                Origin::Void => Some(None),
                _ => None,
            };
            if let (Some(arguments), Some(receiver)) = (arguments, receiver) {
                Computed {
                    value: Value::unknown(),
                    effects: HazardExpr::Call {
                        function: name,
                        arguments,
                        receiver,
                    },
                }
            } else {
                self.unknown()
            }
        } else {
            self.unknown()
        }
    }

    fn call_inputs(
        &self,
        inputs: &[CallArgument],
        frame: &HashMap<String, Value>,
        arguments: &[Value],
        definition: &Definition,
    ) -> Vec<Value> {
        let mut values = Vec::new();
        for argument in inputs {
            match argument {
                CallArgument::Value(name) => {
                    values.push(frame.get(name).cloned().unwrap_or_else(Value::unknown))
                }
                CallArgument::Forward => values.extend_from_slice(arguments),
                CallArgument::Spread(_) => values.extend_from_slice(
                    &arguments[definition.parameters().len().min(arguments.len())..],
                ),
            }
        }
        values
    }

    fn substitute(
        &mut self,
        expression: &HazardExpr,
        frame: &HashMap<String, Value>,
        arguments: &[Value],
        definition: &Definition,
    ) -> HazardExpr {
        if !self.spend() {
            return HazardExpr::Unknown;
        }
        match expression {
            HazardExpr::Bool(value) => HazardExpr::Bool(*value),
            HazardExpr::Unknown => HazardExpr::Unknown,
            HazardExpr::Read { value, slot } => {
                if let Some(value) = frame.get(value) {
                    if self.spend_size(expression_size(&value.hazards[*slot as usize])) {
                        return value.hazard(*slot);
                    }
                }
                HazardExpr::Unknown
            }
            HazardExpr::Or(expressions) => {
                let mut values = Vec::new();
                for expression in expressions {
                    if self.opaque {
                        return HazardExpr::Unknown;
                    }
                    values.push(self.substitute(expression, frame, arguments, definition));
                }
                union(values)
            }
            HazardExpr::Call {
                function,
                arguments: inputs,
                receiver,
            } => {
                let function = frame.get(function).cloned().unwrap_or_else(Value::unknown);
                let values = self.call_inputs(inputs, frame, arguments, definition);
                let receiver = receiver.as_ref().map_or_else(Value::void, |name| {
                    frame.get(name).cloned().unwrap_or_else(Value::unknown)
                });
                self.invoke(function, values, receiver, false).effects
            }
        }
    }

    fn execute_calls(
        &mut self,
        expression: &HazardExpr,
        frame: &HashMap<String, Value>,
        arguments: &[Value],
        definition: &Definition,
    ) {
        match expression {
            HazardExpr::Or(expressions) => {
                for expression in expressions {
                    self.execute_calls(expression, frame, arguments, definition);
                }
            }
            HazardExpr::Call {
                function,
                arguments: inputs,
                receiver,
            } => {
                let function = frame.get(function).cloned().unwrap_or_else(Value::unknown);
                let values = self.call_inputs(inputs, frame, arguments, definition);
                let receiver = receiver.as_ref().map_or_else(Value::void, |name| {
                    frame.get(name).cloned().unwrap_or_else(Value::unknown)
                });
                self.invoke(function, values, receiver, true);
            }
            _ => {}
        }
    }

    fn finish(self, binding: Binding, parameters: Vec<String>) -> Definition {
        if self.opaque {
            return unknown_contract(binding, parameters);
        }
        let value = Self::join(&self.returns);
        let returns = match value.origin {
            Origin::Input(name) => Returns::Argument(name),
            Origin::Void => Returns::Void,
            Origin::Fresh => Returns::Fresh,
            Origin::Unknown => Returns::Unknown,
        };
        let return_hazards = if matches!(returns, Returns::Fresh) {
            HazardFields::new(None, Hazard::ALL.into_iter().zip(value.hazards).collect())
        } else {
            HazardFields::default()
        };
        let effects = union(self.effects);
        Definition::new(
            binding,
            parameters,
            None,
            true,
            HazardFields::new(
                Some(HazardExpr::Bool(false)),
                vec![
                    (Hazard::Call, effects.clone()),
                    (
                        Hazard::Construct,
                        if self.arrow {
                            HazardExpr::Bool(false)
                        } else {
                            effects
                        },
                    ),
                ],
            ),
            returns,
            return_hazards,
            self.updates,
        )
    }
}
