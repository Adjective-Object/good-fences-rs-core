use super::*;
use crate::views::{
    DefinitionView, FunctionIndex, FunctionSource, FunctionView, GlobalSymbolView, InstructionView,
    ParsedFileSource, ParsedFileView,
};

#[derive(Debug, Clone)]
pub(super) struct Closure {
    template: usize,
    captures: HashMap<ValueId, ValueId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instruction {
    Value(ValueId, Hazards),
    Any(Effect, Vec<Effect>),
    Equality(Effect, ValueId, ValueId),
    Operation(ValueId, Hazard),
    Update(ValueId, Hazard, Effect),
    Read(ValueId, ValueId),
    Retain(ValueId, Vec<ValueId>),
    Bind(ValueId, ValueId),
    Join(ValueId, Vec<ValueId>),
    Call(CallResult, ValueId, Vec<ValueId>, ValueId, bool),
    Closure(ValueId, usize),
    External(ValueId, usize),
    Opaque,
}

#[derive(Debug, Clone)]
pub(super) struct Recording {
    values: usize,
    cells: usize,
    equations: usize,
    pub instructions: Vec<Instruction>,
}

mod records;
pub use records::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct Environment {
    pub values: HashMap<ValueId, ValueId>,
    pub externals: Vec<ValueId>,
    pub functions: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Template {
    function: FunctionSource,
    environment: Arc<Environment>,
}

#[derive(Debug, Clone)]
pub(super) struct LazyRoot {
    template: Template,
    running: bool,
    result: Option<CallResult>,
}

impl Context {
    pub(crate) fn reserve_function(&mut self) -> usize {
        let index = self.templates.len();
        self.templates.push(Arc::default());
        index
    }

    pub(crate) fn start_function(&mut self) {
        assert!(self.recording.is_none());
        self.recording = Some(Recording {
            values: self.values.len(),
            cells: self.initial.len(),
            equations: self.equations.len(),
            instructions: Vec::new(),
        });
    }

    pub(crate) fn function_local(&mut self, value: ValueId) {
        let instructions = &mut self
            .recording
            .as_mut()
            .expect("function recording")
            .instructions;
        instructions.push(Instruction::Value(value, Hazards::NONE));
        instructions.push(Instruction::Bind(value, self.undefined));
    }

    pub(crate) fn finish_function(
        &mut self,
        index: usize,
        signature: FunctionSignature,
        returns: Vec<ValueId>,
        effects: Effect,
    ) {
        let function = self.finish_recording(signature, returns, effects);
        self.templates[index] = Arc::new(Template {
            function: FunctionSource::Owned(Arc::new(function)),
            environment: Arc::default(),
        });
    }

    pub(crate) fn finish_recording(
        &mut self,
        signature: FunctionSignature,
        returns: Vec<ValueId>,
        effects: Effect,
    ) -> OwnedFunction {
        let recording = self.recording.take().expect("function recording");
        let mut used = HashSet::from([effects]);
        for instruction in &recording.instructions {
            match instruction {
                Instruction::Any(_, inputs) => used.extend(inputs),
                Instruction::Update(_, _, effect) => {
                    used.insert(*effect);
                }
                _ => {}
            }
        }
        let sources = used
            .into_iter()
            .filter_map(|effect| {
                let index = self
                    .values
                    .partition_point(|value| value.cells[0] <= effect.0)
                    .checked_sub(1)?;
                let slot = Hazard::ALL
                    .into_iter()
                    .find(|slot| self.values[index].cells[*slot as usize] == effect.0)?;
                Some((effect, (ValueId(index), slot)))
            })
            .collect();
        let function = OwnedFunction::new(
            signature,
            recording.instructions,
            returns,
            Some(effects),
            sources,
        );
        self.values.truncate(recording.values);
        self.initial.truncate(recording.cells);
        self.equations.truncate(recording.equations);
        function
    }

    pub(crate) fn read_binding(&mut self, source: ValueId) -> ValueId {
        if self.recording.is_some() {
            let value = self.value(Hazards::NONE);
            self.recording
                .as_mut()
                .expect("function recording")
                .instructions
                .push(Instruction::Read(value, source));
            value
        } else {
            if let Some(root) = self.producers.get(&source).copied() {
                if self.roots[root].running && self.roots[root].result.is_none() {
                    return self.unknown;
                }
                self.run_root(root);
            }
            self.resolve_alias(source)
        }
    }

    pub(crate) fn external_value(&mut self, reference: usize) -> ValueId {
        let value = self.value(Hazards::NONE);
        self.recording
            .as_mut()
            .expect("external reference recording")
            .instructions
            .push(Instruction::External(value, reference));
        value
    }

    pub(crate) fn loose_equality(&mut self, left: ValueId, right: ValueId) -> Effect {
        if self.recording.is_some() {
            let effect = self.expression(Expression::Constant(false));
            self.recording
                .as_mut()
                .expect("function recording")
                .instructions
                .push(Instruction::Equality(effect, left, right));
            return effect;
        }
        let left = self.resolve_alias(left);
        let right = self.resolve_alias(right);
        if left == right && left != self.unknown {
            // Opaque writes can separate bindings that share a recorded identity.
            return self.poison;
        }
        let left = self.operation(left, Hazard::ToPrimitive);
        let right = self.operation(right, Hazard::ToPrimitive);
        self.any([left, right])
    }

    pub(crate) fn set_binding(&mut self, target: ValueId, value: ValueId) {
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Bind(target, value));
            return;
        }
        if target != value {
            let value = self.resolve_alias(value);
            if self.resolve_alias(target) != value {
                for cell in std::mem::take(&mut self.values[target.0].binding_readers) {
                    self.initial[cell] = true;
                }
            }
            let value = if self.values[target.0].retained_capture {
                self.join_values(&[self.resolve_alias(target), value])
            } else {
                value
            };
            self.values[target.0].alias = Some(value);
        }
    }

    pub(crate) fn retain_captures(&mut self, value: ValueId, locals: &[ValueId]) {
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Retain(value, locals.to_vec()));
            return;
        }
        let mut pending = vec![value];
        let mut visited = HashSet::new();
        while let Some(value) = pending.pop() {
            let value = self.resolve_alias(value);
            if !visited.insert(value) {
                continue;
            }
            if let Some(closure) = self.values[value.0].closure.clone() {
                for capture in closure.captures.values() {
                    if locals.contains(capture) {
                        self.values[capture.0].retained_capture = true;
                    }
                    pending.push(*capture);
                }
            }
        }
    }

    pub(crate) fn bind_value(&mut self, target: ValueId, value: ValueId) {
        if target == value {
            return;
        }
        self.values[target.0].alias = Some(value);
        self.link_hazards(target, value);
    }

    pub(super) fn link_hazards(&mut self, target: ValueId, value: ValueId) {
        let targets = self.values[target.0].cells.into_iter().chain([
            self.values[target.0].call_mutations.0,
            self.values[target.0].external_aliases.0,
        ]);
        let values = self.values[value.0].cells.into_iter().chain([
            self.values[value.0].call_mutations.0,
            self.values[value.0].external_aliases.0,
        ]);
        for (target, value) in targets.zip(values) {
            self.equations.push(Equation {
                target,
                expression: Expression::Read(value),
            });
            if !self.latent {
                self.equations.push(Equation {
                    target: value,
                    expression: Expression::Read(target),
                });
            }
        }
    }

    pub(super) fn resolve_alias(&self, mut value: ValueId) -> ValueId {
        let mut visited = HashSet::new();
        while let Some(alias) = self.values[value.0].alias {
            if !visited.insert(value) {
                return self.unknown;
            }
            value = alias;
        }
        value
    }

    pub(crate) fn function_value(&mut self, index: usize) -> ValueId {
        let value = self.value(Hazards::NONE);
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Closure(value, index));
        } else {
            self.install_closure(value, index, &HashMap::new());
        }
        value
    }

    fn install_closure(
        &mut self,
        value: ValueId,
        index: usize,
        environment: &HashMap<ValueId, ValueId>,
    ) {
        let captures = self.templates[index]
            .function
            .view()
            .signature()
            .captures()
            .iter()
            .map(|capture| (capture, mapped(environment, capture)))
            .collect();
        // A bare call hazard has no arguments to specialize. Keep it separate
        // from replacement hazards consulted by a known, instantiated call.
        self.initial[self.values[value.0].cells[Hazard::Call as usize]] =
            self.templates[index].function.view().effects() != Some(self.pure);
        self.initial[self.values[value.0].cells[Hazard::Construct as usize]] = self.templates
            [index]
            .function
            .view()
            .signature()
            .constructible()
            && self.templates[index].function.view().effects() != Some(self.pure);
        self.values[value.0].closure = Some(Closure {
            template: index,
            captures,
        });
    }

    pub(super) fn call_inferred(
        &mut self,
        function: ValueId,
        resolved: ValueId,
        closure: Closure,
        arguments: &[ValueId],
        receiver: ValueId,
    ) -> CallResult {
        let invocation = Invocation {
            function: resolved,
            arguments: arguments
                .iter()
                .map(|value| self.resolve_alias(*value))
                .collect(),
            receiver: self.resolve_alias(receiver),
        };
        if let Some(result) = self.inferred_calls.get(&invocation) {
            return *result;
        }
        // Exact recursive contexts share graph cells; changing contexts widen.
        if !self.expanding_templates.insert(closure.template) {
            return CallResult::new(self.unknown, self.opaque_effects());
        }
        let pending = CallResult::new(
            self.value(Hazards::NONE),
            self.expression(Expression::Constant(false)),
        );
        self.inferred_calls.insert(invocation.clone(), pending);
        let template = self.templates[closure.template].clone();
        let mut capture_dependencies = Vec::new();
        let mut captures: Vec<_> = closure.captures.values().copied().collect();
        captures.extend(
            template
                .function
                .view()
                .instructions()
                .iter()
                .filter_map(|instruction| {
                    if let InstructionView::External(_, reference) = instruction {
                        Some(template.environment.externals[reference])
                    } else {
                        None
                    }
                }),
        );
        captures.sort_unstable();
        captures.dedup();
        for capture in captures {
            self.read_binding(capture);
            let cell = self.initial.len();
            self.initial.push(false);
            self.values[capture.0].binding_readers.push(cell);
            capture_dependencies.push(Effect(cell));
        }
        let mut values = template.environment.values.clone();
        values.extend(closure.captures);
        for (index, parameter) in template
            .function
            .view()
            .signature()
            .parameters()
            .iter()
            .enumerate()
        {
            let slot = self.value(Hazards::NONE);
            self.set_binding(
                slot,
                arguments.get(index).copied().unwrap_or(self.undefined),
            );
            values.insert(parameter, slot);
        }
        if let Some(parameter) = template.function.view().signature().receiver() {
            values.insert(parameter, receiver);
        }
        if let Some(binding) = template.function.view().signature().arguments() {
            values.insert(binding, self.value(Hazards::UNKNOWN));
        }
        if let Some(binding) = template.function.view().signature().self_binding() {
            values.insert(binding, resolved);
        }
        let (value, body, hypothetical_writes) = self.replay(&template, values);
        self.bind_value(pending.value(), value);
        let effects = self.any(capture_dependencies.into_iter().chain([
            body,
            self.values[function.0].call_mutations,
            self.values[resolved.0].call_mutations,
            if hypothetical_writes {
                self.effectful
            } else {
                self.pure
            },
        ]));
        self.equations.push(Equation {
            target: pending.effects().0,
            expression: Expression::Read(effects.0),
        });
        self.inferred_calls.remove(&invocation);
        self.expanding_templates.remove(&closure.template);
        CallResult::new(value, pending.effects())
    }

    fn replay(
        &mut self,
        template: &Template,
        mut values: HashMap<ValueId, ValueId>,
    ) -> (ValueId, Effect, bool) {
        let mut effects = HashMap::from([(Effect(0), self.pure), (Effect(1), self.effectful)]);
        let mut hypothetical_writes = false;
        let function = template.function.view();
        for instruction in function.instructions() {
            match instruction {
                InstructionView::Value(target, hazards) => {
                    let value = self.value(hazards);
                    values.insert(target, value);
                }
                InstructionView::Any(target, inputs) => {
                    let inputs: Vec<_> = inputs
                        .iter()
                        .map(|effect| self.template_effect(effect, function, &values, &effects))
                        .collect();
                    effects.insert(target, self.any(inputs));
                }
                InstructionView::Equality(target, left, right) => {
                    let effect = self.loose_equality(mapped(&values, left), mapped(&values, right));
                    effects.insert(target, effect);
                }
                InstructionView::Operation(value, slot) => {
                    self.operation(mapped(&values, value), slot);
                }
                InstructionView::Update(value, slot, effect) => {
                    if !self.latent {
                        let effect = self.template_effect(effect, function, &values, &effects);
                        self.update(mapped(&values, value), slot, effect);
                    } else {
                        hypothetical_writes = true;
                    }
                }
                InstructionView::Bind(target, source) => {
                    self.set_binding(mapped(&values, target), mapped(&values, source));
                }
                InstructionView::Read(target, source) => {
                    let value = self.read_binding(mapped(&values, source));
                    values.insert(target, value);
                }
                InstructionView::Retain(value, locals) => {
                    let locals: Vec<_> =
                        locals.iter().map(|local| mapped(&values, local)).collect();
                    self.retain_captures(mapped(&values, value), &locals);
                }
                InstructionView::Join(target, inputs) => {
                    let inputs: Vec<_> =
                        inputs.iter().map(|value| mapped(&values, value)).collect();
                    values.insert(target, self.join_values(&inputs));
                }
                InstructionView::Call(target, function, arguments, receiver, pure) => {
                    let arguments: Vec<_> = arguments
                        .iter()
                        .map(|value| mapped(&values, value))
                        .collect();
                    let result = self.call_with_annotation(
                        mapped(&values, function),
                        &arguments,
                        Some(mapped(&values, receiver)),
                        pure,
                    );
                    values.insert(target.value(), result.value());
                    effects.insert(target.effects(), result.effects());
                }
                InstructionView::Closure(target, index) => {
                    self.install_closure(
                        mapped(&values, target),
                        template.environment.functions + index,
                        &values,
                    );
                }
                InstructionView::External(target, reference) => {
                    let value = self.read_binding(template.environment.externals[reference]);
                    values.insert(target, value);
                }
                InstructionView::Opaque => {
                    self.opaque_effects();
                }
            }
        }
        let returns: Vec<_> = function
            .returns()
            .iter()
            .map(|value| self.resolve_alias(mapped(&values, value)))
            .collect();
        let value = if returns.is_empty() {
            self.undefined
        } else {
            self.join_values(&returns)
        };
        let body = function.effects().map_or(self.effectful, |effect| {
            self.template_effect(effect, function, &values, &effects)
        });
        (value, body, hypothetical_writes)
    }

    fn template_effect(
        &self,
        effect: Effect,
        template: FunctionView<'_>,
        values: &HashMap<ValueId, ValueId>,
        effects: &HashMap<Effect, Effect>,
    ) -> Effect {
        if let Some(effect) = effects.get(&effect) {
            return *effect;
        }
        let (value, slot) = template
            .sources()
            .get(&effect)
            .expect("validated effect source");
        self.hazard(mapped(values, value), slot)
    }

    pub(crate) fn parsed_values(&self) -> Vec<crate::analysis::ParsedValue> {
        self.values
            .iter()
            .map(|value| {
                crate::analysis::ParsedValue::new(
                    Hazards(value.cells.map(|cell| self.initial[cell])),
                    value.definition,
                    value.alias,
                )
            })
            .collect()
    }

    pub(crate) fn owned_functions(&self) -> Vec<OwnedFunction> {
        self.templates
            .iter()
            .map(|t| t.function.extraction_owned())
            .collect()
    }

    pub(crate) fn install_values(
        &mut self,
        parsed: ParsedFileView<'_>,
    ) -> HashMap<ValueId, ValueId> {
        let start = self.facts.definitions().len();
        Arc::make_mut(&mut self.facts).definitions_mut().extend(
            parsed
                .declarations()
                .definitions()
                .iter()
                .map(DefinitionView::to_owned),
        );
        Arc::make_mut(&mut self.facts).global_symbols_mut().extend(
            parsed
                .declarations()
                .global_symbols()
                .iter()
                .map(GlobalSymbolView::to_owned),
        );
        let mut values = HashMap::new();
        for (index, value) in parsed.values().iter().enumerate() {
            let runtime = match index {
                0 => self.undefined,
                1 => self.unknown,
                _ => self.value(value.hazards()),
            };
            if let Some(definition) = value.definition() {
                self.values[runtime.0].definition = Some(start + definition);
                self.bindings
                    .entry(
                        parsed
                            .declarations()
                            .definitions()
                            .get(definition)
                            .expect("validated definition")
                            .binding()
                            .to_owned(),
                    )
                    .or_insert(start + definition);
                self.functions.push(runtime);
            }
            values.insert(ValueId(index), runtime);
        }
        for (index, value) in parsed.values().iter().enumerate() {
            if let Some(alias) = value.alias() {
                self.values[values[&ValueId(index)].0].alias = Some(values[&alias]);
            }
        }
        self.instantiate_declarations(start);
        values
    }

    pub(crate) fn install_file(
        &mut self,
        parsed: Arc<dyn ParsedFileSource>,
        values: HashMap<ValueId, ValueId>,
        externals: Vec<ValueId>,
    ) -> (usize, Vec<usize>) {
        let environment = Arc::new(Environment {
            values,
            externals,
            functions: self.templates.len(),
        });
        for index in 0..parsed.view().functions().len() {
            self.templates.push(Arc::new(Template {
                function: FunctionSource::Parsed {
                    file: parsed.clone(),
                    index: FunctionIndex::Function(index),
                },
                environment: environment.clone(),
            }));
        }
        let setup = self.roots.len();
        for selector in std::iter::once(FunctionIndex::Setup)
            .chain((0..parsed.view().roots().len()).map(FunctionIndex::Root))
        {
            let function = FunctionSource::Parsed {
                file: parsed.clone(),
                index: selector,
            };
            let index = self.roots.len();
            for instruction in function.view().instructions() {
                if let InstructionView::Bind(target, _) = instruction {
                    if let Some(target) = environment.values.get(&target) {
                        self.producers.entry(*target).or_insert(index);
                    }
                }
            }
            self.roots.push(LazyRoot {
                template: Template {
                    function,
                    environment: environment.clone(),
                },
                running: false,
                result: None,
            });
        }
        (setup, (setup + 1..self.roots.len()).collect())
    }

    pub(crate) fn run_root(&mut self, root: usize) -> CallResult {
        if let Some(result) = self.roots[root].result {
            return result;
        }
        if self.roots[root].running {
            return CallResult::new(self.unknown, self.effectful);
        }
        self.roots[root].running = true;
        let template = self.roots[root].template.clone();
        let latent = self.latent;
        self.latent = false;
        let (value, effects, _) = self.replay(&template, template.environment.values.clone());
        self.latent = latent;
        let result = CallResult::new(value, effects);
        if let Some(output) = self.root_outputs.get(&root).copied() {
            self.bind_value(output, value);
        }
        self.roots[root].result = Some(result);
        result
    }

    pub(crate) fn root_output(&mut self, root: usize, output: ValueId) {
        self.root_outputs.insert(root, output);
        self.producers.insert(output, root);
    }

    pub(crate) fn declaration_value(
        &self,
        parsed: ParsedFileView<'_>,
        values: &HashMap<ValueId, ValueId>,
        definition: usize,
    ) -> ValueId {
        let index = parsed
            .values()
            .iter()
            .position(|v| v.definition() == Some(definition))
            .expect("validated declaration slot");
        values[&ValueId(index)]
    }
}

impl ParsedFileView<'_> {
    pub(crate) fn valid_value(&self, value: ValueId) -> bool {
        value.0 < self.values().len()
    }

    pub(crate) fn validate_function(&self, function: FunctionView<'_>) -> eyre::Result<()> {
        use eyre::ensure;

        let signature = function.signature();
        for value in signature
            .captures()
            .iter()
            .chain(signature.parameters())
            .chain(signature.receiver())
            .chain(signature.self_binding())
            .chain(signature.arguments())
        {
            ensure!(self.valid_value(value), "invalid Ecco function binding");
        }
        let mut values: HashSet<_> = (0..self.values().len()).map(ValueId).collect();
        let mut effects = HashSet::from([Effect(0), Effect(1)]);
        let check_value = |value: &ValueId, values: &HashSet<ValueId>| {
            ensure!(values.contains(value), "undefined Ecco template value");
            Ok::<_, eyre::Report>(())
        };
        let check_effect =
            |effect: &Effect, values: &HashSet<ValueId>, effects: &HashSet<Effect>| {
                ensure!(
                    effects.contains(effect)
                        || function
                            .sources()
                            .get(effect)
                            .is_some_and(|(value, _)| values.contains(&value)),
                    "undefined Ecco template effect"
                );
                Ok::<_, eyre::Report>(())
            };
        for instruction in function.instructions() {
            match instruction {
                InstructionView::Value(target, _) => {
                    ensure!(target.0 >= 2, "cannot replace Ecco constants");
                    values.insert(target);
                }
                InstructionView::Any(target, inputs) => {
                    for input in inputs {
                        check_effect(&input, &values, &effects)?;
                    }
                    effects.insert(target);
                }
                InstructionView::Equality(target, left, right) => {
                    check_value(&left, &values)?;
                    check_value(&right, &values)?;
                    effects.insert(target);
                }
                InstructionView::Operation(value, _) => check_value(&value, &values)?,
                InstructionView::Update(value, _, effect) => {
                    check_value(&value, &values)?;
                    check_effect(&effect, &values, &effects)?;
                }
                InstructionView::Bind(target, source) | InstructionView::Read(target, source) => {
                    check_value(&target, &values)?;
                    check_value(&source, &values)?;
                    ensure!(target.0 >= 2, "cannot rebind Ecco constants");
                }
                InstructionView::Retain(value, locals) => {
                    check_value(&value, &values)?;
                    for local in locals {
                        check_value(&local, &values)?;
                    }
                }
                InstructionView::Join(target, inputs) => {
                    check_value(&target, &values)?;
                    for input in inputs {
                        check_value(&input, &values)?;
                    }
                }
                InstructionView::Call(target, callee, arguments, receiver, _) => {
                    check_value(&target.value(), &values)?;
                    check_value(&callee, &values)?;
                    check_value(&receiver, &values)?;
                    for argument in arguments {
                        check_value(&argument, &values)?;
                    }
                    effects.insert(target.effects());
                }
                InstructionView::Closure(target, index) => {
                    check_value(&target, &values)?;
                    ensure!(
                        index < self.functions().len(),
                        "invalid Ecco function index"
                    );
                }
                InstructionView::External(target, index) => {
                    check_value(&target, &values)?;
                    ensure!(
                        index < self.external_references().len(),
                        "invalid Ecco external reference"
                    );
                }
                InstructionView::Opaque => {}
            }
        }
        for value in function.returns() {
            check_value(&value, &values)?;
        }
        let effect = function
            .effects()
            .ok_or_else(|| eyre::eyre!("missing Ecco body effect"))?;
        check_effect(&effect, &values, &effects)?;
        Ok(())
    }
}

fn mapped(values: &HashMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    values.get(&value).copied().unwrap_or(value)
}
