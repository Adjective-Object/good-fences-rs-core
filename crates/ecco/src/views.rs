//! Borrowed views of extraction records. Wire variants never materialize a tree.
//!
//! Use checked `WireEcco`/`WireRecord` owners. For another structurally verified
//! generated root, call `ParsedFileView::validate` before traversing its views.
use std::{fmt::Debug, sync::Arc};

use cache_flatbuffers::wire as w;
use flatbuffers::{Follow, ForwardsUOffset, Vector};
use oxc_span::Span;

use crate::analysis::ParsedValue;
use crate::solver::{FunctionSignature, Instruction};
use crate::*;

/// Element mapping for a slice or a FlatBuffers vector.
pub trait Element<'a>: Copy {
    type Owned: 'a;
    type Wire: Follow<'a> + 'a;
    fn owned(value: &'a Self::Owned) -> Self;
    fn wire(value: <Self::Wire as Follow<'a>>::Inner) -> Self;
}

pub struct Sequence<'a, T: Element<'a>> {
    storage: SequenceStorage<'a, T>,
}

enum SequenceStorage<'a, T: Element<'a>> {
    Owned(&'a [T::Owned]),
    Wire(Vector<'a, T::Wire>),
}

impl<'a, T: Element<'a>> Copy for Sequence<'a, T> {}
impl<'a, T: Element<'a>> Clone for Sequence<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, T: Element<'a>> Copy for SequenceStorage<'a, T> {}
impl<'a, T: Element<'a>> Clone for SequenceStorage<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, T: Element<'a> + Debug> Debug for Sequence<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl<'a, T: Element<'a>> Sequence<'a, T> {
    fn owned(values: &'a [T::Owned]) -> Self {
        Self {
            storage: SequenceStorage::Owned(values),
        }
    }
    fn wire(values: Vector<'a, T::Wire>) -> Self {
        Self {
            storage: SequenceStorage::Wire(values),
        }
    }
    pub fn len(self) -> usize {
        match self.storage {
            SequenceStorage::Owned(v) => v.len(),
            SequenceStorage::Wire(v) => v.len(),
        }
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub fn get(self, index: usize) -> Option<T> {
        if index >= self.len() {
            return None;
        }
        Some(match self.storage {
            SequenceStorage::Owned(v) => T::owned(&v[index]),
            SequenceStorage::Wire(v) => T::wire(v.get(index)),
        })
    }
    pub fn iter(self) -> SequenceIter<'a, T> {
        SequenceIter(match self.storage {
            SequenceStorage::Owned(v) => SequenceIteratorStorage::Owned(v.iter()),
            SequenceStorage::Wire(v) => SequenceIteratorStorage::Wire(v.iter()),
        })
    }
}
pub struct SequenceIter<'a, T: Element<'a>>(SequenceIteratorStorage<'a, T>);
enum SequenceIteratorStorage<'a, T: Element<'a>> {
    Owned(std::slice::Iter<'a, T::Owned>),
    Wire(flatbuffers::VectorIter<'a, T::Wire>),
}
impl<'a, T: Element<'a>> Iterator for SequenceIter<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        match &mut self.0 {
            SequenceIteratorStorage::Owned(v) => v.next().map(T::owned),
            SequenceIteratorStorage::Wire(v) => v.next().map(T::wire),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            SequenceIteratorStorage::Owned(v) => v.size_hint(),
            SequenceIteratorStorage::Wire(v) => v.size_hint(),
        }
    }
}
impl<'a, T: Element<'a>> DoubleEndedIterator for SequenceIter<'a, T> {
    fn next_back(&mut self) -> Option<T> {
        match &mut self.0 {
            SequenceIteratorStorage::Owned(v) => v.next_back().map(T::owned),
            SequenceIteratorStorage::Wire(v) => v.next_back().map(T::wire),
        }
    }
}
impl<'a, T: Element<'a>> ExactSizeIterator for SequenceIter<'a, T> {}
impl<'a, T: Element<'a>> IntoIterator for Sequence<'a, T> {
    type Item = T;
    type IntoIter = SequenceIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

macro_rules! view {
    ($view:ident, $owned:ty, $wire:ident) => {
        #[derive(Clone, Copy, Debug)]
        pub enum $view<'a> {
            Owned(&'a $owned),
            Wire(w::$wire<'a>),
        }
        impl<'a> From<&'a $owned> for $view<'a> {
            fn from(value: &'a $owned) -> Self {
                Self::Owned(value)
            }
        }
        impl<'a> From<w::$wire<'a>> for $view<'a> {
            fn from(value: w::$wire<'a>) -> Self {
                Self::Wire(value)
            }
        }
        impl<'a> Element<'a> for $view<'a> {
            type Owned = $owned;
            type Wire = ForwardsUOffset<w::$wire<'a>>;
            fn owned(value: &'a $owned) -> Self {
                value.into()
            }
            fn wire(value: w::$wire<'a>) -> Self {
                value.into()
            }
        }
    };
}
macro_rules! scalar {
    ($name:ident, $ty:ty) => {
        pub fn $name(self) -> $ty {
            match self {
                Self::Owned(v) => v.$name(),
                Self::Wire(v) => v.$name(),
            }
        }
    };
    ($name:ident, $ty:ty, $map:expr) => {
        pub fn $name(self) -> $ty {
            match self {
                Self::Owned(v) => v.$name(),
                Self::Wire(v) => ($map)(v.$name()),
            }
        }
    };
}
macro_rules! nested {
    ($name:ident, $ty:ident) => {
        pub fn $name(self) -> $ty<'a> {
            match self {
                Self::Owned(v) => v.$name().into(),
                Self::Wire(v) => v.$name().into(),
            }
        }
    };
}
macro_rules! sequence {
    ($name:ident, $ty:ty) => {
        pub fn $name(self) -> Sequence<'a, $ty> {
            match self {
                Self::Owned(v) => Sequence::owned(v.$name()),
                Self::Wire(v) => Sequence::wire(v.$name()),
            }
        }
    };
}

impl<'a> Element<'a> for &'a str {
    type Owned = String;
    type Wire = ForwardsUOffset<&'a str>;
    fn owned(value: &'a String) -> Self {
        value
    }
    fn wire(value: &'a str) -> Self {
        value
    }
}
macro_rules! id_element {
    ($id:ident) => {
        impl<'a> Element<'a> for $id {
            type Owned = $id;
            type Wire = u64;
            fn owned(value: &'a Self) -> Self {
                *value
            }
            fn wire(value: u64) -> Self {
                Self(id(value))
            }
        }
    };
}
id_element!(ValueId);
id_element!(Effect);

fn id(value: u64) -> usize {
    usize::try_from(value).expect("Ecco wire ID exceeds platform width")
}
fn span(value: &w::Span) -> Span {
    Span::new(value.start(), value.end())
}
fn hazard(value: w::Hazard) -> Hazard {
    Hazard::ALL[usize::from(value.0)]
}
fn hazards(value: &w::Hazards) -> Hazards {
    Hazards([
        value.property_read(),
        value.to_primitive(),
        value.membership(),
        value.instance_of(),
        value.call(),
        value.construct(),
        value.iterate(),
        value.prototype_read(),
    ])
}

view!(ParsedFileView, ParsedFile, ParsedFile);
view!(ParsedSymbolView, ParsedSymbol, ParsedSymbol);
view!(ParsedRootView, ParsedRoot, ParsedRoot);
view!(ExternalReferenceView, ExternalReference, ExternalReference);
view!(DefinitionMatchView, DefinitionMatch, DefinitionMatch);
view!(ParsedValueView, ParsedValue, ParsedValue);
view!(FunctionView, OwnedFunction, OwnedFunction);
view!(FunctionSignatureView, FunctionSignature, FunctionSignature);
view!(LocalFactsView, LocalFacts, LocalFacts);
view!(DefinitionView, Definition, Definition);
view!(GlobalSymbolView, GlobalSymbol, GlobalSymbol);
view!(UpdateView, Update, Update);
view!(HazardFieldsView, HazardFields, HazardFields);

impl<'a> ParsedFileView<'a> {
    scalar!(version, u32);
    sequence!(values, ParsedValueView<'a>);
    sequence!(symbols, ParsedSymbolView<'a>);
    sequence!(functions, FunctionView<'a>);
    sequence!(roots, ParsedRootView<'a>);
    nested!(setup, FunctionView);
    pub fn declarations(self) -> LocalFactsView<'a> {
        match self {
            Self::Owned(v) => v.declarations().into(),
            Self::Wire(v) => v.facts().into(),
        }
    }
    pub fn external_references(self) -> Sequence<'a, ExternalReferenceView<'a>> {
        match self {
            Self::Owned(v) => Sequence::owned(v.external_references()),
            Self::Wire(v) => Sequence::wire(v.externals()),
        }
    }
    pub fn definition_matches(self) -> Sequence<'a, DefinitionMatchView<'a>> {
        match self {
            Self::Owned(v) => Sequence::owned(v.definition_matches()),
            Self::Wire(v) => Sequence::wire(v.matches()),
        }
    }
}
impl<'a> ParsedSymbolView<'a> {
    scalar!(id, SymbolId, SymbolId);
    scalar!(name, &'a str);
    scalar!(span, Span, span);
    scalar!(value, ValueId, |v| ValueId(id(v)));
}
impl<'a> ParsedValueView<'a> {
    scalar!(hazards, Hazards, hazards);
    scalar!(definition, Option<usize>, |v: Option<u64>| v.map(id));
    scalar!(alias, Option<ValueId>, |v: Option<u64>| v
        .map(|v| ValueId(id(v))));
}
impl<'a> ParsedRootView<'a> {
    scalar!(span, Span, span);
    nested!(function, FunctionView);
}
impl<'a> ExternalReferenceView<'a> {
    scalar!(span, Span, span);
    nested!(binding, BindingView);
}
impl<'a> DefinitionMatchView<'a> {
    scalar!(span, Span, span);
    nested!(binding, BindingView);
    pub fn to_owned(self) -> DefinitionMatch {
        DefinitionMatch::new(self.span(), self.binding().to_owned())
    }
}
impl<'a> LocalFactsView<'a> {
    sequence!(definitions, DefinitionView<'a>);
    sequence!(global_symbols, GlobalSymbolView<'a>);
}
impl<'a> GlobalSymbolView<'a> {
    scalar!(name, &'a str);
    scalar!(read_effects, bool);
    pub fn to_owned(self) -> GlobalSymbol {
        GlobalSymbol::new(self.name().into(), self.read_effects())
    }
}
impl<'a> DefinitionView<'a> {
    nested!(binding, BindingView);
    sequence!(parameters, &'a str);
    scalar!(rest, Option<&'a str>);
    scalar!(variadic, bool);
    nested!(hazards, HazardFieldsView);
    nested!(return_hazards, HazardFieldsView);
    nested!(returns, ReturnsView);
    sequence!(updates, UpdateView<'a>);
    /// Explicit construction of a mutable interpreter declaration.
    pub fn to_owned(self) -> Definition {
        Definition::new(
            self.binding().to_owned(),
            self.parameters().iter().map(str::to_owned).collect(),
            self.rest().map(str::to_owned),
            self.variadic(),
            self.hazards().to_owned(),
            self.returns().to_owned(),
            self.return_hazards().to_owned(),
            self.updates().iter().map(UpdateView::to_owned).collect(),
        )
    }
}
impl<'a> UpdateView<'a> {
    scalar!(target, &'a str);
    scalar!(slot, Option<Hazard>, |v: Option<w::Hazard>| v.map(hazard));
    nested!(expression, HazardExprView);
    pub fn to_owned(self) -> Update {
        Update::new(
            self.target().into(),
            self.slot(),
            self.expression().to_owned(),
        )
    }
}
impl<'a> FunctionSignatureView<'a> {
    sequence!(captures, ValueId);
    sequence!(parameters, ValueId);
    scalar!(receiver, Option<ValueId>, |v: Option<u64>| v
        .map(|v| ValueId(id(v))));
    scalar!(self_binding, Option<ValueId>, |v: Option<u64>| v
        .map(|v| ValueId(id(v))));
    scalar!(constructible, bool);
    scalar!(arguments, Option<ValueId>, |v: Option<u64>| v
        .map(|v| ValueId(id(v))));
}
impl<'a> FunctionView<'a> {
    nested!(signature, FunctionSignatureView);
    sequence!(instructions, InstructionView<'a>);
    sequence!(returns, ValueId);
    scalar!(effects, Option<Effect>, |v: Option<u64>| v
        .map(|v| Effect(id(v))));
    pub fn sources(self) -> EffectSources<'a> {
        EffectSources(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EffectSources<'a>(FunctionView<'a>);
impl<'a> EffectSources<'a> {
    pub fn get(self, effect: &Effect) -> Option<(ValueId, Hazard)> {
        match self.0 {
            FunctionView::Owned(v) => v.sources().get(effect).copied(),
            FunctionView::Wire(v) => v
                .sources()
                .lookup_by_key(effect.0 as u64, |v, key| v.key_compare_with_value(*key))
                .map(|v| (ValueId(id(v.value())), hazard(v.hazard()))),
        }
    }
    pub fn iter(self) -> impl Iterator<Item = (Effect, (ValueId, Hazard))> + 'a {
        let (owned, wire) = match self.0 {
            FunctionView::Owned(v) => (Some(v.sources().iter()), None),
            FunctionView::Wire(v) => (None, Some(v.sources().iter())),
        };
        owned
            .into_iter()
            .flatten()
            .map(|(k, v)| (*k, *v))
            .chain(wire.into_iter().flatten().map(|v| {
                (
                    Effect(id(v.effect())),
                    (ValueId(id(v.value())), hazard(v.hazard())),
                )
            }))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BindingView<'a> {
    Global(Sequence<'a, &'a str>),
    Module {
        module: &'a str,
        path: Sequence<'a, &'a str>,
    },
    Local(&'a str),
}
impl<'a> From<&'a Binding> for BindingView<'a> {
    fn from(v: &'a Binding) -> Self {
        match v {
            Binding::Global(path) => Self::Global(Sequence::owned(path)),
            Binding::Module { module, path } => Self::Module {
                module,
                path: Sequence::owned(path),
            },
            Binding::Local(name) => Self::Local(name),
        }
    }
}
impl<'a> From<w::EccoBinding<'a>> for BindingView<'a> {
    fn from(v: w::EccoBinding<'a>) -> Self {
        match v.value_type() {
            w::EccoBindingValue::GlobalBinding => {
                Self::Global(Sequence::wire(v.value_as_global_binding().unwrap().path()))
            }
            w::EccoBindingValue::ModuleBinding => {
                let v = v.value_as_module_binding().unwrap();
                Self::Module {
                    module: v.module(),
                    path: Sequence::wire(v.path()),
                }
            }
            w::EccoBindingValue::LocalBinding => {
                Self::Local(v.value_as_local_binding().unwrap().name())
            }
            _ => unreachable!("verified binding discriminant"),
        }
    }
}
impl BindingView<'_> {
    pub fn to_owned(self) -> Binding {
        match self {
            Self::Global(path) => Binding::Global(path.iter().map(str::to_owned).collect()),
            Self::Module { module, path } => Binding::Module {
                module: module.into(),
                path: path.iter().map(str::to_owned).collect(),
            },
            Self::Local(name) => Binding::Local(name.into()),
        }
    }
}
impl<'a, 'b> PartialEq<BindingView<'b>> for BindingView<'a> {
    fn eq(&self, other: &BindingView<'b>) -> bool {
        match (*self, *other) {
            (BindingView::Global(a), BindingView::Global(b)) => a.iter().eq(b),
            (
                BindingView::Module {
                    module: a,
                    path: ap,
                },
                BindingView::Module {
                    module: b,
                    path: bp,
                },
            ) => a == b && ap.iter().eq(bp),
            (BindingView::Local(a), BindingView::Local(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for BindingView<'_> {}
impl std::hash::Hash for BindingView<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match *self {
            Self::Local(name) => name.hash(state),
            Self::Global(path) => {
                path.len().hash(state);
                for name in path {
                    name.hash(state);
                }
            }
            Self::Module { module, path } => {
                module.hash(state);
                path.len().hash(state);
                for name in path {
                    name.hash(state);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ReturnsView<'a> {
    Fresh,
    Argument(&'a str),
    Void,
    Unknown,
}
impl<'a> From<&'a Returns> for ReturnsView<'a> {
    fn from(v: &'a Returns) -> Self {
        match v {
            Returns::Fresh => Self::Fresh,
            Returns::Argument(v) => Self::Argument(v),
            Returns::Void => Self::Void,
            Returns::Unknown => Self::Unknown,
        }
    }
}
impl<'a> From<w::Returns<'a>> for ReturnsView<'a> {
    fn from(v: w::Returns<'a>) -> Self {
        match v.value_type() {
            w::ReturnsValue::FreshReturn => Self::Fresh,
            w::ReturnsValue::ArgumentReturn => {
                Self::Argument(v.value_as_argument_return().unwrap().name())
            }
            w::ReturnsValue::VoidReturn => Self::Void,
            w::ReturnsValue::UnknownReturn => Self::Unknown,
            _ => unreachable!("verified return discriminant"),
        }
    }
}
impl ReturnsView<'_> {
    pub fn to_owned(self) -> Returns {
        match self {
            Self::Fresh => Returns::Fresh,
            Self::Argument(v) => Returns::Argument(v.into()),
            Self::Void => Returns::Void,
            Self::Unknown => Returns::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CallArgumentView<'a> {
    Value(&'a str),
    Spread(&'a str),
    Forward,
}
impl<'a> Element<'a> for CallArgumentView<'a> {
    type Owned = CallArgument;
    type Wire = ForwardsUOffset<w::CallArgument<'a>>;
    fn owned(v: &'a CallArgument) -> Self {
        match v {
            CallArgument::Value(v) => Self::Value(v),
            CallArgument::Spread(v) => Self::Spread(v),
            CallArgument::Forward => Self::Forward,
        }
    }
    fn wire(v: w::CallArgument<'a>) -> Self {
        match v.value_type() {
            w::CallArgumentValue::ValueArgument => {
                Self::Value(v.value_as_value_argument().unwrap().name())
            }
            w::CallArgumentValue::SpreadArgument => {
                Self::Spread(v.value_as_spread_argument().unwrap().name())
            }
            w::CallArgumentValue::ForwardArgument => Self::Forward,
            _ => unreachable!("verified argument discriminant"),
        }
    }
}
impl CallArgumentView<'_> {
    pub fn to_owned(self) -> CallArgument {
        match self {
            Self::Value(v) => CallArgument::Value(v.into()),
            Self::Spread(v) => CallArgument::Spread(v.into()),
            Self::Forward => CallArgument::Forward,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum HazardExprView<'a> {
    Bool(bool),
    Unknown,
    Read {
        value: &'a str,
        slot: Hazard,
    },
    Or(Sequence<'a, Self>),
    Call {
        function: &'a str,
        arguments: Sequence<'a, CallArgumentView<'a>>,
        receiver: Option<&'a str>,
    },
}
impl<'a> Element<'a> for HazardExprView<'a> {
    type Owned = HazardExpr;
    type Wire = ForwardsUOffset<w::HazardExpr<'a>>;
    fn owned(v: &'a HazardExpr) -> Self {
        v.into()
    }
    fn wire(v: w::HazardExpr<'a>) -> Self {
        v.into()
    }
}
impl<'a> From<&'a HazardExpr> for HazardExprView<'a> {
    fn from(v: &'a HazardExpr) -> Self {
        match v {
            HazardExpr::Bool(v) => Self::Bool(*v),
            HazardExpr::Unknown => Self::Unknown,
            HazardExpr::Read { value, slot } => Self::Read { value, slot: *slot },
            HazardExpr::Or(v) => Self::Or(Sequence::owned(v)),
            HazardExpr::Call {
                function,
                arguments,
                receiver,
            } => Self::Call {
                function,
                arguments: Sequence::owned(arguments),
                receiver: receiver.as_deref(),
            },
        }
    }
}
impl<'a> From<w::HazardExpr<'a>> for HazardExprView<'a> {
    fn from(v: w::HazardExpr<'a>) -> Self {
        match v.value_type() {
            w::HazardExprValue::BooleanHazard => {
                Self::Bool(v.value_as_boolean_hazard().unwrap().value())
            }
            w::HazardExprValue::UnknownHazard => Self::Unknown,
            w::HazardExprValue::ReadHazard => {
                let v = v.value_as_read_hazard().unwrap();
                Self::Read {
                    value: v.value(),
                    slot: hazard(v.slot()),
                }
            }
            w::HazardExprValue::OrHazard => Self::Or(Sequence::wire(
                v.value_as_or_hazard().unwrap().expressions(),
            )),
            w::HazardExprValue::CallHazard => {
                let v = v.value_as_call_hazard().unwrap();
                Self::Call {
                    function: v.function(),
                    arguments: Sequence::wire(v.arguments()),
                    receiver: v.receiver(),
                }
            }
            _ => unreachable!("verified expression discriminant"),
        }
    }
}
impl HazardExprView<'_> {
    pub fn to_owned(self) -> HazardExpr {
        match self {
            Self::Bool(v) => HazardExpr::Bool(v),
            Self::Unknown => HazardExpr::Unknown,
            Self::Read { value, slot } => HazardExpr::Read {
                value: value.into(),
                slot,
            },
            Self::Or(v) => HazardExpr::Or(v.iter().map(Self::to_owned).collect()),
            Self::Call {
                function,
                arguments,
                receiver,
            } => HazardExpr::Call {
                function: function.into(),
                arguments: arguments.iter().map(CallArgumentView::to_owned).collect(),
                receiver: receiver.map(str::to_owned),
            },
        }
    }
}
impl<'a> Element<'a> for (Hazard, HazardExprView<'a>) {
    type Owned = (Hazard, HazardExpr);
    type Wire = ForwardsUOffset<w::HazardField<'a>>;
    fn owned(v: &'a Self::Owned) -> Self {
        (v.0, (&v.1).into())
    }
    fn wire(v: w::HazardField<'a>) -> Self {
        (hazard(v.slot()), v.expression().into())
    }
}
impl<'a> HazardFieldsView<'a> {
    pub fn default_expression(self) -> Option<HazardExprView<'a>> {
        match self {
            Self::Owned(v) => v.default_expression().map(Into::into),
            Self::Wire(v) => v.default_expression().map(Into::into),
        }
    }
    sequence!(fields, (Hazard, HazardExprView<'a>));
    pub fn get(self, slot: Hazard) -> HazardExprView<'a> {
        self.fields()
            .iter()
            .find(|(field, _)| *field == slot)
            .map(|(_, v)| v)
            .or_else(|| self.default_expression())
            .unwrap_or(HazardExprView::Bool(true))
    }
    pub fn to_owned(self) -> HazardFields {
        HazardFields::new(
            self.default_expression().map(HazardExprView::to_owned),
            self.fields()
                .iter()
                .map(|(k, v)| (k, v.to_owned()))
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum InstructionView<'a> {
    Value(ValueId, Hazards),
    Any(Effect, Sequence<'a, Effect>),
    Equality(Effect, ValueId, ValueId),
    Operation(ValueId, Hazard),
    Update(ValueId, Hazard, Effect),
    Read(ValueId, ValueId),
    Retain(ValueId, Sequence<'a, ValueId>),
    Bind(ValueId, ValueId),
    Join(ValueId, Sequence<'a, ValueId>),
    Call(CallResult, ValueId, Sequence<'a, ValueId>, ValueId, bool),
    Closure(ValueId, usize),
    External(ValueId, usize),
    Opaque,
}
impl<'a> Element<'a> for InstructionView<'a> {
    type Owned = Instruction;
    type Wire = ForwardsUOffset<w::Instruction<'a>>;
    fn owned(v: &'a Instruction) -> Self {
        match v {
            Instruction::Value(a, b) => Self::Value(*a, *b),
            Instruction::Any(a, b) => Self::Any(*a, Sequence::owned(b)),
            Instruction::Equality(a, b, c) => Self::Equality(*a, *b, *c),
            Instruction::Operation(a, b) => Self::Operation(*a, *b),
            Instruction::Update(a, b, c) => Self::Update(*a, *b, *c),
            Instruction::Read(a, b) => Self::Read(*a, *b),
            Instruction::Retain(a, b) => Self::Retain(*a, Sequence::owned(b)),
            Instruction::Bind(a, b) => Self::Bind(*a, *b),
            Instruction::Join(a, b) => Self::Join(*a, Sequence::owned(b)),
            Instruction::Call(a, b, c, d, e) => Self::Call(*a, *b, Sequence::owned(c), *d, *e),
            Instruction::Closure(a, b) => Self::Closure(*a, *b),
            Instruction::External(a, b) => Self::External(*a, *b),
            Instruction::Opaque => Self::Opaque,
        }
    }
    fn wire(v: w::Instruction<'a>) -> Self {
        let value = |v| ValueId(id(v));
        let effect = |v| Effect(id(v));
        match v.value_type() {
            w::InstructionValue::ValueInstruction => {
                let v = v.value_as_value_instruction().unwrap();
                Self::Value(value(v.value()), hazards(v.hazards()))
            }
            w::InstructionValue::AnyInstruction => {
                let v = v.value_as_any_instruction().unwrap();
                Self::Any(effect(v.effect()), Sequence::wire(v.inputs()))
            }
            w::InstructionValue::EqualityInstruction => {
                let v = v.value_as_equality_instruction().unwrap();
                Self::Equality(effect(v.effect()), value(v.left()), value(v.right()))
            }
            w::InstructionValue::OperationInstruction => {
                let v = v.value_as_operation_instruction().unwrap();
                Self::Operation(value(v.value()), hazard(v.hazard()))
            }
            w::InstructionValue::UpdateInstruction => {
                let v = v.value_as_update_instruction().unwrap();
                Self::Update(value(v.value()), hazard(v.hazard()), effect(v.effect()))
            }
            w::InstructionValue::ReadInstruction => {
                let v = v.value_as_read_instruction().unwrap();
                Self::Read(value(v.value()), value(v.source()))
            }
            w::InstructionValue::RetainInstruction => {
                let v = v.value_as_retain_instruction().unwrap();
                Self::Retain(value(v.value()), Sequence::wire(v.locals()))
            }
            w::InstructionValue::BindInstruction => {
                let v = v.value_as_bind_instruction().unwrap();
                Self::Bind(value(v.target()), value(v.value()))
            }
            w::InstructionValue::JoinInstruction => {
                let v = v.value_as_join_instruction().unwrap();
                Self::Join(value(v.value()), Sequence::wire(v.inputs()))
            }
            w::InstructionValue::CallInstruction => {
                let v = v.value_as_call_instruction().unwrap();
                Self::Call(
                    CallResult::new(value(v.result().value()), effect(v.result().effects())),
                    value(v.function()),
                    Sequence::wire(v.arguments()),
                    value(v.receiver()),
                    v.pure_(),
                )
            }
            w::InstructionValue::ClosureInstruction => {
                let v = v.value_as_closure_instruction().unwrap();
                Self::Closure(value(v.value()), id(v.function()))
            }
            w::InstructionValue::ExternalInstruction => {
                let v = v.value_as_external_instruction().unwrap();
                Self::External(value(v.value()), id(v.reference()))
            }
            w::InstructionValue::OpaqueInstruction => Self::Opaque,
            _ => unreachable!("verified instruction discriminant"),
        }
    }
}

/// An immutable owner (including a future AST module owner) supplies borrowed
/// precursors. Analysis retains this owner, not decoded function summaries.
/// Implementations must return the same immutable file throughout their lifetime.
pub trait ParsedFileSource: Debug + Send + Sync {
    fn view(&self) -> ParsedFileView<'_>;
}
impl ParsedFileSource for ParsedFile {
    fn view(&self) -> ParsedFileView<'_> {
        self.into()
    }
}
impl ParsedFileSource for cache_flatbuffers::WireEcco {
    fn view(&self) -> ParsedFileView<'_> {
        self.root().into()
    }
}

/// Ecco nested in a checked module; the enclosing immutable allocation stays
/// alive through analysis and any subsequently specialized runtime calls.
#[derive(Clone, Debug)]
pub struct ModuleEcco {
    record: Arc<cache_flatbuffers::WireRecord>,
}
impl ModuleEcco {
    pub fn new(record: Arc<cache_flatbuffers::WireRecord>) -> eyre::Result<Self> {
        eyre::ensure!(
            record.module().facts().ecco().is_some(),
            "module has no Ecco precursors"
        );
        Ok(Self { record })
    }
}
impl ParsedFileSource for ModuleEcco {
    fn view(&self) -> ParsedFileView<'_> {
        self.record
            .module()
            .facts()
            .ecco()
            .expect("checked ESM module")
            .into()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum FunctionIndex {
    Setup,
    Function(usize),
    Root(usize),
}
#[derive(Clone, Debug)]
pub(crate) enum FunctionSource {
    Owned(Arc<OwnedFunction>),
    Parsed {
        file: Arc<dyn ParsedFileSource>,
        index: FunctionIndex,
    },
}
impl Default for FunctionSource {
    fn default() -> Self {
        Self::Owned(Arc::default())
    }
}
impl FunctionSource {
    pub(crate) fn view(&self) -> FunctionView<'_> {
        match self {
            Self::Owned(v) => v.as_ref().into(),
            Self::Parsed { file, index } => {
                let file = file.view();
                match *index {
                    FunctionIndex::Setup => file.setup(),
                    FunctionIndex::Function(i) => {
                        file.functions().get(i).expect("validated function")
                    }
                    FunctionIndex::Root(i) => {
                        file.roots().get(i).expect("validated root").function()
                    }
                }
            }
        }
    }
    pub(crate) fn extraction_owned(&self) -> OwnedFunction {
        match self {
            Self::Owned(v) => v.as_ref().clone(),
            Self::Parsed { .. } => panic!("extraction cannot consume installed file templates"),
        }
    }
}
