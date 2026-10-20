use super::*;

#[derive(Debug, Clone)]
pub(super) struct Closure {
    template: usize,
    captures: HashMap<ValueId, ValueId>,
}

#[derive(Debug, Clone)]
pub(super) enum Instruction {
    Value(ValueId, Hazards),
    Any(Effect, Vec<Effect>),
    Equality(Effect, ValueId, ValueId),
    Operation(ValueId, Hazard),
    Update(ValueId, Hazard, Effect),
    Read(ValueId, ValueId),
    Retain(ValueId, Vec<ValueId>),
    Bind(ValueId, ValueId),
    Join(ValueId, Vec<ValueId>),
    Call(CallResult, ValueId, Vec<ValueId>, ValueId),
    Closure(ValueId, usize),
    Opaque,
}

#[derive(Debug, Clone)]
pub(super) struct Recording {
    values: usize,
    cells: usize,
    equations: usize,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FunctionSignature {
    pub captures: Vec<ValueId>,
    pub parameters: Vec<ValueId>,
    pub receiver: Option<ValueId>,
    pub self_binding: Option<ValueId>,
    pub constructible: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Template {
    signature: FunctionSignature,
    instructions: Vec<Instruction>,
    returns: Vec<ValueId>,
    effects: Option<Effect>,
    sources: HashMap<Effect, (ValueId, Hazard)>,
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
        self.templates[index] = Arc::new(Template {
            signature,
            instructions: recording.instructions,
            returns,
            effects: Some(effects),
            sources,
        });
        self.values.truncate(recording.values);
        self.initial.truncate(recording.cells);
        self.equations.truncate(recording.equations);
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
            self.resolve_alias(source)
        }
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
            .signature
            .captures
            .iter()
            .map(|capture| (*capture, mapped(environment, *capture)))
            .collect();
        // A bare call hazard has no arguments to specialize. Keep it separate
        // from replacement hazards consulted by a known, instantiated call.
        self.initial[self.values[value.0].cells[Hazard::Call as usize]] =
            self.templates[index].effects != Some(self.pure);
        self.initial[self.values[value.0].cells[Hazard::Construct as usize]] =
            self.templates[index].signature.constructible
                && self.templates[index].effects != Some(self.pure);
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
            return CallResult {
                value: self.unknown,
                effects: self.opaque_effects(),
            };
        }
        let pending = CallResult {
            value: self.value(Hazards::NONE),
            effects: self.expression(Expression::Constant(false)),
        };
        self.inferred_calls.insert(invocation.clone(), pending);
        let template = self.templates[closure.template].clone();
        let mut capture_dependencies = Vec::new();
        for capture in closure.captures.values() {
            let cell = self.initial.len();
            self.initial.push(false);
            self.values[capture.0].binding_readers.push(cell);
            capture_dependencies.push(Effect(cell));
        }
        let mut values = closure.captures;
        for (index, parameter) in template.signature.parameters.iter().enumerate() {
            let slot = self.value(Hazards::NONE);
            self.set_binding(
                slot,
                arguments.get(index).copied().unwrap_or(self.undefined),
            );
            values.insert(*parameter, slot);
        }
        if let Some(parameter) = template.signature.receiver {
            values.insert(parameter, receiver);
        }
        if let Some(binding) = template.signature.self_binding {
            values.insert(binding, resolved);
        }
        let mut effects = HashMap::from([(self.pure, self.pure), (self.effectful, self.effectful)]);
        let mut hypothetical_writes = false;
        for instruction in &template.instructions {
            match instruction {
                Instruction::Value(target, hazards) => {
                    let value = self.value(*hazards);
                    values.insert(*target, value);
                }
                Instruction::Any(target, inputs) => {
                    let inputs: Vec<_> = inputs
                        .iter()
                        .map(|effect| self.template_effect(*effect, &template, &values, &effects))
                        .collect();
                    effects.insert(*target, self.any(inputs));
                }
                Instruction::Equality(target, left, right) => {
                    let effect =
                        self.loose_equality(mapped(&values, *left), mapped(&values, *right));
                    effects.insert(*target, effect);
                }
                Instruction::Operation(value, slot) => {
                    self.operation(mapped(&values, *value), *slot);
                }
                Instruction::Update(value, slot, effect) => {
                    if !self.latent {
                        let effect = self.template_effect(*effect, &template, &values, &effects);
                        self.update(mapped(&values, *value), *slot, effect);
                    } else {
                        hypothetical_writes = true;
                    }
                }
                Instruction::Bind(target, source) => {
                    self.set_binding(mapped(&values, *target), mapped(&values, *source));
                }
                Instruction::Read(target, source) => {
                    values.insert(*target, self.resolve_alias(mapped(&values, *source)));
                }
                Instruction::Retain(value, locals) => {
                    let locals: Vec<_> =
                        locals.iter().map(|local| mapped(&values, *local)).collect();
                    self.retain_captures(mapped(&values, *value), &locals);
                }
                Instruction::Join(target, inputs) => {
                    let inputs: Vec<_> =
                        inputs.iter().map(|value| mapped(&values, *value)).collect();
                    values.insert(*target, self.join_values(&inputs));
                }
                Instruction::Call(target, function, arguments, receiver) => {
                    let arguments: Vec<_> = arguments
                        .iter()
                        .map(|value| mapped(&values, *value))
                        .collect();
                    let result = self.call(
                        mapped(&values, *function),
                        &arguments,
                        Some(mapped(&values, *receiver)),
                    );
                    values.insert(target.value, result.value);
                    effects.insert(target.effects, result.effects);
                }
                Instruction::Closure(target, index) => {
                    self.install_closure(mapped(&values, *target), *index, &values);
                }
                Instruction::Opaque => {
                    self.opaque_effects();
                }
            }
        }
        let returns: Vec<_> = template
            .returns
            .iter()
            .map(|value| self.resolve_alias(mapped(&values, *value)))
            .collect();
        let value = if returns.is_empty() {
            self.undefined
        } else {
            self.join_values(&returns)
        };
        self.bind_value(pending.value, value);
        let body = template.effects.map_or(self.effectful, |effect| {
            self.template_effect(effect, &template, &values, &effects)
        });
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
            target: pending.effects.0,
            expression: Expression::Read(effects.0),
        });
        self.inferred_calls.remove(&invocation);
        self.expanding_templates.remove(&closure.template);
        CallResult {
            value,
            effects: pending.effects,
        }
    }

    fn template_effect(
        &self,
        effect: Effect,
        template: &Template,
        values: &HashMap<ValueId, ValueId>,
        effects: &HashMap<Effect, Effect>,
    ) -> Effect {
        if let Some(effect) = effects.get(&effect) {
            return *effect;
        }
        let (value, slot) = template.sources[&effect];
        self.hazard(mapped(values, value), slot)
    }
}

fn mapped(values: &HashMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    values.get(&value).copied().unwrap_or(value)
}
