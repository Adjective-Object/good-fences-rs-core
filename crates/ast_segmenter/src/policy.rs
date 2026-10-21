//! Checked source-local subset; resolution and runtime support are separate gates.
use std::collections::HashSet;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::ScopeFlags;
use oxc_span::Span;

use crate::owned::{DependencyKind, ModuleFacts};

pub(crate) fn check(program: &Program<'_>, facts: &ModuleFacts) -> eyre::Result<()> {
    let mut policy = Policy {
        commonjs: program.source_type.is_commonjs(),
        function_depth: 0,
        arguments_depth: 0,
        wrapper_arguments: HashSet::new(),
        module_receivers: HashSet::new(),
        errors: Vec::new(),
    };
    policy.visit_program(program);
    let requires: Vec<_> = facts
        .dependencies()
        .iter()
        .filter(|d| d.kind() == DependencyKind::Require)
        .collect();
    for reference in facts.references() {
        if reference.binding().is_some() || reference.type_only() {
            continue;
        }
        match reference.name() {
            "require" | "module" | "exports" if !policy.commonjs => {
                policy.reject(reference.span(), "mixed ESM/CommonJS wrapper reference");
            }
            "require" => {
                let i = requires.partition_point(|d| d.span().start <= reference.span().start);
                let direct_call = i.checked_sub(1).is_some_and(|i| {
                    let d = requires[i];
                    reference.span().end <= d.specifier_span().start
                        && reference.span().start >= d.span().start
                });
                if reference.write() || !direct_call {
                    policy.reject(
                        reference.span(),
                        "unsupported require reassignment, alias or indirect use",
                    );
                }
            }
            "module"
                if !reference.write()
                    && !policy.module_receivers.contains(&reference.span().start) =>
            {
                policy.reject(
                    reference.span(),
                    "unsupported module access (only module.exports is supported)",
                );
            }
            "eval" => policy.reject(reference.span(), "unsupported unshadowed eval"),
            "arguments"
                if policy.commonjs
                    && policy.wrapper_arguments.contains(&reference.span().start) =>
            {
                policy.reject(
                    reference.span(),
                    "unsupported CommonJS wrapper arguments access",
                );
            }
            _ => {}
        }
    }
    if policy.commonjs {
        for binding in facts.bindings() {
            if binding.top_level()
                && !binding.type_only()
                && (matches!(binding.name(), "require" | "module" | "exports")
                    || (matches!(binding.name(), "__filename" | "__dirname")
                        && !binding.function_scoped()
                        && !binding.function()))
            {
                policy.reject(
                    binding.declarations()[0],
                    "unsupported CommonJS wrapper redeclaration",
                );
            }
        }
    }
    eyre::ensure!(policy.errors.is_empty(), "{}", policy.errors.join("; "));
    Ok(())
}

struct Policy {
    commonjs: bool,
    function_depth: usize,
    arguments_depth: usize,
    wrapper_arguments: HashSet<u32>,
    module_receivers: HashSet<u32>,
    errors: Vec<String>,
}

impl Policy {
    fn reject(&mut self, span: Span, message: &str) {
        self.errors
            .push(format!("{message} at bytes {}..{}", span.start, span.end));
    }

    fn module_declaration(&mut self, span: Span, attributes: bool, phase: bool) {
        if self.commonjs {
            self.reject(span, "mixed CommonJS/ESM declaration");
        }
        if attributes || phase {
            self.reject(span, "unsupported import attributes or import phase");
        }
    }

    fn module_receiver(&mut self, object: &Expression<'_>, exports: bool) {
        if !exports {
            return;
        }
        if let Expression::Identifier(id) = object.get_inner_expression() {
            if id.name == "module" {
                self.module_receivers.insert(id.span.start);
            }
        }
    }
}

impl<'a> Visit<'a> for Policy {
    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        if self.arguments_depth == 0 && reference.name == "arguments" {
            self.wrapper_arguments.insert(reference.span.start);
        }
    }

    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        self.module_declaration(
            declaration.span,
            declaration.with_clause.is_some(),
            declaration.phase.is_some(),
        );
        walk::walk_import_declaration(self, declaration);
    }

    fn visit_export_named_declaration(&mut self, declaration: &ExportNamedDeclaration<'a>) {
        self.module_declaration(declaration.span, declaration.with_clause.is_some(), false);
        walk::walk_export_named_declaration(self, declaration);
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        self.module_declaration(declaration.span, declaration.with_clause.is_some(), false);
        walk::walk_export_all_declaration(self, declaration);
    }

    fn visit_export_default_declaration(&mut self, declaration: &ExportDefaultDeclaration<'a>) {
        self.module_declaration(declaration.span, false, false);
        walk::walk_export_default_declaration(self, declaration);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if expression.options.is_some() || expression.phase.is_some() {
            self.reject(
                expression.span,
                "unsupported dynamic import options or phase",
            );
        }
        walk::walk_import_expression(self, expression);
    }

    fn visit_ts_import_type(&mut self, expression: &TSImportType<'a>) {
        if expression.options.is_some() {
            self.reject(expression.span, "unsupported import type options");
        }
        walk::walk_ts_import_type(self, expression);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        self.module_receiver(&member.object, member.property.name == "exports");
        walk::walk_static_member_expression(self, member);
    }

    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        self.module_receiver(&member.object,
            matches!(member.expression.get_inner_expression(), Expression::StringLiteral(s) if s.value == "exports"));
        walk::walk_computed_member_expression(self, member);
    }

    fn visit_with_statement(&mut self, statement: &WithStatement<'a>) {
        self.reject(statement.span, "unsupported with statement");
    }

    fn visit_meta_property(&mut self, property: &MetaProperty<'a>) {
        if property.meta.name == "import" {
            self.reject(property.span, "unsupported import.meta");
        }
    }

    fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
        if self.commonjs && self.function_depth == 0 {
            self.reject(expression.span, "unsupported top-level await");
        }
        walk::walk_await_expression(self, expression);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        if self.commonjs && statement.r#await && self.function_depth == 0 {
            self.reject(statement.span, "unsupported top-level for-await");
        }
        walk::walk_for_of_statement(self, statement);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        self.function_depth += 1;
        self.arguments_depth += 1;
        walk::walk_function(self, function, flags);
        self.arguments_depth -= 1;
        self.function_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, function);
        self.function_depth -= 1;
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        if matches!(
            declaration.kind,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        ) {
            self.reject(
                declaration.span,
                "unsupported resource-management transform",
            );
        }
        walk::walk_variable_declaration(self, declaration);
    }

    fn visit_jsx_element(&mut self, element: &JSXElement<'a>) {
        self.reject(element.span, "unsupported JSX transform");
    }

    fn visit_jsx_fragment(&mut self, fragment: &JSXFragment<'a>) {
        self.reject(fragment.span, "unsupported JSX transform");
    }

    fn visit_decorator(&mut self, decorator: &Decorator<'a>) {
        self.reject(decorator.span, "unsupported decorator transform");
    }

    fn visit_accessor_property(&mut self, property: &AccessorProperty<'a>) {
        self.reject(property.span, "unsupported auto-accessor transform");
    }

    fn visit_ts_enum_declaration(&mut self, declaration: &TSEnumDeclaration<'a>) {
        self.reject(declaration.span, "unsupported TypeScript enum transform");
    }

    fn visit_ts_module_declaration(&mut self, declaration: &TSModuleDeclaration<'a>) {
        self.reject(
            declaration.span,
            "unsupported TypeScript namespace transform",
        );
    }

    fn visit_formal_parameter(&mut self, parameter: &FormalParameter<'a>) {
        if parameter.accessibility.is_some() || parameter.readonly || parameter.r#override {
            self.reject(
                parameter.span,
                "unsupported TypeScript parameter property transform",
            );
        }
        walk::walk_formal_parameter(self, parameter);
    }

    fn visit_ts_import_equals_declaration(&mut self, declaration: &TSImportEqualsDeclaration<'a>) {
        self.reject(declaration.span, "unsupported TypeScript import assignment");
    }

    fn visit_ts_export_assignment(&mut self, declaration: &TSExportAssignment<'a>) {
        self.reject(declaration.span, "unsupported TypeScript export assignment");
    }

    fn visit_ts_namespace_export_declaration(
        &mut self,
        declaration: &TSNamespaceExportDeclaration<'a>,
    ) {
        self.reject(declaration.span, "unsupported TypeScript namespace export");
    }
}
