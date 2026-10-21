use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::*;

mod inference;
pub(crate) use inference::FunctionSignature;
pub use inference::OwnedFunction;
use inference::{Closure, Instruction, Recording, Template};
mod records;
pub use records::CallResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ValueId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Effect(usize);

#[derive(Debug, Clone)]
enum Expression {
    Constant(bool),
    Read(usize),
    Or(Vec<Self>),
    And(Vec<Self>),
}

impl Expression {
    fn evaluate(&self, state: &[bool]) -> bool {
        match self {
            Self::Constant(value) => *value,
            Self::Read(cell) => state[*cell],
            Self::Or(expressions) => expressions.iter().any(|e| e.evaluate(state)),
            Self::And(expressions) => expressions.iter().all(|e| e.evaluate(state)),
        }
    }

    fn dependencies(&self, output: &mut Vec<usize>) {
        match self {
            Self::Constant(_) => {}
            Self::Read(cell) => output.push(*cell),
            Self::Or(expressions) | Self::And(expressions) => {
                for expression in expressions {
                    expression.dependencies(output);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Equation {
    target: usize,
    expression: Expression,
}

#[derive(Debug, Clone)]
struct Value {
    cells: [usize; Hazard::ALL.len()],
    definition: Option<usize>,
    alias: Option<ValueId>,
    closure: Option<Closure>,
    call_mutations: Effect,
    external_aliases: Effect,
    retained_capture: bool,
    binding_readers: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Invocation {
    function: ValueId,
    arguments: Vec<ValueId>,
    receiver: ValueId,
}

struct Frame<'a> {
    definition: &'a Definition,
    arguments: &'a [ValueId],
    receiver: ValueId,
    undefined: ValueId,
}

impl Frame<'_> {
    fn value(&self, name: &str) -> ValueId {
        if name == "this" {
            return self.receiver;
        }
        let index = self
            .definition
            .parameters()
            .iter()
            .position(|parameter| parameter == name)
            .expect("validated parameter reference");
        self.arguments.get(index).copied().unwrap_or(self.undefined)
    }

    fn call_arguments(&self, arguments: &[CallArgument]) -> Vec<ValueId> {
        let mut values = Vec::new();
        for argument in arguments {
            match argument {
                CallArgument::Value(name) => values.push(self.value(name)),
                CallArgument::Spread(_) => values.extend(
                    self.arguments
                        .iter()
                        .skip(self.definition.parameters().len())
                        .copied(),
                ),
                CallArgument::Forward => values.extend_from_slice(self.arguments),
            }
        }
        values
    }
}

/// An owned graph. Construction is serial; evaluation runs independent equations
/// in parallel against immutable snapshots and commits monotone updates.
#[derive(Debug, Clone)]
pub struct Context {
    facts: LocalFacts,
    values: Vec<Value>,
    functions: Vec<ValueId>,
    bindings: HashMap<Binding, usize>,
    initial: Vec<bool>,
    equations: Vec<Equation>,
    calls: HashMap<Invocation, Effect>,
    expanding: HashSet<Invocation>,
    executing: HashSet<Invocation>,
    opaque: bool,
    undefined: ValueId,
    unknown: ValueId,
    pure: Effect,
    effectful: Effect,
    poison: Effect,
    templates: Vec<Arc<Template>>,
    recording: Option<Recording>,
    inferred_calls: HashMap<Invocation, CallResult>,
    expanding_templates: HashSet<usize>,
    latent: bool,
    roots: Vec<inference::LazyRoot>,
    producers: HashMap<ValueId, usize>,
    root_outputs: HashMap<usize, ValueId>,
}

impl Context {
    pub fn from_local_facts(facts: LocalFacts) -> eyre::Result<Self> {
        let mut context = Self::recording_context(facts)?;
        context.instantiate_declarations(0);
        Ok(context)
    }

    pub(crate) fn recording_context(facts: LocalFacts) -> eyre::Result<Self> {
        crate::parser::validate_facts(&facts)?;
        let mut context = Self {
            facts,
            values: Vec::new(),
            functions: Vec::new(),
            bindings: HashMap::new(),
            initial: vec![false, true, false],
            equations: Vec::new(),
            calls: HashMap::new(),
            expanding: HashSet::new(),
            executing: HashSet::new(),
            opaque: false,
            undefined: ValueId(0),
            unknown: ValueId(0),
            pure: Effect(0),
            effectful: Effect(1),
            poison: Effect(2),
            templates: Vec::new(),
            recording: None,
            inferred_calls: HashMap::new(),
            expanding_templates: HashSet::new(),
            latent: false,
            roots: Vec::new(),
            producers: HashMap::new(),
            root_outputs: HashMap::new(),
        };
        context.undefined = context.value(Hazards::NONE);
        context.unknown = context.value(Hazards::UNKNOWN);
        context.initial[context.values[context.unknown.0].external_aliases.0] = true;
        for index in 0..context.facts.definitions().len() {
            let value = context.value(Hazards::NONE);
            context.values[value.0].definition = Some(index);
            context.functions.push(value);
            context
                .bindings
                .insert(context.facts.definitions()[index].binding().clone(), index);
        }
        Ok(context)
    }

    fn instantiate_declarations(&mut self, start: usize) {
        for index in start..self.facts.definitions().len() {
            let definition = self.facts.definitions()[index].clone();
            let arguments = vec![self.unknown; definition.parameters().len()];
            let frame = Frame {
                definition: &definition,
                arguments: &arguments,
                receiver: self.unknown,
                undefined: self.undefined,
            };
            for slot in Hazard::ALL {
                let expression = self.instantiate(&definition.hazards().get(slot), &frame);
                let target = self.values[self.functions[index].0].cells[slot as usize];
                self.equations.push(Equation { target, expression });
            }
        }
    }

    pub fn facts(&self) -> &LocalFacts {
        &self.facts
    }

    pub fn definition(&self, binding: &Binding) -> Option<ValueId> {
        self.bindings
            .get(binding)
            .map(|index| self.functions[*index])
    }

    pub fn value(&mut self, hazards: Hazards) -> ValueId {
        let cells = std::array::from_fn(|index| {
            let cell = self.initial.len();
            self.initial.push(hazards.0[index]);
            cell
        });
        let id = ValueId(self.values.len());
        let call_mutations = Effect(self.initial.len());
        self.initial.push(false);
        let external_aliases = Effect(self.initial.len());
        self.initial.push(false);
        self.values.push(Value {
            cells,
            definition: None,
            alias: None,
            closure: None,
            call_mutations,
            external_aliases,
            retained_capture: false,
            binding_readers: Vec::new(),
        });
        if let Some(recording) = &mut self.recording {
            recording.instructions.push(Instruction::Value(id, hazards));
        }
        for cell in cells.into_iter().chain([call_mutations.0]) {
            self.equations.push(Equation {
                target: cell,
                expression: Expression::Read(self.poison.0),
            });
        }
        id
    }

    pub fn undefined(&self) -> ValueId {
        self.undefined
    }

    pub fn unknown(&self) -> ValueId {
        self.unknown
    }

    pub fn pure(&self) -> Effect {
        self.pure
    }

    pub fn effectful(&self) -> Effect {
        self.effectful
    }

    pub fn hazard(&self, value: ValueId, slot: Hazard) -> Effect {
        Effect(self.values[value.0].cells[slot as usize])
    }

    /// Execute an operation whose hooks have no mutation summary. Merely
    /// consulting a hazard with `hazard` does not execute the operation.
    pub fn operation(&mut self, value: ValueId, slot: Hazard) -> Effect {
        let effect = self.hazard(value, slot);
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Operation(value, slot));
            return effect;
        }
        if self.latent {
            return effect;
        }
        self.equations.push(Equation {
            target: self.poison.0,
            expression: Expression::Read(effect.0),
        });
        effect
    }

    pub fn any(&mut self, effects: impl IntoIterator<Item = Effect>) -> Effect {
        let mut cells: Vec<_> = effects.into_iter().map(|effect| effect.0).collect();
        cells.sort_unstable();
        cells.dedup();
        let result = match cells.as_slice() {
            [] => self.pure,
            [cell] => Effect(*cell),
            _ => self.expression(Expression::Or(
                cells.iter().copied().map(Expression::Read).collect(),
            )),
        };
        if let Some(recording) = &mut self.recording {
            recording.instructions.push(Instruction::Any(
                result,
                cells.into_iter().map(Effect).collect(),
            ));
        }
        result
    }

    pub fn update(&mut self, value: ValueId, slot: Hazard, effects: Effect) {
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Update(value, slot, effects));
            return;
        }
        let value = self.resolve_alias(value);
        self.equations.push(Equation {
            target: self.poison.0,
            expression: Expression::And(vec![
                Expression::Read(self.values[value.0].external_aliases.0),
                Expression::Read(effects.0),
            ]),
        });
        self.equations.push(Equation {
            target: self.values[value.0].cells[slot as usize],
            expression: Expression::Read(effects.0),
        });
        if slot == Hazard::Call {
            self.equations.push(Equation {
                target: self.values[value.0].call_mutations.0,
                expression: Expression::Read(effects.0),
            });
        }
    }

    /// A conservative may-alias join: later updates must reach every candidate.
    pub fn join_values(&mut self, values: &[ValueId]) -> ValueId {
        if let Some(value) = values.first() {
            if values.iter().all(|candidate| candidate == value) {
                return *value;
            }
        }
        let result = self.value(Hazards::NONE);
        if let Some(recording) = &mut self.recording {
            recording
                .instructions
                .push(Instruction::Join(result, values.to_vec()));
            return result;
        }
        for value in values {
            self.link_hazards(result, *value);
        }
        result
    }

    /// Unknown code can retain arguments, invoke closures, or alter prototypes.
    /// Until reachability is modeled, poison all value hazards, including values
    /// constructed after the unknown operation.
    pub fn opaque_effects(&mut self) -> Effect {
        if let Some(recording) = &mut self.recording {
            recording.instructions.push(Instruction::Opaque);
            return self.effectful;
        }
        if self.latent {
            return self.effectful;
        }
        self.opaque = true;
        self.effectful
    }

    pub fn call(
        &mut self,
        function: ValueId,
        arguments: &[ValueId],
        receiver: Option<ValueId>,
    ) -> CallResult {
        self.call_with_annotation(function, arguments, receiver, false)
    }

    pub(crate) fn call_with_annotation(
        &mut self,
        function: ValueId,
        arguments: &[ValueId],
        receiver: Option<ValueId>,
        pure: bool,
    ) -> CallResult {
        let receiver = receiver.unwrap_or(self.undefined);
        if self.recording.is_some() {
            let value = self.value(Hazards::NONE);
            let effects = self.expression(Expression::Constant(false));
            let result = CallResult::new(value, effects);
            self.recording
                .as_mut()
                .expect("recording")
                .instructions
                .push(Instruction::Call(
                    result,
                    function,
                    arguments.to_vec(),
                    receiver,
                    pure,
                ));
            return result;
        }
        if pure {
            let previous = self.latent;
            self.latent = true;
            let result = self.call(function, arguments, Some(receiver));
            self.latent = previous;
            return CallResult::new(result.value(), self.pure);
        }
        let resolved = self.resolve_alias(function);
        if !self.latent {
            self.equations.push(Equation {
                target: self.poison.0,
                expression: Expression::Read(self.values[resolved.0].call_mutations.0),
            });
        }
        if let Some(closure) = self.values[resolved.0].closure.clone() {
            return self.call_inferred(function, resolved, closure, arguments, receiver);
        }
        let function = resolved;
        let Some(index) = self.values[function.0].definition else {
            return CallResult::new(self.unknown, self.opaque_effects());
        };
        let definition = self.facts.definitions()[index].clone();
        if !definition.variadic() && arguments.len() > definition.parameters().len() {
            return CallResult::new(self.unknown, self.opaque_effects());
        }
        let effects = self.call_effects(function, arguments, receiver);
        let frame = Frame {
            definition: &definition,
            arguments,
            receiver,
            undefined: self.undefined,
        };
        if !self.latent {
            let invocation = Invocation {
                function,
                arguments: arguments.to_vec(),
                receiver,
            };
            if Self::growing_context(&self.executing, &invocation) {
                self.opaque_effects();
            } else if self.executing.insert(invocation.clone()) {
                self.execute_dependencies(&definition.hazards().get(Hazard::Call), &frame);
                self.executing.remove(&invocation);
            }
        }
        let value = match definition.returns() {
            Returns::Argument(name) => frame.value(name),
            Returns::Void => self.undefined,
            Returns::Unknown => self.unknown,
            Returns::Fresh => {
                let value = self.value(Hazards::NONE);
                for slot in Hazard::ALL {
                    let expression =
                        self.instantiate(&definition.return_hazards().get(slot), &frame);
                    self.equations.push(Equation {
                        target: self.values[value.0].cells[slot as usize],
                        expression,
                    });
                }
                value
            }
        };
        for update in definition.updates() {
            if self.latent {
                break;
            }
            let target = frame.value(&update.target());
            let expression = self.instantiate(&update.expression(), &frame);
            let effects = self.expression(expression);
            for slot in Hazard::ALL {
                if update.slot().is_none_or(|updated| updated == slot) {
                    self.update(target, slot, effects);
                }
            }
        }
        CallResult::new(value, effects)
    }

    fn expression(&mut self, expression: Expression) -> Effect {
        let target = self.initial.len();
        self.initial.push(false);
        self.equations.push(Equation { target, expression });
        Effect(target)
    }

    fn call_effects(
        &mut self,
        function: ValueId,
        arguments: &[ValueId],
        receiver: ValueId,
    ) -> Effect {
        let function = self.resolve_alias(function);
        if self.values[function.0].closure.is_some() {
            let previous = self.latent;
            self.latent = true;
            let result = self.call(function, arguments, Some(receiver));
            self.latent = previous;
            return result.effects();
        }
        let Some(index) = self.values[function.0].definition else {
            if !self.latent && function != self.undefined {
                self.opaque = true;
            }
            return self.hazard(function, Hazard::Call);
        };
        let definition = self.facts.definitions()[index].clone();
        // shortcut: hypothetical writes need an isolated heap to specialize.
        if self.latent && !definition.updates().is_empty() {
            return self.effectful;
        }
        if !definition.variadic() && arguments.len() > definition.parameters().len() {
            return self.effectful;
        }
        let invocation = Invocation {
            function,
            arguments: arguments.to_vec(),
            receiver,
        };
        if let Some(effect) = self.calls.get(&invocation) {
            return *effect;
        }
        // Argument forwarding can grow an unbounded family of recursive call
        // contexts. Fixed-size argument permutations are finite; widen growth.
        if Self::growing_context(&self.expanding, &invocation) {
            self.opaque |= !self.latent;
            return self.effectful;
        }
        self.expanding.insert(invocation.clone());
        // Install the cell before expansion so recursive summaries share a cycle.
        let effect = Effect(self.initial.len());
        self.initial.push(false);
        self.calls.insert(invocation.clone(), effect);
        let frame = Frame {
            definition: &definition,
            arguments,
            receiver,
            undefined: self.undefined,
        };
        let body = self.instantiate(&definition.hazards().get(Hazard::Call), &frame);
        self.expanding.remove(&invocation);
        self.equations.push(Equation {
            target: effect.0,
            expression: Expression::Or(vec![
                body,
                Expression::Read(self.values[function.0].call_mutations.0),
            ]),
        });
        effect
    }

    fn instantiate(&mut self, expression: &HazardExpr, frame: &Frame<'_>) -> Expression {
        match expression {
            HazardExpr::Bool(value) => Expression::Constant(*value),
            HazardExpr::Unknown => Expression::Constant(true),
            HazardExpr::Read { value, slot } => {
                Expression::Read(self.hazard(frame.value(value), *slot).0)
            }
            HazardExpr::Or(expressions) => Expression::Or(
                expressions
                    .iter()
                    .map(|e| self.instantiate(e, frame))
                    .collect(),
            ),
            HazardExpr::Call {
                function,
                arguments,
                receiver,
            } => {
                let values = frame.call_arguments(arguments);
                let receiver = receiver
                    .as_ref()
                    .map_or(self.undefined, |name| frame.value(name));
                let previous = self.latent;
                self.latent = true;
                let effects = self.call_effects(frame.value(function), &values, receiver);
                self.latent = previous;
                Expression::Read(effects.0)
            }
        }
    }

    fn execute_dependencies(&mut self, expression: &HazardExpr, frame: &Frame<'_>) {
        match expression {
            HazardExpr::Bool(_) => {}
            HazardExpr::Unknown => {
                self.opaque_effects();
            }
            HazardExpr::Read { value, slot } => {
                self.operation(frame.value(value), *slot);
            }
            HazardExpr::Or(expressions) => {
                for expression in expressions {
                    self.execute_dependencies(expression, frame);
                }
            }
            HazardExpr::Call {
                function,
                arguments,
                receiver,
            } => {
                let function = frame.value(function);
                let arguments = frame.call_arguments(arguments);
                let receiver = receiver
                    .as_ref()
                    .map_or(self.undefined, |name| frame.value(name));
                self.call(function, &arguments, Some(receiver));
            }
        }
    }

    fn growing_context(active: &HashSet<Invocation>, invocation: &Invocation) -> bool {
        active.iter().any(|parent| {
            parent.function == invocation.function
                && parent.arguments.len() < invocation.arguments.len()
        })
    }

    pub fn evaluate(&self) -> Evaluation {
        let mut state = self.initial.clone();
        if self.opaque {
            state[self.poison.0] = true;
        }
        let mut readers = vec![Vec::new(); state.len()];
        for (index, equation) in self.equations.iter().enumerate() {
            let mut dependencies = Vec::new();
            equation.expression.dependencies(&mut dependencies);
            dependencies.sort_unstable();
            dependencies.dedup();
            for cell in dependencies {
                readers[cell].push(index);
            }
        }
        let mut pending: Vec<_> = (0..self.equations.len()).collect();
        while !pending.is_empty() {
            let mut changed: Vec<_> = pending
                .par_iter()
                .filter_map(|index| {
                    let equation = &self.equations[*index];
                    (!state[equation.target] && equation.expression.evaluate(&state))
                        .then_some(equation.target)
                })
                .collect();
            changed.sort_unstable();
            changed.dedup();
            let mut next = Vec::new();
            for cell in changed {
                state[cell] = true;
                next.extend_from_slice(&readers[cell]);
            }
            next.sort_unstable();
            next.dedup();
            pending = next;
        }
        Evaluation {
            state,
            values: self.values.iter().map(|value| value.cells).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Evaluation {
    state: Vec<bool>,
    values: Vec<[usize; Hazard::ALL.len()]>,
}

impl Evaluation {
    pub fn effect(&self, effect: Effect) -> bool {
        self.state[effect.0]
    }

    pub fn hazards(&self, value: ValueId) -> Hazards {
        Hazards(self.values[value.0].map(|cell| self.state[cell]))
    }
}
