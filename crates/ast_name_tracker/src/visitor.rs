use ahashmap::{AHashMap, AHashSet};
use logger_srcfile::SrcFileLogger;
use swc_atoms::Atom;
use swc_common::{Span, Spanned};
use swc_ecma_ast::{AssignPat, Module};
use swc_ecma_visit::{Visit, VisitWith};

// unique identifier of a variable declaration within a file
#[derive(Clone, Copy)]
pub struct VarID(pub Span);

pub struct VariableScope {
    /// Variables declared within the current scope, sorted by name
    local_symbols: AHashMap<swc_atoms::Atom, VarID>,

    // Names in this scope that are "hoisted"
    // This is type 1/3 hoisting from here, where a name is considered pre-declared for
    // the entire scope, even if it is declared later in the source.
    //
    // https://developer.mozilla.org/en-US/docs/Glossary/Hoisting
    escaped_symbols: AHashSet<swc_atoms::Atom>,
}

#[derive(thiserror::Error, Debug)]
pub enum VariableScopeError {
    #[error("Variable {0} is already declared in this scope")]
    DuplicateDeclaration(swc_atoms::Atom),
}

impl VariableScope {
    fn get_locals() -> &AHashMap<swc_atoms::Atom, VarID> {
        self.local_symbols
    }

    fn new() -> Self {
        Self {
            local_symbols: AHashMap::default(),
            escaped_symbols: AHashSet::default(),
        }
    }

    fn declare_local(&mut self, ident: &swc_ecma_ast::Ident) -> Result<(), VariableScopeError> {
        match self.local_symbols.entry(ident.sym.clone()) {
            ahashmap::hash_map::Entry::Occupied(_) => {
                // Should you warn here about a name collision within the scope?
                Err(VariableScopeError::DuplicateDeclaration(ident.sym.clone()))
            }
            ahashmap::hash_map::Entry::Vacant(entry) => {
                entry.insert(VarID(ident.span));
                // println!(
                //     "unregister removed_symbol? {} -> {}",
                //     ident.sym,
                //     self.escaped_symbols
                //         .iter()
                //         .map(|x| x.as_ref())
                //         .collect::<Vec<&str>>()
                //         .join(", ")
                // );
                self.escaped_symbols.remove(&ident.sym);
                Ok(())
            }
        }
    }

    // "uses" a symbol within this scope.
    //
    // If the symbol is not declared in this scope, it is added to the list of escaped symbols.
    fn use_symbol(&mut self, sym: &swc_atoms::Atom) {
        if !self.local_symbols.contains_key(sym) {
            // println!(
            //     "register escaped_symbol {} -> {}",
            //     sym,
            //     self.escaped_symbols
            //         .iter()
            //         .map(|x| x.as_ref())
            //         .collect::<Vec<&str>>()
            //         .join(", ")
            // );
            self.escaped_symbols.insert(sym.clone());
        }
    }

    // owned version of use_symbol
    fn use_symbol_owned(&mut self, sym: swc_atoms::Atom) {
        if !self.local_symbols.contains_key(&sym) {
            // println!(
            //     "register escaped_symbol {} -> {}",
            //     sym,
            //     self.escaped_symbols
            //         .iter()
            //         .map(|x| x.as_ref())
            //         .collect::<Vec<&str>>()
            //         .join(", ")
            // );
            self.escaped_symbols.insert(sym);
        }
    }
}
impl Default for VariableScope {
    fn default() -> Self {
        Self::new()
    }
}

/// Visitor that builds a VariableScope from a source file.
struct VariableScopeVisitor<'a, TLogger: SrcFileLogger> {
    logger: &'a TLogger,
    node: &'a mut VariableScope,
}

impl<'a, TLogger> VariableScopeVisitor<'a, TLogger>
where
    TLogger: SrcFileLogger,
{
    fn new(
        logger: &'a TLogger,
        root_scope: &'a mut VariableScope,
    ) -> VariableScopeVisitor<'a, TLogger> {
        Self {
            logger,
            node: root_scope,
        }
    }

    fn visit_binding_pattern(&mut self, pattern: &swc_ecma_ast::Pat) {
        match pattern {
            swc_ecma_ast::Pat::Ident(ident) => {
                self.declare_local(ident);
            }
            swc_ecma_ast::Pat::Array(array_pat) => {
                // This .iter().flatten() iterates only over the Some() elements.
                // See: https://rust-lang.github.io/rust-clippy/master/index.html#manual_flatten
                for subpattern in array_pat.elems.iter().flatten() {
                    self.visit_binding_pattern(subpattern);
                }
            }
            swc_ecma_ast::Pat::Object(object_pat) => {
                for prop in &object_pat.props {
                    match prop {
                        swc_ecma_ast::ObjectPatProp::KeyValue(kv) => {
                            self.visit_binding_pattern(&kv.value);
                        }
                        swc_ecma_ast::ObjectPatProp::Assign(assign_prop) => {
                            // little used "destructure with default" syntax:
                            // let { a = defaultValue } = destructured_object;
                            self.declare_local(&assign_prop.key.id)
                        }
                        swc_ecma_ast::ObjectPatProp::Rest(rest) => {
                            self.visit_binding_pattern(&rest.arg);
                        }
                    }
                }
            }
            swc_ecma_ast::Pat::Rest(rest_pat) => {
                self.visit_binding_pattern(&rest_pat.arg);
            }
            swc_ecma_ast::Pat::Assign(assign_pat) => {
                self.visit_binding_pattern(&assign_pat.left);
                // visit the right side of the binding pattern.
                assign_pat.right.visit_with(self);
            }
            swc_ecma_ast::Pat::Invalid(invalid_pat) => {
                self.logger
                    .src_warn(&invalid_pat.span, "invalid pattern in variable declaration");
            }
            swc_ecma_ast::Pat::Expr(expr_pat) => {
                self.logger.src_warn(
                    &expr_pat.leftmost().span(),
                    "expr pattern in variable declaration was ignored",
                );
            }
        }
    }

    fn declare_local(&mut self, ident: &swc_ecma_ast::Ident) {
        if let Err(e) = self.node.declare_local(ident) {
            self.logger.src_error(&ident.span, format!("{}", e));
        }
    }

    fn mark_all_escaped(&mut self, mut child_scope_escaped_symbols: AHashSet<Atom>) {
        for sym in child_scope_escaped_symbols.drain() {
            self.node.use_symbol_owned(sym);
        }
    }
}

impl<TLogger> Visit for VariableScopeVisitor<'_, TLogger>
where
    TLogger: SrcFileLogger,
{
    fn visit_var_decl(&mut self, node: &swc_ecma_ast::VarDecl) {
        for decl in &node.decls {
            self.visit_binding_pattern(&decl.name);
        }
        for decl in &node.decls {
            if let Some(init) = &decl.init {
                init.visit_with(self);
            }
        }
    }

    fn visit_constructor(&mut self, node: &swc_ecma_ast::Constructor) {
        // First, visit the initializer expressions in the constructor, if any.
        // We do this in the current scope, since they are evaluated in the constructor scope.
        for param in &node.params {
            if let swc_ecma_ast::ParamOrTsParamProp::TsParamProp(swc_ecma_ast::TsParamProp {
                param: swc_ecma_ast::TsParamPropParam::Assign(AssignPat { right, .. }),
                ..
            }) = param
            {
                right.visit_with(self);
            }
        }

        // Create a new scope for the child
        let mut child_scope = VariableScope::new();
        let mut child_visitor = VariableScopeVisitor::new(self.logger, &mut child_scope);

        // pre-declare function parameters in the child scope
        for param in &node.params {
            match param {
                swc_ecma_ast::ParamOrTsParamProp::Param(param) => {
                    child_visitor.visit_binding_pattern(&param.pat);
                }
                swc_ecma_ast::ParamOrTsParamProp::TsParamProp(m) => match &m.param {
                    swc_ecma_ast::TsParamPropParam::Ident(ident) => {
                        child_visitor.declare_local(ident);
                    }
                    swc_ecma_ast::TsParamPropParam::Assign(assign) => {
                        // the right side of the pattern is already visited in the above loop
                        child_visitor.visit_binding_pattern(&assign.left);
                    }
                },
            }
        }

        // visit the child scope
        node.body.visit_with(&mut child_visitor);
        self.mark_all_escaped(child_scope.escaped_symbols);
    }

    fn visit_fn_decl(&mut self, node: &swc_ecma_ast::FnDecl) {
        self.declare_local(&node.ident);
        // Create a new scope for the child
        let mut child_scope = VariableScope::new();
        let mut child_visitor = VariableScopeVisitor::new(self.logger, &mut child_scope);
        // pre-declare function parameters in the child scope
        for param in &node.function.params {
            child_visitor.visit_binding_pattern(&param.pat);
        }
        // visit the child scope
        node.function.body.visit_with(&mut child_visitor);
        self.mark_all_escaped(child_scope.escaped_symbols);
    }

    fn visit_fn_expr(&mut self, node: &swc_ecma_ast::FnExpr) {
        // Create a new scope for the child
        let mut child_scope = VariableScope::new();
        let mut child_visitor = VariableScopeVisitor::new(self.logger, &mut child_scope);
        // pre-declare function parameters in the child scope
        for param in &node.function.params {
            child_visitor.visit_binding_pattern(&param.pat);
        }
        // visit the child scope
        node.function.body.visit_with(&mut child_visitor);
        self.mark_all_escaped(child_scope.escaped_symbols);
    }

    fn visit_block_stmt(&mut self, node: &swc_ecma_ast::BlockStmt) {
        // Create a new scope for the child
        let mut child_scope = VariableScope::new();
        let mut child_visitor = VariableScopeVisitor::new(self.logger, &mut child_scope);
        // visit the child scope
        node.visit_children_with(&mut child_visitor);
        self.mark_all_escaped(child_scope.escaped_symbols);
    }

    fn visit_ident_name(&mut self, node: &swc_ecma_ast::IdentName) {
        self.node.use_symbol(&node.sym);
    }

    fn visit_import_decl(&mut self, node: &swc_ecma_ast::ImportDecl) {
        for spec in &node.specifiers {
            match spec {
                swc_ecma_ast::ImportSpecifier::Named(named_spec) => {
                    self.declare_local(&named_spec.local);
                }
                swc_ecma_ast::ImportSpecifier::Default(default_spec) => {
                    self.declare_local(&default_spec.local);
                }
                swc_ecma_ast::ImportSpecifier::Namespace(namespace_spec) => {
                    self.declare_local(&namespace_spec.local);
                }
            }
        }
    }
}

pub fn find_escaping_names<TLogger>(file_logger: TLogger, ast_node: Module) -> VariableScope
where
    TLogger: SrcFileLogger,
{
    let mut child_scope = VariableScope::new();
    let mut child_visitor = VariableScopeVisitor::new(&file_logger, &mut child_scope);
    // run the visitor
    ast_node.visit_with(&mut child_visitor);
    // get the resulting root scope
    child_scope
}

#[cfg(test)]
mod test {
    use super::*;
    use pretty_assertions::assert_eq;

    fn get_scope(src_str: &str) -> VariableScope {
        let (sourcemap, parsed_module) = swc_utils_parse::parse_ecma_src("test.ts", src_str);

        let logger = logger::StdioLogger::new();
        let file_logger = logger_srcfile::WrapFileLogger::new(&sourcemap, &logger);

        find_escaping_names(file_logger, parsed_module)
    }

    #[derive(Default)]
    struct ExpectedScope {
        local_symbols: Vec<&'static str>,
        escaped_symbols: Vec<&'static str>,
    }

    fn run_test(src_str: &str, mut expected: ExpectedScope) {
        let scope = get_scope(src_str);

        let mut locals = scope
            .local_symbols
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<_>>();
        locals.sort();
        expected.local_symbols.sort();
        assert_eq!(expected.local_symbols, locals);

        let mut escaped = scope
            .escaped_symbols
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>();
        escaped.sort();
        expected.escaped_symbols.sort();
    }

    #[test]
    fn simple_let_binding() {
        run_test(
            r#"
            let a = 1;
            "#,
            ExpectedScope {
                local_symbols: vec!["a"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn simple_var_binding() {
        run_test(
            r#"
            var b = 1;
            "#,
            ExpectedScope {
                local_symbols: vec!["b"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn simple_const_binding() {
        run_test(
            r#"
            const c = 1;
            "#,
            ExpectedScope {
                local_symbols: vec!["c"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn export_binding() {
        run_test(
            r#"
            export let a = 1;
            export var b = 1;
            export const c = 1;
            "#,
            ExpectedScope {
                local_symbols: vec!["a", "b", "c"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn escape_in_var_initializer() {
        run_test(
            r#"
            const c = forward_declared();
            "#,
            ExpectedScope {
                local_symbols: vec!["c"],
                escaped_symbols: vec!["forward_declared"],
            },
        );
    }

    #[test]
    fn forward_declared_function() {
        run_test(
            r#"
            const c = forward_declared();
            function forward_declared() {
                return 1;
            }
            "#,
            ExpectedScope {
                local_symbols: vec!["c", "forward_declared"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn shadowed_fn_params() {
        run_test(
            r#"
            const c = 1;
            function helper_fn(c, d) {
                return c + d;
            }
            "#,
            ExpectedScope {
                local_symbols: vec!["c", "helper_fn"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn escape_from_subfn() {
        run_test(
            r#"
            const c = 1;
            function helper_fn() {
                return c + d + e
            }
            "#,
            ExpectedScope {
                local_symbols: vec!["c", "helper_fn"],
                escaped_symbols: vec!["d", "e"],
            },
        );
    }

    #[test]
    fn reference_after_parent_scope() {
        run_test(
            r#"
            const c = 1;
            function helper_fn() {
                return c + d + e
            }
            const e = 1;
            "#,
            ExpectedScope {
                local_symbols: vec!["c", "e", "helper_fn"],
                escaped_symbols: vec!["d"],
            },
        );
    }

    #[test]
    fn import_statement_name() {
        run_test(
            r#"
            import { name } from 'module';
            "#,
            ExpectedScope {
                local_symbols: vec!["name"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn import_statement_name_rebound() {
        run_test(
            r#"
            import { name as rebound } from 'module';
            "#,
            ExpectedScope {
                local_symbols: vec!["rebound"],
                ..Default::default()
            },
        );
    }

    #[test]
    fn import_statement_default() {
        run_test(
            r#"
            import name from 'module';
            "#,
            ExpectedScope {
                local_symbols: vec!["name"],
                ..Default::default()
            },
        )
    }

    #[test]
    fn import_statement_ns_rebound() {
        run_test(
            r#"
            import * as rebound from 'module';
            "#,
            ExpectedScope {
                local_symbols: vec!["rebound"],
                ..Default::default()
            },
        )
    }
}
