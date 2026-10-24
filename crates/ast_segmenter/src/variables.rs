use std::fmt::Display;

use ahashmap::AHashMap;
use compact_str::CompactString;

/// Hoisting level that a symbol is declared at.
/// See: <https://developer.mozilla.org/en-US/docs/Glossary/Hoisting>
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoistingLevel {
    /// The value and its effects are usable in scope before it is declared.
    /// This is only true for `import` declarations.
    ImportHoisting,
    /// The value and its effects can be hoisted to the top of the scope
    /// without any side effects. Only true for `function` declarations
    /// (not function expressions).
    FunctionHoisting,
    /// Neither the value nor the effects are hoisted, but the name declaration
    /// is hoisted to the top of its declared scope.
    LetConstHoisting,
}

impl Display for HoistingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoistingLevel::ImportHoisting => write!(f, "hoist:import"),
            HoistingLevel::FunctionHoisting => write!(f, "hoist:function"),
            HoistingLevel::LetConstHoisting => write!(f, "hoist:let/const"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariableScope {
    /// Variables declared within the current scope
    local_symbols: AHashMap<CompactString, HoistingLevel>,
    /// Names in this scope that escape, e.g. they are used in this scope,
    /// but are not declared in this scope.
    /// The value is true only when every reference to the name is type-only.
    escaped_symbols: AHashMap<CompactString, bool>,
}

impl VariableScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_locals(&self) -> impl Iterator<Item = &CompactString> {
        self.local_symbols.keys()
    }

    /// Returns an iterator over all local symbols with their hoisting levels.
    pub fn get_locals_with_hoisting(
        &self,
    ) -> impl Iterator<Item = (&CompactString, HoistingLevel)> {
        self.local_symbols.iter().map(|(k, h)| (k, *h))
    }

    /// Returns an iterator over all escaped (unresolved) symbols in this scope.
    pub fn get_escaped_symbols(&self) -> impl Iterator<Item = &CompactString> {
        self.escaped_symbols.keys()
    }

    /// Returns escaped names and whether all their references are type-only.
    pub fn get_escaped_symbols_with_type_only(
        &self,
    ) -> impl Iterator<Item = (&CompactString, bool)> {
        self.escaped_symbols
            .iter()
            .map(|(name, type_only)| (name, *type_only))
    }

    /// Insert a local symbol with its hoisting level directly.
    /// Useful for constructing test fixtures without parsing source code.
    pub fn insert_local(&mut self, name: CompactString, hoisting: HoistingLevel) {
        self.local_symbols.insert(name, hoisting);
    }

    /// Insert a local symbol from a `&str`.
    pub fn insert_local_str(&mut self, name: &str, hoisting: HoistingLevel) {
        self.insert_local(CompactString::new(name), hoisting);
    }

    /// Insert an escaped symbol directly.
    /// Useful for constructing test fixtures without parsing source code.
    pub fn insert_escaped(&mut self, name: CompactString) {
        self.insert_escaped_reference(name, false);
    }

    /// Merge a reference, preserving runtime usage regardless of insertion order.
    pub fn insert_escaped_reference(&mut self, name: CompactString, is_type_only: bool) {
        self.escaped_symbols
            .entry(name)
            .and_modify(|type_only| *type_only &= is_type_only)
            .or_insert(is_type_only);
    }
}
