use ahashmap::{AHashMap, AHashSet};
use compact_str::CompactString as CompactStr;
use oxc_ast::ast::Statement;
use oxc_semantic::{Semantic, SymbolFlags};
use oxc_span::{GetSpan, Span};

// Hoisting level that a symbol is declared at.
// See: https://developer.mozilla.org/en-US/docs/Glossary/Hoisting
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoistingLevel {
    // The value and its effects are usable in scope before it is declared.
    // Only true for `import` declarations.
    ImportHoisting,
    // The value and its effects can be hoisted to the top of the scope.
    // Only true for `function` declarations (not function expressions).
    FunctionHoisting,
    // The name declaration is hoisted but neither value nor effects are.
    LetConstHoisting,
}

impl std::fmt::Display for HoistingLevel {
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
    /// Variables declared within the current scope.
    pub local_symbols: AHashMap<CompactStr, HoistingLevel>,
    /// Names referenced in this scope but declared elsewhere.
    pub escaped_symbols: AHashSet<CompactStr>,
}

impl VariableScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_locals(&self) -> impl Iterator<Item = &CompactStr> {
        self.local_symbols.keys()
    }

    pub fn get_locals_with_hoisting(&self) -> impl Iterator<Item = (&CompactStr, HoistingLevel)> {
        self.local_symbols.iter().map(|(k, h)| (k, *h))
    }

    pub fn get_escaped_symbols(&self) -> impl Iterator<Item = &CompactStr> {
        self.escaped_symbols.iter()
    }

    /// Insert a local symbol with its hoisting level.
    /// Useful for constructing test fixtures.
    pub fn insert_local(&mut self, name: CompactStr, hoisting: HoistingLevel) {
        self.local_symbols.insert(name, hoisting);
    }

    /// Insert an escaped symbol.
    /// Useful for constructing test fixtures.
    pub fn insert_escaped(&mut self, name: CompactStr) {
        self.escaped_symbols.insert(name);
    }
}

/// Map OXC `SymbolFlags` to our `HoistingLevel`.
pub fn hoisting_for_flags(flags: SymbolFlags) -> HoistingLevel {
    if flags.intersects(SymbolFlags::Import | SymbolFlags::TypeImport) {
        HoistingLevel::ImportHoisting
    } else if flags.contains(SymbolFlags::Function) {
        HoistingLevel::FunctionHoisting
    } else {
        HoistingLevel::LetConstHoisting
    }
}

/// Compute `VariableScope` for every top-level `Statement` in one pass.
/// Returns a `Vec` with one entry per `body[i]` in order.
///
/// - `local_symbols`: declarations in the root scope whose span falls within that
///   statement.
/// - `escaped_symbols`: names referenced in that statement but declared in a
///   different top-level statement (or globally unresolved).
pub fn variables_for_top_level_statements<'a>(
    semantic: &Semantic<'a>,
    body: &[Statement<'a>],
) -> Vec<VariableScope> {
    if body.is_empty() {
        return Vec::new();
    }

    // Collect (start, end) spans for each top-level statement.
    let stmt_spans: Vec<(u32, u32)> = body
        .iter()
        .map(|s| {
            let sp = s.span();
            (sp.start, sp.end)
        })
        .collect();

    let mut out: Vec<VariableScope> =
        (0..body.len()).map(|_| VariableScope::default()).collect();

    /// Binary-search for the statement index that contains `offset`.
    /// Statements must be non-overlapping and sorted by position.
    let find_stmt = |offset: u32| -> Option<usize> {
        // partition_point finds the first index i where stmt_spans[i].end > offset
        let i = stmt_spans.partition_point(|(_, end)| *end <= offset);
        stmt_spans
            .get(i)
            .and_then(|(start, end)| (offset >= *start && offset < *end).then_some(i))
    };

    let scoping = semantic.scoping();
    let root_scope = scoping.root_scope_id();

    // ── Locals: root-scope bindings only ──────────────────────────────────────
    // Using iter_bindings_in avoids including symbols from nested scopes.
    for sym_id in scoping.iter_bindings_in(root_scope) {
        let decl_span = scoping.symbol_span(sym_id);
        let Some(stmt_idx) = find_stmt(decl_span.start) else {
            continue;
        };
        let name = CompactStr::from(scoping.symbol_name(sym_id));
        let hoisting = hoisting_for_flags(scoping.symbol_flags(sym_id));
        out[stmt_idx].local_symbols.insert(name, hoisting);
    }

    // ── Escaped via resolved references ───────────────────────────────────────
    // For each symbol, find every value reference to it. If the reference is in a
    // different statement than the one containing the symbol's declaration,
    // the name escapes from the referencing statement.
    // Type-only references (e.g. `T extends Foo` in type parameter bounds) are
    // excluded because they are erased at runtime and do not create value dependencies.
    for sym_id in scoping.symbol_ids() {
        let sym_decl_span: Span = scoping.symbol_span(sym_id);

        for reference in scoping.get_resolved_references(sym_id) {
            // Skip type-only references — they don't create runtime dependencies.
            if reference.flags().is_type_only() {
                continue;
            }
            let ref_span = semantic.reference_span(reference);
            let Some(r_idx) = find_stmt(ref_span.start) else {
                continue;
            };
            // Check whether the symbol's declaration is inside the referencing statement.
            let (rs, re) = stmt_spans[r_idx];
            let decl_in_same_stmt =
                sym_decl_span.start >= rs && sym_decl_span.end <= re;
            if !decl_in_same_stmt {
                let name = CompactStr::from(semantic.reference_name(reference));
                out[r_idx].escaped_symbols.insert(name);
            }
        }
    }

    // ── Escaped via unresolved (global) references ────────────────────────────
    // Any unresolved value reference is always escaped from its statement.
    // Type-only unresolved references are skipped for the same reason as above.
    for ref_ids in scoping.root_unresolved_references_ids() {
        for ref_id in ref_ids {
            let reference = scoping.get_reference(ref_id);
            // Skip type-only references.
            if reference.flags().is_type_only() {
                continue;
            }
            let ref_span = semantic.reference_span(reference);
            let Some(stmt_idx) = find_stmt(ref_span.start) else {
                continue;
            };
            let name = CompactStr::from(semantic.reference_name(reference));
            out[stmt_idx].escaped_symbols.insert(name);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(src: &str) -> Vec<VariableScope> {
        let allocator = oxc_allocator::Allocator::default();
        let ret = oxc_utils_parse::parse_ts(&allocator, src);
        let semantic_ret = oxc_semantic::SemanticBuilder::new().build(&ret.program);
        variables_for_top_level_statements(&semantic_ret.semantic, &ret.program.body)
    }

    fn local_names(vs: &VariableScope) -> Vec<&str> {
        let mut v: Vec<&str> = vs.get_locals().map(|s| s.as_str()).collect();
        v.sort();
        v
    }

    fn escaped_names(vs: &VariableScope) -> Vec<&str> {
        let mut v: Vec<&str> = vs.get_escaped_symbols().map(|s| s.as_str()).collect();
        v.sort();
        v
    }

    #[test]
    fn single_const_decl_is_local() {
        let scopes = variables("const x = 1;");
        assert_eq!(scopes.len(), 1);
        assert!(local_names(&scopes[0]).contains(&"x"));
    }

    #[test]
    fn import_gets_import_hoisting() {
        let scopes = variables("import { foo } from './foo';");
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].local_symbols.get("foo"), Some(&HoistingLevel::ImportHoisting));
    }

    #[test]
    fn function_decl_gets_function_hoisting() {
        let scopes = variables("function bar() {}");
        assert_eq!(scopes.len(), 1);
        assert_eq!(
            scopes[0].local_symbols.get("bar"),
            Some(&HoistingLevel::FunctionHoisting)
        );
    }

    #[test]
    fn cross_statement_reference_is_escaped() {
        // stmt 0 declares x; stmt 1 uses x → x is escaped in stmt 1
        let scopes = variables("const x = 1;\nconst y = x + 1;");
        assert_eq!(scopes.len(), 2);
        assert!(escaped_names(&scopes[1]).contains(&"x"), "x should escape into stmt 1");
    }

    #[test]
    fn same_statement_reference_is_not_escaped() {
        // x is declared and used in the same statement
        let scopes = variables("const x = 1, y = x + 1;");
        assert_eq!(scopes.len(), 1);
        // x may or may not be in escaped; it should NOT be escaped since it's in the same stmt
        assert!(!escaped_names(&scopes[0]).contains(&"x"));
    }

    #[test]
    fn global_reference_is_escaped() {
        // `console` is unresolved → should appear in escaped_symbols
        let scopes = variables("console.log('hi');");
        assert_eq!(scopes.len(), 1);
        assert!(
            escaped_names(&scopes[0]).contains(&"console"),
            "unresolved 'console' should be escaped"
        );
    }

    #[test]
    fn multiple_statements_correct_scope() {
        let scopes = variables("const a = 1;\nconst b = 2;\nconst c = a + b;");
        assert_eq!(scopes.len(), 3);
        assert!(local_names(&scopes[0]).contains(&"a"));
        assert!(local_names(&scopes[1]).contains(&"b"));
        assert!(local_names(&scopes[2]).contains(&"c"));
        // a and b escape into stmt 2
        assert!(escaped_names(&scopes[2]).contains(&"a"));
        assert!(escaped_names(&scopes[2]).contains(&"b"));
    }
}
