//! Checked source-local subset; resolution and runtime support are separate gates.
use std::collections::HashSet;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::ScopeFlags;
use oxc_span::Span;

use crate::owned::{DependencyKind, ModuleFacts};

pub(crate) fn json_import_attribute(declaration: &ImportDeclaration<'_>) -> Option<Span> {
    let clause = declaration.with_clause.as_ref()?;
    (clause.with_entries.len() == 1
        && match &clause.with_entries[0].key {
            ImportAttributeKey::Identifier(key) => key.name == "type",
            ImportAttributeKey::StringLiteral(key) => key.value == "type",
        }
        && clause.with_entries[0].value.value == "json")
        .then(|| Span::new(declaration.source.span.end, clause.span.end))
}

pub(crate) fn check(program: &Program<'_>, facts: &ModuleFacts) -> eyre::Result<()> {
    let mut policy = Policy {
        commonjs: program.source_type.is_commonjs(),
        bundled_imports: facts
            .dependencies()
            .iter()
            .filter(|dependency| dependency.kind() == DependencyKind::DynamicImport)
            .map(|dependency| dependency.id().0)
            .collect(),
        resource_urls: facts
            .dependencies()
            .iter()
            .filter(|dependency| dependency.kind() == DependencyKind::Url)
            .map(|dependency| dependency.span().start)
            .collect(),
        resource_url_depth: false,
        function_depth: 0,
        arguments_depth: 0,
        wrapper_arguments: HashSet::new(),
        module_receivers: HashSet::new(),
        module_probes: HashSet::new(),
        typeof_names: HashSet::new(),
        require_selections: HashSet::new(),
        wrapper_operations: HashSet::new(),
        eval_operations: HashSet::new(),
        operation: false,
        errors: Vec::new(),
    };
    policy.visit_program(program);
    let requires: Vec<_> = facts
        .dependencies()
        .iter()
        .filter(|d| d.kind() == DependencyKind::Require)
        .collect();
    if !policy.commonjs {
        for dependency in &requires {
            if dependency.pattern().is_some() {
                policy.reject(
                    dependency.span(),
                    "unsupported ESM require context (expected a literal request)",
                );
            }
        }
    }
    for reference in facts.references() {
        if reference.binding().is_some() || reference.type_only() {
            continue;
        }
        let i = requires.partition_point(|d| d.span().start <= reference.span().start);
        let direct_call = i.checked_sub(1).is_some_and(|i| {
            let d = requires[i];
            reference.span().end <= d.specifier_span().start
                && reference.span().start >= d.span().start
        });
        match reference.name() {
            "require" if !reference.write() && direct_call => {}
            "require" | "module" | "exports"
                if !policy.commonjs
                    && (reference.write()
                        || policy.wrapper_operations.contains(&reference.span().start)
                        || (reference.name() == "require"
                            && !policy.typeof_names.contains(&reference.span().start)
                            && !policy.require_selections.contains(&reference.span().start))) =>
            {
                policy.reject(reference.span(), "mixed ESM/CommonJS wrapper reference");
            }
            "require" if policy.commonjs => {
                if reference.write() || !direct_call {
                    policy.reject(
                        reference.span(),
                        "unsupported require reassignment, alias or indirect use",
                    );
                }
            }
            "module"
                if policy.commonjs
                    && !reference.write()
                    && !policy.module_receivers.contains(&reference.span().start)
                    && !policy.typeof_names.contains(&reference.span().start)
                    && !policy.module_probes.contains(&reference.span().start) =>
            {
                policy.reject(
                    reference.span(),
                    "unsupported module access (expected module.exports or a guarded module detector)",
                );
            }
            "eval"
                if reference.write()
                    || policy.eval_operations.contains(&reference.span().start) =>
            {
                policy.reject(reference.span(), "unsupported unshadowed eval operation");
            }
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
    bundled_imports: HashSet<u32>,
    resource_urls: HashSet<u32>,
    resource_url_depth: bool,
    function_depth: usize,
    arguments_depth: usize,
    wrapper_arguments: HashSet<u32>,
    module_receivers: HashSet<u32>,
    module_probes: HashSet<u32>,
    typeof_names: HashSet<u32>,
    require_selections: HashSet<u32>,
    wrapper_operations: HashSet<u32>,
    eval_operations: HashSet<u32>,
    operation: bool,
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

fn module_identifier(expression: &Expression<'_>) -> Option<u32> {
    match expression.get_inner_expression() {
        Expression::Identifier(id) if id.name == "module" => Some(id.span.start),
        _ => None,
    }
}

fn and_operands<'b, 'a>(
    expression: &'b Expression<'a>,
) -> Option<(&'b Expression<'a>, &'b Expression<'a>)> {
    match expression.get_inner_expression() {
        Expression::LogicalExpression(logical) if logical.operator == LogicalOperator::And => {
            Some((&logical.left, &logical.right))
        }
        _ => None,
    }
}

// Admit only the value-producing detector tail, not arbitrary wrapper escapes
// or member operations. A preceding guard (e.g. freeExports) stays untouched.
fn module_detector(expression: &LogicalExpression<'_>) -> Option<[u32; 3]> {
    if expression.operator != LogicalOperator::And {
        return None;
    }
    let last = module_identifier(&expression.right)?;
    let (prefix, not_node) = and_operands(&expression.left)?;
    let Expression::UnaryExpression(not_node) = not_node.get_inner_expression() else {
        return None;
    };
    if not_node.operator != UnaryOperator::LogicalNot {
        return None;
    }
    let Expression::StaticMemberExpression(member) = not_node.argument.get_inner_expression()
    else {
        return None;
    };
    if member.optional || member.property.name != "nodeType" {
        return None;
    }
    let node = module_identifier(&member.object)?;
    let (guard, truthy) = and_operands(prefix)?;
    let truthy = module_identifier(truthy)?;
    let guard = and_operands(guard).map_or(guard, |(_, right)| right);
    let Expression::BinaryExpression(guard) = guard.get_inner_expression() else {
        return None;
    };
    if !matches!(
        guard.operator,
        BinaryOperator::Equality | BinaryOperator::StrictEquality
    ) {
        return None;
    }
    let is_object = |expression: &Expression<'_>| matches!(expression.get_inner_expression(), Expression::StringLiteral(s) if s.value == "object");
    let is_module_type = |expression: &Expression<'_>| {
        matches!(expression.get_inner_expression(), Expression::UnaryExpression(u)
            if u.operator == UnaryOperator::Typeof && module_identifier(&u.argument).is_some())
    };
    ((is_object(&guard.left) && is_module_type(&guard.right))
        || (is_module_type(&guard.left) && is_object(&guard.right)))
    .then_some([truthy, node, last])
}

impl<'a> Visit<'a> for Policy {
    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        if self.operation && reference.name == "eval" {
            self.eval_operations.insert(reference.span.start);
        }
        if !self.commonjs
            && self.operation
            && matches!(reference.name.as_str(), "require" | "module" | "exports")
        {
            self.wrapper_operations.insert(reference.span.start);
        }
        if self.arguments_depth == 0 && reference.name == "arguments" {
            self.wrapper_arguments.insert(reference.span.start);
        }
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        if expression.operator == UnaryOperator::Typeof {
            if let Expression::Identifier(id) = expression.argument.get_inner_expression() {
                self.typeof_names.insert(id.span.start);
            }
        }
        let saved = self.operation;
        self.operation = expression.operator == UnaryOperator::Delete;
        walk::walk_unary_expression(self, expression);
        self.operation = saved;
    }

    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        let saved = self.operation;
        self.operation = true;
        walk::walk_assignment_target(self, target);
        self.operation = saved;
    }

    fn visit_conditional_expression(&mut self, expression: &ConditionalExpression<'a>) {
        let saved = self.operation;
        if !self.commonjs && !saved {
            for branch in [&expression.consequent, &expression.alternate] {
                if let Expression::Identifier(id) = branch.get_inner_expression() {
                    if id.name == "require" {
                        self.require_selections.insert(id.span.start);
                    }
                }
            }
        }
        self.operation = false;
        self.visit_expression(&expression.test);
        self.operation = saved;
        self.visit_expression(&expression.consequent);
        self.visit_expression(&expression.alternate);
    }

    fn visit_sequence_expression(&mut self, expression: &SequenceExpression<'a>) {
        let saved = self.operation;
        let length = expression.expressions.len();
        for (index, expression) in expression.expressions.iter().enumerate() {
            self.operation = saved && index + 1 == length;
            self.visit_expression(expression);
        }
        self.operation = saved;
    }

    fn visit_binary_expression(&mut self, expression: &BinaryExpression<'a>) {
        let saved = self.operation;
        self.operation = false;
        walk::walk_binary_expression(self, expression);
        self.operation = saved;
    }

    fn visit_logical_expression(&mut self, expression: &LogicalExpression<'a>) {
        if self.commonjs && !self.operation {
            if let Some(references) = module_detector(expression) {
                self.module_probes.extend(references);
            }
        }
        // Either operand can supply the result. Unlike a conditional test or
        // discarded sequence prefix, conservatively check both as operations.
        walk::walk_logical_expression(self, expression);
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        let saved = self.operation;
        self.operation = true;
        walk::walk_update_expression(self, expression);
        self.operation = saved;
    }

    fn visit_call_expression(&mut self, expression: &CallExpression<'a>) {
        let saved = self.operation;
        self.operation = true;
        self.visit_expression(&expression.callee);
        self.operation = false;
        for argument in &expression.arguments {
            self.visit_argument(argument);
        }
        if let Some(arguments) = &expression.type_arguments {
            self.visit_ts_type_parameter_instantiation(arguments);
        }
        self.operation = saved;
    }

    fn visit_new_expression(&mut self, expression: &NewExpression<'a>) {
        let saved = self.operation;
        let saved_url = self.resource_url_depth;
        self.resource_url_depth = self.resource_urls.contains(&expression.span.start);
        self.operation = true;
        self.visit_expression(&expression.callee);
        self.operation = false;
        for argument in &expression.arguments {
            self.visit_argument(argument);
        }
        if let Some(arguments) = &expression.type_arguments {
            self.visit_ts_type_parameter_instantiation(arguments);
        }
        self.operation = saved;
        self.resource_url_depth = saved_url;
    }

    fn visit_tagged_template_expression(&mut self, expression: &TaggedTemplateExpression<'a>) {
        let saved = self.operation;
        self.operation = true;
        self.visit_expression(&expression.tag);
        self.operation = false;
        self.visit_template_literal(&expression.quasi);
        if let Some(arguments) = &expression.type_arguments {
            self.visit_ts_type_parameter_instantiation(arguments);
        }
        self.operation = saved;
    }

    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        let json_attribute = json_import_attribute(declaration).is_some();
        if json_attribute
            && declaration.specifiers.as_ref().is_some_and(|specifiers| {
                specifiers.iter().any(|specifier| match specifier {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => false,
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                        specifier.imported.name() != "default"
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => true,
                })
            })
        {
            self.reject(
                declaration.span,
                "JSON import attributes support only default or side-effect imports",
            );
        }
        self.module_declaration(
            declaration.span,
            declaration.with_clause.is_some() && !json_attribute,
            declaration.phase.is_some(),
        );
        walk::walk_import_declaration(self, declaration);
    }

    fn visit_export_named_declaration(&mut self, declaration: &ExportNamedDeclaration<'a>) {
        self.module_declaration(declaration.span, false, false);
        walk::walk_export_named_declaration(self, declaration);
    }

    fn visit_export_from_declaration(&mut self, declaration: &ExportFromDeclaration<'a>) {
        self.module_declaration(declaration.span, declaration.with_clause.is_some(), false);
        walk::walk_export_from_declaration(self, declaration);
    }

    fn visit_export_declaration(&mut self, declaration: &ExportDeclaration<'a>) {
        self.module_declaration(declaration.span, false, false);
        walk::walk_export_declaration(self, declaration);
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
        if (expression.options.is_some() || expression.phase.is_some())
            && self.bundled_imports.contains(&expression.span.start)
        {
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
        self.visit_expression(&member.object);
        let saved = self.operation;
        self.operation = false;
        self.visit_expression(&member.expression);
        self.operation = saved;
    }

    fn visit_with_statement(&mut self, statement: &WithStatement<'a>) {
        self.reject(statement.span, "unsupported with statement");
    }

    fn visit_import_meta(&mut self, property: &ImportMeta) {
        if !self.resource_url_depth {
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
        let saved = self.operation;
        self.operation = false;
        self.function_depth += 1;
        self.arguments_depth += 1;
        walk::walk_function(self, function, flags);
        self.arguments_depth -= 1;
        self.function_depth -= 1;
        self.operation = saved;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let saved = self.operation;
        self.operation = false;
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, function);
        self.function_depth -= 1;
        self.operation = saved;
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

    fn visit_ts_namespace_declaration(&mut self, declaration: &TSNamespaceDeclaration<'a>) {
        self.reject(
            declaration.span,
            "unsupported TypeScript namespace transform",
        );
    }

    fn visit_ts_external_module_declaration(
        &mut self,
        declaration: &TSExternalModuleDeclaration<'a>,
    ) {
        self.reject(
            declaration.span,
            "unsupported TypeScript namespace transform",
        );
    }

    fn visit_ts_global_declaration(&mut self, declaration: &TSGlobalDeclaration<'a>) {
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
