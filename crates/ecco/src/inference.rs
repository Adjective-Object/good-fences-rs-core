use std::collections::{HashMap, HashSet, VecDeque};

use eyre::{ensure, Result};
use oxc_ast::ast::*;
use oxc_ast_visit::Visit;
use oxc_semantic::{ScopeFlags, Semantic, SymbolId};

use crate::frontier::{FlatVisit, Mode, Op, Plan, Role, Step};
use crate::*;

// Bound substitutions and copied expressions, not just source visits.
const CONTRACT_BUDGET: usize = 4096;

struct Function<'s, 'a> {
    symbol: SymbolId,
    name: String,
    params: &'s FormalParameters<'a>,
    body: Option<&'s FunctionBody<'a>>,
    arrow: bool,
    expression: Option<&'s Expression<'a>>,
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
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(node) => collect_function(node, &mut functions),
                Declaration::VariableDeclaration(node) => collect_variables(node, &mut functions),
                _ => {}
            },
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(node) = &export.declaration
                {
                    collect_function(node, &mut functions);
                }
            }
            Statement::ImportDeclaration(import) if !import.import_kind.is_type() => {
                let Some(module) = import.source.value.as_str() else {
                    continue;
                };
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
                                module: module.to_owned(),
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
            expressions: None,
        };
        if let Some(body) = function.body {
            dependencies.visit_function_body(body);
        }
        if let Some(expression) = function.expression {
            dependencies.visit_expression(expression);
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
            if let Some(expression) = function.expression {
                let value = analyzer.expression(expression);
                analyzer.effects.push(value.effects);
                analyzer.returns.push(value.value);
            } else if let Some(body) = function.body {
                if !analyzer.statements(&body.statements) {
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
                expression: None,
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
                expression: None,
                unsupported: node.r#async || node.generator || node.body.is_none(),
            },
            Expression::ArrowFunctionExpression(node) => Function {
                symbol,
                name: identifier.name.to_string(),
                params: &node.params,
                body: node.body.as_function_body(),
                arrow: true,
                expression: node.get_expression(),
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
    expressions: Option<Vec<&'a Expression<'a>>>,
}

impl<'a> FlatVisit<'a> for Dependencies<'_, 'a> {
    fn expression_queue(&mut self) -> &mut Option<Vec<&'a Expression<'a>>> {
        &mut self.expressions
    }
}

impl<'a> Visit<'a> for Dependencies<'_, 'a> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        self.flat_expression(expression);
    }
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

// Common leaves stay inline without materializing every symbolic hazard slot.
#[derive(Clone)]
enum ValueHazards {
    Pure,
    Unknown,
    Input,
    Fields(Box<[HazardExpr; Hazard::ALL.len()]>),
}

#[derive(Clone)]
struct Value {
    origin: Origin,
    hazards: ValueHazards,
    callable: Option<usize>,
    binding: Option<Binding>,
}

impl Value {
    fn size(&self) -> usize {
        match &self.hazards {
            ValueHazards::Fields(fields) => fields.iter().map(expression_size).sum(),
            _ => Hazard::ALL.len(),
        }
    }

    fn unknown() -> Self {
        Self {
            origin: Origin::Unknown,
            hazards: ValueHazards::Unknown,
            callable: None,
            binding: None,
        }
    }
    fn primitive() -> Self {
        Self {
            origin: Origin::Fresh,
            hazards: ValueHazards::Pure,
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
            origin: Origin::Input(name),
            hazards: ValueHazards::Input,
            callable: None,
            binding: None,
        }
    }
    fn hazard(&self, slot: Hazard) -> HazardExpr {
        match &self.hazards {
            ValueHazards::Pure => HazardExpr::Bool(false),
            ValueHazards::Unknown => HazardExpr::Unknown,
            ValueHazards::Input => {
                let Origin::Input(name) = &self.origin else {
                    unreachable!("input hazards have an input origin")
                };
                HazardExpr::Read {
                    value: name.clone(),
                    slot,
                }
            }
            ValueHazards::Fields(fields) => fields[slot as usize].clone(),
        }
    }

    fn hazard_size(&self, slot: Hazard) -> usize {
        match &self.hazards {
            ValueHazards::Fields(fields) => expression_size(&fields[slot as usize]),
            _ => 1,
        }
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

    fn binary(operator: &str, left: Self, right: Self) -> Self {
        let mut effects = vec![left.effects, right.effects];
        match operator {
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
        Self {
            value: Value::primitive(),
            effects: union(effects),
        }
    }

    fn unary(operator: &str, argument: Self) -> Self {
        let effects = if matches!(operator, "!" | "void" | "typeof") {
            argument.effects
        } else {
            union([argument.effects, argument.value.hazard(Hazard::ToPrimitive)])
        };
        Self {
            value: if operator == "void" {
                Value::void()
            } else {
                Value::primitive()
            },
            effects,
        }
    }

    fn alternatives(left: Self, right: Self) -> Self {
        Self {
            value: Transfer::join(&[left.value, right.value]),
            effects: union([left.effects, right.effects]),
        }
    }

    fn conditional(test: Self, left: Self, right: Self) -> Self {
        Self {
            value: Transfer::join(&[left.value, right.value]),
            effects: union([test.effects, left.effects, right.effects]),
        }
    }

    fn object(empty: bool, unknown_read: bool, effects: Vec<HazardExpr>) -> Self {
        let mut value = Value::primitive();
        let mut hazards = std::array::from_fn(|_| HazardExpr::Unknown);
        hazards[Hazard::PropertyRead as usize] = if unknown_read {
            HazardExpr::Unknown
        } else {
            HazardExpr::Bool(false)
        };
        hazards[Hazard::ToPrimitive as usize] = if empty {
            HazardExpr::Bool(false)
        } else {
            HazardExpr::Unknown
        };
        value.hazards = ValueHazards::Fields(Box::new(hazards));
        Self {
            value,
            effects: union(effects),
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
}

/// Borrowed transfer inputs contain no arena or mutable collection state.
#[derive(Clone, Copy)]
struct TransferInputs<'s> {
    contracts: &'s [Definition],
    symbols: &'s HashMap<SymbolId, usize>,
    bindings: &'s HashMap<Binding, usize>,
    imports: &'s HashMap<SymbolId, Binding>,
    prefixes: &'s HashSet<Binding>,
    global_symbols: &'s [GlobalSymbol],
    locals: &'s HashMap<SymbolId, Value>,
    arrow: bool,
}

struct TransferOutcome {
    computed: Computed,
    updates: Vec<Update>,
    budget_used: usize,
    opaque: bool,
}

/// One owned accumulator per expression root, not one copied log per AST edge.
struct Transfer<'s> {
    inputs: TransferInputs<'s>,
    updates: Vec<Update>,
    budget: usize,
    remaining: usize,
    opaque: bool,
    expanding: HashSet<usize>,
}

impl<'s> Transfer<'s> {
    fn new(inputs: TransferInputs<'s>, budget: usize, opaque: bool) -> Self {
        Self {
            inputs,
            updates: Vec::new(),
            budget,
            remaining: budget,
            opaque,
            expanding: HashSet::new(),
        }
    }

    fn finish(self, computed: Computed) -> TransferOutcome {
        TransferOutcome {
            computed,
            updates: self.updates,
            budget_used: self.budget - self.remaining,
            opaque: self.opaque,
        }
    }

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
        if let Some(index) = self.inputs.bindings.get(&binding) {
            value.callable = Some(*index);
            let definition = &self.inputs.contracts[*index];
            value.hazards = ValueHazards::Fields(Box::new(std::array::from_fn(|index| {
                match definition.hazards().get(Hazard::ALL[index]) {
                    HazardExpr::Bool(false) => HazardExpr::Bool(false),
                    _ => HazardExpr::Unknown,
                }
            })));
        } else if self.inputs.prefixes.contains(&binding) {
            value.hazards = ValueHazards::Fields(Box::new(std::array::from_fn(|index| {
                if Hazard::ALL[index] == Hazard::PropertyRead {
                    HazardExpr::Bool(false)
                } else {
                    HazardExpr::Unknown
                }
            })));
        }
        value.binding = Some(binding);
        value
    }

    fn identifier(&mut self, symbol: Option<SymbolId>, name: &str) -> Computed {
        let value = if let Some(symbol) = symbol {
            if let Some(value) = self.inputs.locals.get(&symbol) {
                if !self.spend_size(value.size()) {
                    return self.unknown();
                }
                value.clone()
            } else if let Some(index) = self.inputs.symbols.get(&symbol) {
                self.bound(self.inputs.contracts[*index].binding().clone())
            } else if let Some(binding) = self.inputs.imports.get(&symbol) {
                self.bound(binding.clone())
            } else {
                Value::unknown()
            }
        } else if name == "undefined" {
            Value::void()
        } else {
            self.bound(Binding::Global(vec![name.to_owned()]))
        };
        Computed {
            value,
            effects: if symbol.is_none()
                && self
                    .inputs
                    .global_symbols
                    .iter()
                    .any(|symbol| symbol.name() == name && symbol.read_effects())
            {
                HazardExpr::Unknown
            } else {
                HazardExpr::Bool(false)
            },
        }
    }
}

impl Transfer<'_> {
    fn evaluate(&mut self, plan: &Plan<'_>) -> Computed {
        let mut results = Vec::new();
        results.resize_with(plan.nodes.len(), || None);
        let mut step = 0;
        while step < plan.steps.len() {
            match plan.steps[step] {
                Step::Enter(id) => {
                    if plan.nodes[id].charge && (self.opaque || !self.spend()) {
                        results[id] = Some((self.unknown(), None));
                        step = plan.nodes[id].end + 1;
                        continue;
                    }
                }
                Step::Input(_) => {}
                Step::Ready(id) => {
                    let (computed, receiver) = {
                        let mut values =
                            plan.inputs[plan.nodes[id].inputs.clone()]
                                .iter()
                                .map(|input| {
                                    results[input.node].take().expect("ready expression input")
                                });
                        let mut receiver = None;
                        let computed = match &plan.nodes[id].op {
                            Op::Primitive => Computed::pure(Value::primitive()),
                            Op::This => Computed::pure(if self.inputs.arrow {
                                Value::unknown()
                            } else {
                                Value::input("this".into())
                            }),
                            Op::Identifier { symbol, name, .. } => self.identifier(*symbol, name),
                            Op::Member {
                                property,
                                reference,
                                ..
                            } => {
                                let object = values.next().expect("member object").0;
                                if *reference {
                                    receiver = Some(object.value.clone());
                                }
                                self.member(object, *property, values.next().map(|value| value.0))
                            }
                            Op::Call { pure, spread } => {
                                let (callee, receiver) = values.next().expect("callee");
                                if *spread || (values.len() > 0 && self.opaque) {
                                    self.unknown()
                                } else {
                                    let mut effects = vec![callee.effects];
                                    let arguments = values
                                        .map(|(value, _)| {
                                            effects.push(value.effects);
                                            value.value
                                        })
                                        .collect();
                                    let result = self.invocation(
                                        callee.value,
                                        arguments,
                                        receiver.unwrap_or_else(Value::void),
                                        *pure,
                                    );
                                    effects.push(result.effects);
                                    Computed {
                                        value: result.value,
                                        effects: union(effects),
                                    }
                                }
                            }
                            Op::Binary(operator) => Computed::binary(
                                operator,
                                values.next().expect("left").0,
                                values.next().expect("right").0,
                            ),
                            Op::Unary(operator) => {
                                Computed::unary(operator, values.next().expect("argument").0)
                            }
                            Op::Logical => Computed::alternatives(
                                values.next().expect("left").0,
                                values.next().expect("right").0,
                            ),
                            Op::Conditional => Computed::conditional(
                                values.next().expect("test").0,
                                values.next().expect("left").0,
                                values.next().expect("right").0,
                            ),
                            Op::Sequence => {
                                let mut effects = Vec::new();
                                let mut last = Value::void();
                                for (value, _) in values {
                                    effects.push(value.effects);
                                    last = value.value;
                                }
                                Computed {
                                    value: last,
                                    effects: union(effects),
                                }
                            }
                            Op::Template
                            | Op::Object {
                                unsupported: false, ..
                            } => {
                                let mut effects = Vec::new();
                                for ((value, _), input) in
                                    values.zip(&plan.inputs[plan.nodes[id].inputs.clone()])
                                {
                                    effects.push(value.effects);
                                    if input.role == Role::Coerce {
                                        effects.push(value.value.hazard(Hazard::ToPrimitive));
                                    }
                                }
                                if let Op::Object {
                                    empty,
                                    unknown_read,
                                    ..
                                } = &plan.nodes[id].op
                                {
                                    Computed::object(*empty, *unknown_read, effects)
                                } else {
                                    Computed {
                                        value: Value::primitive(),
                                        effects: union(effects),
                                    }
                                }
                            }
                            Op::Function { .. } => Computed::pure(Value::unknown()),
                            _ => self.unknown(),
                        };
                        (computed, receiver)
                    };
                    results[id] = Some((computed, receiver));
                }
            }
            step += 1;
        }
        results[0].take().expect("root summary").0
    }
}

impl Transfer<'_> {
    fn member(
        &self,
        mut object: Computed,
        property: Option<&str>,
        key: Option<Computed>,
    ) -> Computed {
        let binding = match (object.value.binding.take(), property) {
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
}

impl<'a> Analyzer<'_, 'a> {
    fn expression(&mut self, expression: &Expression<'a>) -> Computed {
        let outcome = self.expression_outcome(expression);
        self.remaining -= outcome.budget_used;
        self.opaque = outcome.opaque;
        for update in outcome.updates {
            if !self.updates.contains(&update) {
                self.updates.push(update);
            }
        }
        outcome.computed
    }

    fn expression_outcome(&self, expression: &Expression<'a>) -> TransferOutcome {
        let inputs = TransferInputs {
            contracts: self.contracts,
            symbols: self.symbols,
            bindings: self.bindings,
            imports: self.imports,
            prefixes: self.prefixes,
            global_symbols: self.global_symbols,
            locals: &self.locals,
            arrow: self.arrow,
        };
        let mut transfer = Transfer::new(inputs, self.remaining, self.opaque);
        if self.opaque || self.remaining == 0 {
            let computed = transfer.unknown();
            return transfer.finish(computed);
        }
        // Every uncharged member has a charged call parent: exceeding this
        // projection bound already implies visit-budget exhaustion.
        let computed = if let Some(plan) = Plan::lower(
            expression,
            self.semantic,
            Mode::Contract,
            self.remaining * 2 + 1,
        ) {
            transfer.evaluate(&plan)
        } else {
            transfer.remaining = 0;
            transfer.unknown()
        };
        transfer.finish(computed)
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
}

impl Transfer<'_> {
    fn join(values: &[Value]) -> Value {
        let fallback = Value::void();
        let first = values.first().unwrap_or(&fallback);
        let hazards = match &first.hazards {
            ValueHazards::Pure
                if values
                    .iter()
                    .all(|value| matches!(value.hazards, ValueHazards::Pure)) =>
            {
                ValueHazards::Pure
            }
            ValueHazards::Unknown
                if values
                    .iter()
                    .all(|value| matches!(value.hazards, ValueHazards::Unknown)) =>
            {
                ValueHazards::Unknown
            }
            ValueHazards::Input
                if values.iter().all(|value| {
                    value.origin == first.origin && matches!(value.hazards, ValueHazards::Input)
                }) =>
            {
                ValueHazards::Input
            }
            _ => ValueHazards::Fields(Box::new(std::array::from_fn(|index| {
                union(values.iter().map(|value| value.hazard(Hazard::ALL[index])))
            }))),
        };
        let origin = if values.iter().all(|value| value.origin == first.origin) {
            first.origin.clone()
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
            hazards,
            callable: None,
            binding: None,
        }
    }

    fn invocation(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        receiver: Value,
        pure: bool,
    ) -> Computed {
        let opaque = self.opaque;
        let mut result = self.invoke(function, arguments, receiver, !pure);
        if pure {
            // Only invocation opacity is masked; inputs and spent work survive.
            self.opaque = opaque;
            result.effects = HazardExpr::Bool(false);
        }
        result
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
            let definition = &self.inputs.contracts[index];
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
                    definition,
                )
            };
            if execute {
                self.execute_calls(
                    &definition.hazards().get(Hazard::Call),
                    &frame,
                    &arguments,
                    definition,
                );
                for update in definition.updates() {
                    let expression =
                        self.substitute(update.expression(), &frame, &arguments, definition);
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
                    hazards: ValueHazards::Fields(Box::new(std::array::from_fn(|index| {
                        self.substitute(
                            &definition.return_hazards().get(Hazard::ALL[index]),
                            &frame,
                            &arguments,
                            definition,
                        )
                    }))),
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
                    if self.spend_size(value.hazard_size(*slot)) {
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
}

impl Analyzer<'_, '_> {
    fn finish(self, binding: Binding, parameters: Vec<String>) -> Definition {
        if self.opaque {
            return unknown_contract(binding, parameters);
        }
        let value = Transfer::join(&self.returns);
        let returns = match value.origin {
            Origin::Input(name) => Returns::Argument(name),
            Origin::Void => Returns::Void,
            Origin::Fresh => Returns::Fresh,
            Origin::Unknown => Returns::Unknown,
        };
        let return_hazards = if matches!(returns, Returns::Fresh) {
            let fields = match value.hazards {
                ValueHazards::Pure => std::array::from_fn(|_| HazardExpr::Bool(false)),
                ValueHazards::Unknown => std::array::from_fn(|_| HazardExpr::Unknown),
                ValueHazards::Fields(fields) => *fields,
                ValueHazards::Input => unreachable!("fresh return has no input hazards"),
            };
            HazardFields::new(None, Hazard::ALL.into_iter().zip(fields).collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontier_planning_skips_opaque_initializers_and_respects_remaining_work() {
        let large = vec!["a"; 512].join("||");
        for (prefix, declarations, max_nodes) in [
            (
                String::new(),
                format!("const first = opaque(), second = {large}, third = {large};"),
                2,
            ),
            (
                "0;".repeat(CONTRACT_BUDGET),
                format!("const first = {large}, second = {large};"),
                CONTRACT_BUDGET,
            ),
            (
                "0;".repeat(CONTRACT_BUDGET - 1),
                format!("const first = {large}, second = {large};"),
                CONTRACT_BUDGET - 1 + 3,
            ),
        ] {
            let allocator = oxc_allocator::Allocator::default();
            let source = format!("function check(a) {{ {prefix} {declarations} }}");
            let parsed = oxc_utils_parse::parse_bundler_file(
                &allocator,
                &source,
                std::path::Path::new("planning.js"),
                oxc_utils_parse::ModuleFormat::Esm,
            )
            .unwrap();
            let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
            let before = crate::frontier::projected_nodes();
            let facts = infer_function_contracts(
                &parsed.program,
                &semantic.semantic,
                &LocalFacts::default(),
            )
            .unwrap();
            let projected = crate::frontier::projected_nodes() - before;
            assert!(
                projected <= max_nodes,
                "projected {projected} nodes, bound {max_nodes}"
            );
            assert_eq!(facts.definitions()[0].returns(), &Returns::Unknown);
            assert_eq!(
                facts.definitions()[0].hazards().get(Hazard::Call),
                HazardExpr::Unknown
            );
        }
    }

    #[test]
    fn transfer_outcomes_isolate_updates_opacity_and_work() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<TransferInputs<'_>>();
        send_sync::<Transfer<'_>>();
        send_sync::<TransferOutcome>();
        send_sync::<Plan<'_>>();

        let facts = parse_declarations(
            "global fn mutate(target) {
                call_effects = false
                updates target.property_read_effects |= true
                returns = target
            }",
        )
        .unwrap();
        let symbols = HashMap::new();
        let bindings = facts
            .definitions()
            .iter()
            .enumerate()
            .map(|(index, definition)| (definition.binding().clone(), index))
            .collect();
        let imports = HashMap::new();
        let prefixes = HashSet::new();
        let locals = HashMap::new();
        let inputs = TransferInputs {
            contracts: facts.definitions(),
            symbols: &symbols,
            bindings: &bindings,
            imports: &imports,
            prefixes: &prefixes,
            global_symbols: &[],
            locals: &locals,
            arrow: false,
        };
        for pure in [false, true] {
            let mut transfer = Transfer::new(inputs, 10, false);
            let function = transfer.bound(Binding::Global(vec!["mutate".into()]));
            let computed = transfer.invocation(
                function,
                vec![Value::input("input".into())],
                Value::void(),
                pure,
            );
            let outcome = transfer.finish(computed);
            assert!(
                matches!(outcome.computed.value.origin, Origin::Input(ref name) if name == "input")
            );
            assert_eq!(outcome.computed.effects, HazardExpr::Bool(false));
            assert!(!outcome.opaque);
            assert_eq!(outcome.budget_used, if pure { 1 } else { 3 });
            assert_eq!(
                outcome.updates,
                if pure {
                    Vec::new()
                } else {
                    vec![Update::new(
                        "input".into(),
                        Some(Hazard::PropertyRead),
                        HazardExpr::Bool(true),
                    )]
                }
            );
        }
        for pure in [false, true] {
            let mut transfer = Transfer::new(inputs, 10, false);
            let computed = transfer.invocation(Value::unknown(), Vec::new(), Value::void(), pure);
            let outcome = transfer.finish(computed);
            assert!(matches!(outcome.computed.value.origin, Origin::Unknown));
            assert_eq!(outcome.opaque, !pure);
            assert_eq!(outcome.budget_used, 1);
            assert!(outcome.updates.is_empty());
            assert_eq!(
                outcome.computed.effects,
                if pure {
                    HazardExpr::Bool(false)
                } else {
                    HazardExpr::Unknown
                }
            );
        }
        let mut transfer = Transfer::new(inputs, 2, false);
        let function = transfer.bound(Binding::Global(vec!["mutate".into()]));
        let computed = transfer.invocation(
            function,
            vec![Value::input("input".into())],
            Value::void(),
            false,
        );
        let outcome = transfer.finish(computed);
        assert!(outcome.opaque);
        assert_eq!(outcome.budget_used, 2);
        assert_eq!(outcome.updates[0].expression(), &HazardExpr::Unknown);

        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            "/*#__PURE__*/ mutate(0);",
            std::path::Path::new("budget.js"),
            oxc_utils_parse::ModuleFormat::Esm,
        )
        .unwrap();
        let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
        let Statement::ExpressionStatement(statement) = &parsed.program.body[0] else {
            unreachable!("expression fixture")
        };
        let plan = Plan::lower(
            &statement.expression,
            &semantic.semantic,
            Mode::Contract,
            16,
        )
        .unwrap();
        for budget in [2, 3, 4] {
            let mut transfer = Transfer::new(inputs, budget, false);
            let computed = transfer.evaluate(&plan);
            let outcome = transfer.finish(computed);
            assert_eq!(outcome.budget_used, budget);
            // Two visits exhaust work in an argument; three exhaust it only
            // inside the masked invocation. Neither path refunds work.
            assert_eq!(outcome.opaque, budget == 2);
            assert_eq!(
                outcome.computed.effects,
                if budget == 2 {
                    HazardExpr::Unknown
                } else {
                    HazardExpr::Bool(false)
                }
            );
            assert_eq!(
                matches!(outcome.computed.value.origin, Origin::Fresh),
                budget == 4
            );
            let mut next = Transfer::new(inputs, budget - outcome.budget_used, outcome.opaque);
            let computed = next.evaluate(&plan);
            assert!(next.finish(computed).opaque);
        }
    }
}
