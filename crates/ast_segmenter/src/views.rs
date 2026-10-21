//! Copy views over immutable extraction records or verified FlatBuffers tables.
//! Collections yield values, never references into temporary adapter objects.
use cache_flatbuffers::wire as w;
use flatbuffers::{Follow, ForwardsUOffset, Vector};
use oxc_span::Span;

use crate::{effects as ef, emission as em, execution as ex, invocation as inv, owned as o};

/// The owner travels with graph inputs; borrowed tables are created on demand.
#[derive(Debug, Clone)]
pub enum FactsStorage {
    Owned(std::sync::Arc<o::ModuleFacts>),
    Wire(cache_flatbuffers::WireRecord),
}
impl From<o::ModuleFacts> for FactsStorage {
    fn from(value: o::ModuleFacts) -> Self {
        Self::Owned(std::sync::Arc::new(value))
    }
}
impl FactsStorage {
    pub fn view(&self) -> ModuleFacts<'_> {
        match self {
            Self::Owned(v) => ModuleFacts::Owned(v),
            Self::Wire(v) => ModuleFacts::Wire(v.module().facts()),
        }
    }
}
macro_rules! storage_accessors {
        ($($name:ident => $ty:ty),* $(,)?) => {
            impl FactsStorage { $(pub fn $name(&self) -> $ty { self.view().$name() })* }
        };
    }
storage_accessors! {
    ecco => Option<ecco::views::ParsedFileView<'_>>,
    ecco_declarations_hash => crate::DeclarationFingerprint,
    commonjs => bool,
    top_level_await => Sequence<'_, Span>,
    effects => Sequence<'_, EffectSite<'_>>,
    invocations => Sequence<'_, Invocation<'_>>,
    emission => EmissionFacts<'_>,
    bindings => Sequence<'_, Binding<'_>>,
    references => Sequence<'_, Reference<'_>>,
    dependencies => Sequence<'_, Dependency<'_>>,
    imports => Sequence<'_, ImportBinding<'_>>,
    exports => Sequence<'_, Export<'_>>,
    directives => Sequence<'_, Directive<'_>>,
    hashbang => Option<Directive<'_>>,
}
impl FactsStorage {
    pub fn owned(&self) -> eyre::Result<&o::ModuleFacts> {
        match self {
            Self::Owned(v) => Ok(v),
            Self::Wire(_) => eyre::bail!("byte-backed facts are not owned extraction records"),
        }
    }
    pub fn into_owned(self) -> eyre::Result<o::ModuleFacts> {
        match self {
            Self::Owned(v) => Ok(std::sync::Arc::unwrap_or_clone(v)),
            Self::Wire(_) => eyre::bail!("byte-backed facts are not owned extraction records"),
        }
    }
    pub fn ecco_source(
        &self,
    ) -> eyre::Result<Option<std::sync::Arc<dyn ecco::views::ParsedFileSource>>> {
        if self.view().ecco().is_none() {
            return Ok(None);
        }
        Ok(Some(match self {
            Self::Owned(v) => std::sync::Arc::new(OwnedEcco(v.clone())),
            Self::Wire(v) => std::sync::Arc::new(ecco::views::ModuleEcco::new(
                std::sync::Arc::new(v.clone()),
            )?),
        }))
    }
}

#[derive(Debug)]
struct OwnedEcco(std::sync::Arc<o::ModuleFacts>);
impl ecco::views::ParsedFileSource for OwnedEcco {
    fn view(&self) -> ecco::views::ParsedFileView<'_> {
        self.0.ecco().expect("checked ESM module").into()
    }
}

pub fn module_body(v: w::CachedModule<'_>) -> crate::ModuleBody {
    match v.body_type() {
        w::ModuleBody::EsmBody => crate::ModuleBody::Esm,
        w::ModuleBody::CommonJsBody => crate::ModuleBody::CommonJs {
            span: span(v.body_as_common_js_body().unwrap().span()),
        },
        _ => unreachable!("verified module body"),
    }
}

pub trait Element<'a>: Copy + 'a {
    type Owned: 'a;
    type Wire: Follow<'a> + 'a;
    fn owned(value: &'a Self::Owned) -> Self;
    fn wire(value: <Self::Wire as Follow<'a>>::Inner) -> Self;
}

/// Original ownership range. Legacy extraction tags are deliberately not a wire field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRange {
    pub span: Span,
}
impl<'a> Element<'a> for SegmentRange {
    type Owned = crate::Segment;
    type Wire = w::Span;
    fn owned(value: &'a crate::Segment) -> Self {
        Self { span: value.span }
    }
    fn wire(value: &'a w::Span) -> Self {
        Self { span: span(value) }
    }
}

pub enum Sequence<'a, T: Element<'a>> {
    Owned(&'a [T::Owned]),
    Wire(Vector<'a, T::Wire>),
    WireRange(Vector<'a, T::Wire>, usize, usize),
}
impl<'a, T: Element<'a>> Copy for Sequence<'a, T> {}
impl<'a, T: Element<'a>> Clone for Sequence<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, T: Element<'a> + std::fmt::Debug> std::fmt::Debug for Sequence<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl<'a, T: Element<'a> + PartialEq> PartialEq for Sequence<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
impl<'a, T: Element<'a> + Eq> Eq for Sequence<'a, T> {}
impl<'a, T: Element<'a> + PartialEq> PartialEq<Vec<T>> for Sequence<'a, T> {
    fn eq(&self, other: &Vec<T>) -> bool {
        self.iter().eq(other.iter().copied())
    }
}
impl<'a, T: Element<'a> + PartialEq, const N: usize> PartialEq<[T; N]> for Sequence<'a, T> {
    fn eq(&self, other: &[T; N]) -> bool {
        self.iter().eq(other.iter().copied())
    }
}
impl<'a, T: Element<'a>> Sequence<'a, T> {
    pub fn len(self) -> usize {
        match self {
            Self::Owned(v) => v.len(),
            Self::Wire(v) => v.len(),
            Self::WireRange(_, start, end) => end - start,
        }
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub fn get(self, index: usize) -> Option<T> {
        if index >= self.len() {
            return None;
        }
        Some(match self {
            Self::Owned(v) => T::owned(&v[index]),
            Self::Wire(v) => T::wire(v.get(index)),
            Self::WireRange(v, start, _) => T::wire(v.get(start + index)),
        })
    }
    pub fn first(self) -> Option<T> {
        self.get(0)
    }
    pub fn last(self) -> Option<T> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }
    pub fn iter(self) -> impl DoubleEndedIterator<Item = T> + ExactSizeIterator + Clone + 'a {
        (0..self.len()).map(move |i| self.get(i).unwrap())
    }
    pub fn contains(self, value: &T) -> bool
    where
        T: PartialEq,
    {
        self.iter().any(|v| v == *value)
    }
    pub fn partition_point(self, mut predicate: impl FnMut(&T) -> bool) -> usize {
        let (mut left, mut right) = (0, self.len());
        while left < right {
            let middle = left + (right - left) / 2;
            if predicate(&self.get(middle).unwrap()) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }
    pub fn slice(self, range: impl std::ops::RangeBounds<usize>) -> Self {
        use std::ops::Bound;
        let start = match range.start_bound() {
            Bound::Included(v) => *v,
            Bound::Excluded(v) => v.checked_add(1).expect("range start overflow"),
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(v) => v.checked_add(1).expect("range end overflow"),
            Bound::Excluded(v) => *v,
            Bound::Unbounded => self.len(),
        };
        assert!(start <= end && end <= self.len());
        match self {
            Self::Owned(v) => Self::Owned(&v[start..end]),
            Self::Wire(v) => Self::WireRange(v, start, end),
            Self::WireRange(v, offset, _) => Self::WireRange(v, offset + start, offset + end),
        }
    }
}
pub struct SequenceIter<'a, T: Element<'a>> {
    sequence: Sequence<'a, T>,
    range: std::ops::Range<usize>,
}
impl<'a, T: Element<'a>> Iterator for SequenceIter<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.range.next().and_then(|i| self.sequence.get(i))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}
impl<'a, T: Element<'a>> DoubleEndedIterator for SequenceIter<'a, T> {
    fn next_back(&mut self) -> Option<T> {
        self.range.next_back().and_then(|i| self.sequence.get(i))
    }
}
impl<'a, T: Element<'a>> ExactSizeIterator for SequenceIter<'a, T> {}
impl<'a, T: Element<'a>> IntoIterator for Sequence<'a, T> {
    type Item = T;
    type IntoIter = SequenceIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        SequenceIter {
            sequence: self,
            range: 0..self.len(),
        }
    }
}
impl<'a> Element<'a> for Span {
    type Owned = Span;
    type Wire = w::Span;
    fn owned(v: &'a Span) -> Self {
        *v
    }
    fn wire(v: &'a w::Span) -> Self {
        span(v)
    }
}
impl<'a> Element<'a> for u32 {
    type Owned = u32;
    type Wire = u32;
    fn owned(v: &'a u32) -> Self {
        *v
    }
    fn wire(v: u32) -> Self {
        v
    }
}
impl<'a> Element<'a> for o::BindingId {
    type Owned = Self;
    type Wire = u32;
    fn owned(v: &'a Self) -> Self {
        *v
    }
    fn wire(v: u32) -> Self {
        Self(v)
    }
}
impl<'a> Element<'a> for Option<u32> {
    type Owned = Self;
    type Wire = ForwardsUOffset<w::OptionalIndex<'a>>;
    fn owned(v: &'a Self) -> Self {
        *v
    }
    fn wire(v: w::OptionalIndex<'a>) -> Self {
        v.value()
    }
}
impl<'a> Element<'a> for &'a str {
    type Owned = String;
    type Wire = ForwardsUOffset<&'a str>;
    fn owned(v: &'a String) -> Self {
        v
    }
    fn wire(v: &'a str) -> Self {
        v
    }
}
pub(crate) fn span(v: &w::Span) -> Span {
    Span::new(v.start(), v.end())
}

macro_rules! view {
    ($name:ident, $owned:ty) => {
        #[derive(Debug, Clone, Copy)]
        pub enum $name<'a> {
            Owned(&'a $owned),
            Wire(w::$name<'a>),
        }
        impl<'a> From<&'a $owned> for $name<'a> {
            fn from(v: &'a $owned) -> Self {
                Self::Owned(v)
            }
        }
        impl<'a> From<w::$name<'a>> for $name<'a> {
            fn from(v: w::$name<'a>) -> Self {
                Self::Wire(v)
            }
        }
        impl<'a> Element<'a> for $name<'a> {
            type Owned = $owned;
            type Wire = ForwardsUOffset<w::$name<'a>>;
            fn owned(v: &'a $owned) -> Self {
                v.into()
            }
            fn wire(v: w::$name<'a>) -> Self {
                v.into()
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
macro_rules! sequence {
    ($name:ident, $ty:ty) => {
        pub fn $name(self) -> Sequence<'a, $ty> {
            match self {
                Self::Owned(v) => Sequence::Owned(v.$name()),
                Self::Wire(v) => Sequence::Wire(v.$name()),
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
macro_rules! enumeration {
    ($function:ident, $ty:path, $wire:ident, $($variant:ident),+ $(,)?) => {
        pub(crate) fn $function(v: w::$wire) -> $ty {
            match v { $(w::$wire::$variant => <$ty>::$variant,)+ _ => unreachable!("verified enum") }
        }
    };
}
enumeration!(
    commonjs,
    o::CommonJsGlobal,
    CommonJsGlobal,
    Require,
    Module,
    Exports
);
enumeration!(
    dependency_kind,
    o::DependencyKind,
    DependencyKind,
    Import,
    ReExport,
    DynamicImport,
    Require,
    ImportType
);
enumeration!(
    effect_reason,
    ef::EffectReason,
    EffectReason,
    UnknownExpression,
    UnknownControl,
    Invocation,
    Construction,
    PropertyRead,
    Coercion,
    Mutation,
    SpreadOrIteration,
    Destructuring,
    ClassHeritage,
    ComputedKey,
    StaticBlock,
    UnresolvedRead,
    BindingRead,
    OpaqueCommonJs
);
enumeration!(
    value_use,
    inv::ValueUse,
    ValueUse,
    Discarded,
    StatementValue,
    Required
);
enumeration!(
    context_kind,
    ex::ContextKind,
    ContextKind,
    Callable,
    InstanceInitializer
);
enumeration!(shorthand, em::Shorthand, Shorthand, None, Property, Proto);
enumeration!(
    lazy_wrapper_kind,
    em::LazyWrapperKind,
    LazyWrapperKind,
    Unknown,
    LazyImport,
    LazyAction
);
enumeration!(
    namespace_reason,
    em::NamespaceReason,
    NamespaceReason,
    Other,
    Receiver,
    Tag,
    Write,
    Delete
);

view!(ModuleFacts, o::ModuleFacts);
view!(Binding, o::Binding);
view!(Reference, o::Reference);
view!(Dependency, o::Dependency);
view!(ImportBinding, o::ImportBinding);
view!(Export, o::Export);
view!(Directive, o::Directive);
view!(EffectSite, ef::EffectSite);
view!(IdentifierCallee, inv::IdentifierCallee);
view!(InvocationArgument, inv::InvocationArgument);
view!(Invocation, inv::Invocation);
view!(ExecutionContext, ex::ExecutionContext);
view!(ExecutionFacts, ex::ExecutionFacts);
view!(PlacementFacts, em::PlacementFacts);
view!(NameRestrictions, em::NameRestrictions);
view!(NamespaceUse, em::NamespaceUse);
view!(SourceSlot, em::SourceSlot);
view!(CalleePlan, em::CalleePlan);
view!(PureLowering, em::PureLowering);
view!(Operation, em::Operation);
view!(SegmentRecipe, em::SegmentRecipe);
view!(EmissionFacts, em::EmissionFacts);
view!(DynamicNamespace, em::DynamicNamespace);
view!(LazyModuleCandidate, em::LazyModuleCandidate);
view!(LazyOptionProperty, em::LazyOptionProperty);
view!(LazySelectorCandidate, em::LazySelectorCandidate);

impl<'a> ModuleFacts<'a> {
    pub fn ecco(self) -> Option<ecco::views::ParsedFileView<'a>> {
        match self {
            Self::Owned(v) => v.ecco().map(Into::into),
            Self::Wire(v) => v.ecco().map(Into::into),
        }
    }
    scalar!(
        ecco_declarations_hash,
        crate::DeclarationFingerprint,
        |v: &w::Fingerprint| crate::DeclarationFingerprint::from_bytes(std::array::from_fn(|i| v
            .bytes()
            .get(i)))
    );
    scalar!(commonjs, bool);
    sequence!(top_level_await, Span);
    sequence!(effects, EffectSite<'a>);
    sequence!(invocations, Invocation<'a>);
    nested!(emission, EmissionFacts);
    sequence!(bindings, Binding<'a>);
    sequence!(references, Reference<'a>);
    sequence!(dependencies, Dependency<'a>);
    sequence!(imports, ImportBinding<'a>);
    sequence!(exports, Export<'a>);
    sequence!(directives, Directive<'a>);
    pub fn hashbang(self) -> Option<Directive<'a>> {
        match self {
            Self::Owned(v) => v.hashbang().map(Into::into),
            Self::Wire(v) => v.hashbang().map(Into::into),
        }
    }
}
impl<'a> Binding<'a> {
    scalar!(id, o::BindingId, o::BindingId);
    scalar!(name, &'a str);
    sequence!(declarations, Span);
    scalar!(top_level, bool);
    scalar!(function_scoped, bool);
    scalar!(function, bool);
    scalar!(import, bool);
    scalar!(constant, bool);
    scalar!(type_only, bool);
    scalar!(ambient, bool);
}
impl<'a> Reference<'a> {
    scalar!(name, &'a str);
    scalar!(span, Span, span);
    scalar!(binding, Option<o::BindingId>, |v: Option<u32>| v
        .map(o::BindingId));
    scalar!(read, bool);
    scalar!(write, bool);
    scalar!(type_only, bool);
    scalar!(async_deferred, bool);
    scalar!(commonjs, Option<o::CommonJsGlobal>, |v: Option<
        w::CommonJsGlobal,
    >| v.map(commonjs));
}
impl<'a> Dependency<'a> {
    scalar!(id, o::DependencyId, o::DependencyId);
    scalar!(kind, o::DependencyKind, dependency_kind);
    scalar!(span, Span, span);
    scalar!(specifier, &'a str);
    scalar!(specifier_span, Span, span);
    scalar!(type_only, bool);
    scalar!(enclosing_function, Option<Span>, |v: Option<&w::Span>| v
        .map(span));
    pub fn pattern(self) -> Option<Sequence<'a, &'a str>> {
        match self {
            Self::Owned(v) => v.pattern().map(Sequence::Owned),
            Self::Wire(v) => v.pattern().map(Sequence::Wire),
        }
    }
    pub fn matches_specifier(self, request: &str) -> bool {
        let Some(parts) = self.pattern() else {
            return request == self.specifier();
        };
        let Some(first) = parts.first() else {
            return false;
        };
        let Some(mut rest) = request.strip_prefix(first) else {
            return false;
        };
        if parts.len() == 1 {
            return rest.is_empty();
        }
        let Some(middle) = rest.strip_suffix(parts.last().unwrap()) else {
            return false;
        };
        rest = middle;
        for part in parts.iter().skip(1).take(parts.len() - 2) {
            let Some(index) = rest.find(part) else {
                return false;
            };
            rest = &rest[index + part.len()..];
        }
        true
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedName<'a> {
    Name(&'a str),
    Namespace,
}
impl<'a> From<&'a o::ImportedName> for ImportedName<'a> {
    fn from(v: &'a o::ImportedName) -> Self {
        match v {
            o::ImportedName::Name(n) => Self::Name(n),
            o::ImportedName::Namespace => Self::Namespace,
        }
    }
}
impl<'a> From<w::ImportedName<'a>> for ImportedName<'a> {
    fn from(v: w::ImportedName<'a>) -> Self {
        match v.value_type() {
            w::ImportedNameValue::NamedImport => {
                Self::Name(v.value_as_named_import().unwrap().name())
            }
            w::ImportedNameValue::NamespaceImport => Self::Namespace,
            _ => unreachable!("verified import"),
        }
    }
}
impl ImportedName<'_> {
    pub fn to_owned(self) -> o::ImportedName {
        match self {
            Self::Name(v) => o::ImportedName::Name(v.to_owned()),
            Self::Namespace => o::ImportedName::Namespace,
        }
    }
}
impl<'a> ImportBinding<'a> {
    scalar!(dependency, o::DependencyId, o::DependencyId);
    scalar!(binding, o::BindingId, o::BindingId);
    nested!(imported, ImportedName);
    scalar!(span, Span, span);
    scalar!(imported_span, Span, span);
    scalar!(type_only, bool);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportValue<'a> {
    Binding(o::BindingId),
    ReExport {
        dependency: o::DependencyId,
        imported: ImportedName<'a>,
    },
    Expression(Span),
}
impl<'a> Export<'a> {
    scalar!(name, Option<&'a str>);
    scalar!(span, Span, span);
    scalar!(name_span, Option<Span>, |v: Option<&w::Span>| v.map(span));
    scalar!(type_only, bool);
    pub fn value(self) -> ExportValue<'a> {
        match self {
            Self::Owned(v) => match v.value() {
                o::ExportValue::Binding(b) => ExportValue::Binding(*b),
                o::ExportValue::ReExport {
                    dependency,
                    imported,
                } => ExportValue::ReExport {
                    dependency: *dependency,
                    imported: imported.into(),
                },
                o::ExportValue::Expression(s) => ExportValue::Expression(*s),
            },
            Self::Wire(v) => match v.value_type() {
                w::ExportValue::BindingExport => ExportValue::Binding(o::BindingId(
                    v.value_as_binding_export().unwrap().binding(),
                )),
                w::ExportValue::ReExport => {
                    let v = v.value_as_re_export().unwrap();
                    ExportValue::ReExport {
                        dependency: o::DependencyId(v.dependency()),
                        imported: v.imported().into(),
                    }
                }
                w::ExportValue::ExpressionExport => {
                    ExportValue::Expression(span(v.value_as_expression_export().unwrap().span()))
                }
                _ => unreachable!("verified export"),
            },
        }
    }
}
impl<'a> Directive<'a> {
    scalar!(value, &'a str);
    scalar!(span, Span, span);
}
impl EffectSite<'_> {
    scalar!(span, Span, span);
    scalar!(reason, ef::EffectReason, effect_reason);
    scalar!(deferred, Option<Span>, |v: Option<&w::Span>| v.map(span));
    pub fn to_owned(self) -> ef::EffectSite {
        ef::EffectSite::new(self.span(), self.reason(), self.deferred())
    }
}
impl PartialEq for EffectSite<'_> {
    fn eq(&self, other: &Self) -> bool {
        (self.span(), self.reason(), self.deferred())
            == (other.span(), other.reason(), other.deferred())
    }
}
impl Eq for EffectSite<'_> {}
impl<'a> IdentifierCallee<'a> {
    scalar!(name, &'a str);
    scalar!(binding, Option<o::BindingId>, |v: Option<u32>| v
        .map(o::BindingId));
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeShape<'a> {
    Identifier(IdentifierCallee<'a>),
    Member {
        receiver: Span,
        receiver_identifier: Option<IdentifierCallee<'a>>,
        property: Option<&'a str>,
        computed: bool,
    },
    Other,
}
impl InvocationArgument<'_> {
    scalar!(span, Span, span);
    scalar!(spread, bool);
    pub fn to_owned(self) -> inv::InvocationArgument {
        inv::InvocationArgument::new(self.span(), self.spread())
    }
}
impl IdentifierCallee<'_> {
    pub fn to_owned(self) -> inv::IdentifierCallee {
        inv::IdentifierCallee::new(self.name().to_owned(), self.binding())
    }
}
impl CalleeShape<'_> {
    pub fn to_owned(self) -> inv::CalleeShape {
        match self {
            Self::Identifier(v) => inv::CalleeShape::Identifier(v.to_owned()),
            Self::Member {
                receiver,
                receiver_identifier,
                property,
                computed,
            } => inv::CalleeShape::Member {
                receiver,
                receiver_identifier: receiver_identifier.map(IdentifierCallee::to_owned),
                property: property.map(str::to_owned),
                computed,
            },
            Self::Other => inv::CalleeShape::Other,
        }
    }
}
impl<'a> Invocation<'a> {
    scalar!(span, Span, span);
    scalar!(callee, Span, span);
    sequence!(arguments, InvocationArgument<'a>);
    scalar!(construct, bool);
    scalar!(optional_call, bool);
    scalar!(optional_chain, bool);
    pub fn pure(self) -> Option<Span> {
        match self {
            Self::Owned(v) => v.pure(),
            Self::Wire(v) => v.pure_().map(span),
        }
    }
    scalar!(deferred, Option<Span>, |v: Option<&w::Span>| v.map(span));
    scalar!(value_use, inv::ValueUse, value_use);
    pub fn shape(self) -> CalleeShape<'a> {
        match self {
            Self::Owned(v) => match v.shape() {
                inv::CalleeShape::Identifier(v) => CalleeShape::Identifier(v.into()),
                inv::CalleeShape::Member {
                    receiver,
                    receiver_identifier,
                    property,
                    computed,
                } => CalleeShape::Member {
                    receiver: *receiver,
                    receiver_identifier: receiver_identifier.as_ref().map(Into::into),
                    property: property.as_deref(),
                    computed: *computed,
                },
                inv::CalleeShape::Other => CalleeShape::Other,
            },
            Self::Wire(v) => match v.shape_type() {
                w::CalleeShape::IdentifierCallee => {
                    CalleeShape::Identifier(v.shape_as_identifier_callee().unwrap().into())
                }
                w::CalleeShape::MemberCallee => {
                    let v = v.shape_as_member_callee().unwrap();
                    CalleeShape::Member {
                        receiver: span(v.receiver()),
                        receiver_identifier: v.receiver_identifier().map(Into::into),
                        property: v.property(),
                        computed: v.computed(),
                    }
                }
                w::CalleeShape::OtherCallee => CalleeShape::Other,
                _ => unreachable!("verified callee"),
            },
        }
    }
}
impl<'a> ExecutionFacts<'a> {
    sequence!(contexts, ExecutionContext<'a>);
    sequence!(references, Option<u32>);
    sequence!(invocations, Option<u32>);
    pub fn effect_is_module_evaluation(self, site: EffectSite<'_>) -> eyre::Result<bool> {
        let context = site
            .deferred()
            .map(|span| {
                self.contexts()
                    .iter()
                    .rposition(|c| c.span() == span)
                    .map(|i| i as u32)
                    .ok_or_else(|| eyre::eyre!("missing recorded effect execution context"))
            })
            .transpose()?;
        Ok(self.is_module_evaluation(context))
    }
    pub fn is_module_evaluation(self, mut context: Option<u32>) -> bool {
        while let Some(index) = context {
            let current = self
                .contexts()
                .get(index as usize)
                .expect("validated execution context");
            if current.immediate().is_none() {
                return false;
            }
            context = current.parent();
        }
        true
    }
    pub fn context_at(self, span: Span) -> Option<u32> {
        self.contexts()
            .iter()
            .rposition(|c| c.span().start <= span.start && span.end <= c.span().end)
            .map(|i| i as u32)
    }
}
impl ExecutionContext<'_> {
    scalar!(span, Span, span);
    scalar!(parent, Option<u32>);
    scalar!(kind, ex::ContextKind, context_kind);
    scalar!(immediate, Option<u32>);
}
impl<'a> PlacementFacts<'a> {
    sequence!(declarations, o::BindingId);
    sequence!(hoisted, o::BindingId);
    scalar!(instantiable, bool);
    scalar!(default_expression, bool);
    sequence!(forward_references, o::BindingId);
    sequence!(eager_reads, u32);
    sequence!(eager_writes, u32);
    sequence!(writes, u32);
}
impl<'a> NameRestrictions<'a> {
    sequence!(named, o::BindingId);
    scalar!(inference_sensitive, bool);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceAccess<'a> {
    DirectRead { member: Span, property: &'a str },
    Native(em::NamespaceReason),
}
impl<'a> NamespaceUse<'a> {
    scalar!(reference, u32);
    pub fn access(self) -> NamespaceAccess<'a> {
        match self {
            Self::Owned(v) => match v.access() {
                em::NamespaceAccess::DirectRead { member, property } => {
                    NamespaceAccess::DirectRead {
                        member: *member,
                        property,
                    }
                }
                em::NamespaceAccess::Native(v) => NamespaceAccess::Native(*v),
            },
            Self::Wire(v) => match v.access_type() {
                w::NamespaceAccess::DirectNamespaceRead => {
                    let v = v.access_as_direct_namespace_read().unwrap();
                    NamespaceAccess::DirectRead {
                        member: span(v.member()),
                        property: v.property(),
                    }
                }
                w::NamespaceAccess::NativeNamespace => NamespaceAccess::Native(namespace_reason(
                    v.access_as_native_namespace().unwrap().reason(),
                )),
                _ => unreachable!("verified namespace"),
            },
        }
    }
}
impl<'a> SourceSlot<'a> {
    scalar!(span, Span, span);
    sequence!(operations, u32);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStep<'a> {
    Static {
        property: &'a str,
        optional: bool,
    },
    Computed {
        property: SourceSlot<'a>,
        optional: bool,
    },
    Call {
        suffix: SourceSlot<'a>,
    },
}
impl<'a> Element<'a> for ChainStep<'a> {
    type Owned = em::ChainStep;
    type Wire = ForwardsUOffset<w::ChainStep<'a>>;
    fn owned(v: &'a em::ChainStep) -> Self {
        match v {
            em::ChainStep::Static { property, optional } => Self::Static {
                property,
                optional: *optional,
            },
            em::ChainStep::Computed { property, optional } => Self::Computed {
                property: property.into(),
                optional: *optional,
            },
            em::ChainStep::Call { suffix } => Self::Call {
                suffix: suffix.into(),
            },
        }
    }
    fn wire(v: w::ChainStep<'a>) -> Self {
        match v.value_type() {
            w::ChainStepValue::StaticChainStep => {
                let v = v.value_as_static_chain_step().unwrap();
                Self::Static {
                    property: v.property(),
                    optional: v.optional(),
                }
            }
            w::ChainStepValue::ComputedChainStep => {
                let v = v.value_as_computed_chain_step().unwrap();
                Self::Computed {
                    property: v.property().into(),
                    optional: v.optional(),
                }
            }
            w::ChainStepValue::CallChainStep => Self::Call {
                suffix: v.value_as_call_chain_step().unwrap().suffix().into(),
            },
            _ => unreachable!("verified chain"),
        }
    }
}
impl<'a> CalleePlan<'a> {
    nested!(base, SourceSlot);
    sequence!(steps, ChainStep<'a>);
}
impl<'a> PureLowering<'a> {
    nested!(callee, SourceSlot);
    pub fn chain(self) -> Option<CalleePlan<'a>> {
        match self {
            Self::Owned(v) => v.chain().map(Into::into),
            Self::Wire(v) => v.chain().map(Into::into),
        }
    }
    sequence!(arguments, SourceSlot<'a>);
    scalar!(guard_start, Option<u32>);
    scalar!(crosses_await_guard, bool);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportForm<'a> {
    Remove,
    Declaration {
        declaration: Span,
    },
    Default {
        value: SourceSlot<'a>,
        hoisted: bool,
    },
}
impl<'a> From<&'a em::ExportForm> for ExportForm<'a> {
    fn from(v: &'a em::ExportForm) -> Self {
        match v {
            em::ExportForm::Remove => Self::Remove,
            em::ExportForm::Declaration { declaration } => Self::Declaration {
                declaration: *declaration,
            },
            em::ExportForm::Default { value, hoisted } => Self::Default {
                value: value.into(),
                hoisted: *hoisted,
            },
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind<'a> {
    Binding {
        binding: o::BindingId,
        declaration: u32,
        shorthand: em::Shorthand,
    },
    Reference {
        reference: u32,
        shorthand: em::Shorthand,
    },
    Namespace {
        usage: u32,
    },
    Dependency {
        dependency: o::DependencyId,
    },
    Export {
        span: Span,
        form: ExportForm<'a>,
    },
    SourceHint {
        span: Span,
    },
    Invocation {
        invocation: u32,
        lowering: PureLowering<'a>,
    },
}
impl<'a> Operation<'a> {
    scalar!(parent, Option<u32>);
    pub fn kind(self) -> OperationKind<'a> {
        use OperationKind as K;
        match self {
            Self::Owned(v) => match v.kind() {
                em::OperationKind::Binding {
                    binding,
                    declaration,
                    shorthand,
                } => K::Binding {
                    binding: *binding,
                    declaration: *declaration,
                    shorthand: *shorthand,
                },
                em::OperationKind::Reference {
                    reference,
                    shorthand,
                } => K::Reference {
                    reference: *reference,
                    shorthand: *shorthand,
                },
                em::OperationKind::Namespace { usage } => K::Namespace { usage: *usage },
                em::OperationKind::Dependency { dependency } => K::Dependency {
                    dependency: *dependency,
                },
                em::OperationKind::Export { span, form } => K::Export {
                    span: *span,
                    form: form.into(),
                },
                em::OperationKind::SourceHint { span } => K::SourceHint { span: *span },
                em::OperationKind::Invocation {
                    invocation,
                    lowering,
                } => K::Invocation {
                    invocation: *invocation,
                    lowering: lowering.into(),
                },
            },
            Self::Wire(v) => match v.kind_type() {
                w::OperationKind::BindingOperation => {
                    let v = v.kind_as_binding_operation().unwrap();
                    K::Binding {
                        binding: o::BindingId(v.binding()),
                        declaration: v.declaration(),
                        shorthand: shorthand(v.shorthand()),
                    }
                }
                w::OperationKind::ReferenceOperation => {
                    let v = v.kind_as_reference_operation().unwrap();
                    K::Reference {
                        reference: v.reference(),
                        shorthand: shorthand(v.shorthand()),
                    }
                }
                w::OperationKind::NamespaceOperation => K::Namespace {
                    usage: v.kind_as_namespace_operation().unwrap().usage(),
                },
                w::OperationKind::DependencyOperation => K::Dependency {
                    dependency: o::DependencyId(
                        v.kind_as_dependency_operation().unwrap().dependency(),
                    ),
                },
                w::OperationKind::SourceHintOperation => K::SourceHint {
                    span: span(v.kind_as_source_hint_operation().unwrap().span()),
                },
                w::OperationKind::InvocationOperation => {
                    let v = v.kind_as_invocation_operation().unwrap();
                    K::Invocation {
                        invocation: v.invocation(),
                        lowering: v.lowering().into(),
                    }
                }
                w::OperationKind::ExportOperation => {
                    let v = v.kind_as_export_operation().unwrap();
                    let form = match v.form_type() {
                        w::ExportForm::RemoveExport => ExportForm::Remove,
                        w::ExportForm::DeclarationExport => ExportForm::Declaration {
                            declaration: span(
                                v.form_as_declaration_export().unwrap().declaration(),
                            ),
                        },
                        w::ExportForm::DefaultExport => {
                            let v = v.form_as_default_export().unwrap();
                            ExportForm::Default {
                                value: v.value().into(),
                                hoisted: v.hoisted(),
                            }
                        }
                        _ => unreachable!("verified export form"),
                    };
                    K::Export {
                        span: span(v.span()),
                        form,
                    }
                }
                _ => unreachable!("verified operation"),
            },
        }
    }
}
impl OperationKind<'_> {
    pub fn span(self, facts: ModuleFacts<'_>, recipe: SegmentRecipe<'_>) -> Span {
        match self {
            Self::Binding {
                binding,
                declaration,
                ..
            } => facts
                .bindings()
                .iter()
                .find(|b| b.id() == binding)
                .expect("validated binding")
                .declarations()
                .get(declaration as usize)
                .expect("validated declaration"),
            Self::Reference { reference, .. } => facts
                .references()
                .get(reference as usize)
                .expect("validated reference")
                .span(),
            Self::Namespace { usage } => match recipe
                .namespaces()
                .get(usage as usize)
                .expect("validated namespace")
                .access()
            {
                NamespaceAccess::DirectRead { member, .. } => member,
                NamespaceAccess::Native(_) => unreachable!("only direct reads have operations"),
            },
            Self::Dependency { dependency } => facts
                .dependencies()
                .iter()
                .find(|d| d.id() == dependency)
                .expect("validated dependency")
                .span(),
            Self::Export { span, .. } | Self::SourceHint { span } => span,
            Self::Invocation { invocation, .. } => facts
                .invocations()
                .get(invocation as usize)
                .expect("validated invocation")
                .span(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiSpan<'a>(pub Sequence<'a, Span>);
impl<'a> MultiSpan<'a> {
    pub fn spans(self) -> Sequence<'a, Span> {
        self.0
    }
    pub fn contains(self, span: Span) -> bool {
        self.0
            .iter()
            .any(|s| s.start <= span.start && span.end <= s.end)
    }
}
impl<'a> SegmentRecipe<'a> {
    pub fn deferred(self) -> DeferredForm<'a> {
        match self {
            Self::Owned(v) => match v.deferred() {
                em::DeferredForm::Unsupported => DeferredForm::Unsupported,
                em::DeferredForm::Function => DeferredForm::Function,
                em::DeferredForm::Effect => DeferredForm::Effect,
                em::DeferredForm::Variable {
                    binding,
                    initializer,
                } => DeferredForm::Variable {
                    binding: *binding,
                    initializer: initializer.as_ref().map(Into::into),
                },
            },
            Self::Wire(v) => match v.deferred_type() {
                w::DeferredForm::UnsupportedDeferred => DeferredForm::Unsupported,
                w::DeferredForm::FunctionDeferred => DeferredForm::Function,
                w::DeferredForm::EffectDeferred => DeferredForm::Effect,
                w::DeferredForm::VariableDeferred => {
                    let v = v.deferred_as_variable_deferred().unwrap();
                    DeferredForm::Variable {
                        binding: o::BindingId(v.binding()),
                        initializer: v.initializer().map(Into::into),
                    }
                }
                _ => unreachable!("verified deferred form"),
            },
        }
    }
    pub fn source(self) -> MultiSpan<'a> {
        MultiSpan(match self {
            Self::Owned(v) => Sequence::Owned(v.source().as_slice()),
            Self::Wire(v) => Sequence::Wire(v.source()),
        })
    }
    scalar!(connector_only, bool);
    nested!(placement, PlacementFacts);
    nested!(names, NameRestrictions);
    sequence!(namespaces, NamespaceUse<'a>);
    sequence!(operations, Operation<'a>);
}
impl<'a> EmissionFacts<'a> {
    nested!(execution, ExecutionFacts);
    sequence!(segments, SegmentRecipe<'a>);
    sequence!(nested_bindings, o::BindingId);
    sequence!(free_references, u32);
    sequence!(dynamic_namespaces, DynamicNamespace<'a>);
    sequence!(lazy_modules, LazyModuleCandidate<'a>);
    sequence!(lazy_selectors, LazySelectorCandidate<'a>);
    scalar!(uses_this, bool);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredForm<'a> {
    Unsupported,
    Function,
    Effect,
    Variable {
        binding: o::BindingId,
        initializer: Option<SourceSlot<'a>>,
    },
}

impl<'a> Element<'a> for (&'a str, bool) {
    type Owned = (String, bool);
    type Wire = ForwardsUOffset<w::NamespaceProperty<'a>>;
    fn owned(value: &'a Self::Owned) -> Self {
        (&value.0, value.1)
    }
    fn wire(value: w::NamespaceProperty<'a>) -> Self {
        (value.name(), value.receiver())
    }
}

impl<'a> DynamicNamespace<'a> {
    scalar!(dependency, o::DependencyId, o::DependencyId);
    pub fn properties(self) -> Option<Sequence<'a, (&'a str, bool)>> {
        match self {
            Self::Owned(v) => v.properties().map(Sequence::Owned),
            Self::Wire(v) => v.properties().map(Sequence::Wire),
        }
    }
}

impl<'a> LazyModuleCandidate<'a> {
    scalar!(segment, u32);
    scalar!(binding, o::BindingId, o::BindingId);
    scalar!(constructor, u32);
    scalar!(dependency, o::DependencyId, o::DependencyId);
    scalar!(import_span, Span, span);
    scalar!(declaration, Span, span);
    scalar!(construction, Span, span);
    sequence!(arguments, SourceSlot<'a>);
    pub fn options_properties(self) -> Option<Sequence<'a, LazyOptionProperty<'a>>> {
        match self {
            Self::Owned(v) => v.options_properties().map(Sequence::Owned),
            Self::Wire(v) => v.options_properties().map(Sequence::Wire),
        }
    }
    pub fn callback(self) -> SourceSlot<'a> {
        self.arguments().get(0).expect("validated lazy callback")
    }
    pub fn options(self) -> Option<SourceSlot<'a>> {
        self.arguments().get(1)
    }
}

impl LazyOptionProperty<'_> {
    scalar!(key, Span, span);
    scalar!(value, Span, span);
}

impl<'a> LazySelectorCandidate<'a> {
    scalar!(segment, u32);
    scalar!(kind, em::LazyWrapperKind, lazy_wrapper_kind);
    scalar!(constructor, u32);
    scalar!(module_binding, o::BindingId, o::BindingId);
    scalar!(module_reference, u32);
    scalar!(getter_reference, u32);
    scalar!(property, &'a str);
    scalar!(property_span, Span, span);
    scalar!(construction, Span, span);
    sequence!(arguments, SourceSlot<'a>);
    pub fn module_argument(self) -> SourceSlot<'a> {
        self.arguments()
            .get(0)
            .expect("validated lazy module argument")
    }
    pub fn getter(self) -> SourceSlot<'a> {
        self.arguments().get(1).expect("validated lazy getter")
    }
}

macro_rules! equal_view {
    ($ty:ident, $($field:ident),+ $(,)?) => {
        impl PartialEq for $ty<'_> {
            fn eq(&self, other: &Self) -> bool { $(self.$field() == other.$field())&&+ }
        }
        impl Eq for $ty<'_> {}
    };
}
equal_view!(
    Binding,
    id,
    name,
    declarations,
    top_level,
    function_scoped,
    function,
    import,
    constant,
    type_only,
    ambient
);
equal_view!(
    Reference,
    name,
    span,
    binding,
    read,
    write,
    type_only,
    commonjs,
    async_deferred
);
equal_view!(
    Dependency,
    id,
    kind,
    span,
    specifier,
    pattern,
    specifier_span,
    type_only,
    enclosing_function
);
equal_view!(
    ImportBinding,
    dependency,
    binding,
    imported,
    span,
    imported_span,
    type_only
);
equal_view!(Export, name, value, span, name_span, type_only);
equal_view!(Directive, value, span);
equal_view!(IdentifierCallee, name, binding);
equal_view!(InvocationArgument, span, spread);
equal_view!(
    Invocation,
    span,
    callee,
    shape,
    arguments,
    construct,
    optional_call,
    optional_chain,
    pure,
    deferred,
    value_use
);
equal_view!(ExecutionContext, span, parent, kind, immediate);
equal_view!(ExecutionFacts, contexts, references, invocations);
equal_view!(
    PlacementFacts,
    declarations,
    hoisted,
    instantiable,
    default_expression,
    forward_references,
    eager_reads,
    eager_writes,
    writes
);
equal_view!(NameRestrictions, named, inference_sensitive);
equal_view!(NamespaceUse, reference, access);
equal_view!(SourceSlot, span, operations);
equal_view!(CalleePlan, base, steps);
equal_view!(
    PureLowering,
    callee,
    chain,
    arguments,
    guard_start,
    crosses_await_guard
);
equal_view!(Operation, parent, kind);
equal_view!(
    SegmentRecipe,
    source,
    connector_only,
    placement,
    deferred,
    names,
    namespaces,
    operations
);
equal_view!(
    EmissionFacts,
    execution,
    segments,
    nested_bindings,
    free_references,
    dynamic_namespaces,
    lazy_modules,
    lazy_selectors,
    uses_this
);
equal_view!(DynamicNamespace, dependency, properties);
equal_view!(
    LazyModuleCandidate,
    segment,
    binding,
    constructor,
    dependency,
    import_span,
    declaration,
    construction,
    arguments,
    options_properties
);
equal_view!(LazyOptionProperty, key, value);
equal_view!(
    LazySelectorCandidate,
    segment,
    kind,
    constructor,
    module_binding,
    module_reference,
    getter_reference,
    property,
    property_span,
    construction,
    arguments
);

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
