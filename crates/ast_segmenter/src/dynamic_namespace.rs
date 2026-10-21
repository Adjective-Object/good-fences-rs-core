//! Conservative observation shapes for namespaces produced by literal import().
use std::collections::HashMap;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::GetSpan;

use crate::emission::DynamicNamespace;
use crate::owned::{BindingId, DependencyKind, ExportValue, ModuleFacts};

pub(super) fn extract(program: &Program<'_>, facts: &ModuleFacts) -> (Vec<DynamicNamespace>, bool) {
    let mut visitor = Observations {
        facts,
        imports: HashMap::new(),
        reads: HashMap::new(),
        blocked: false,
        receiver: None,
        uses_this: false,
    };
    visitor.visit_program(program);
    let observations = facts
        .dependencies()
        .iter()
        .filter(|d| d.kind() == DependencyKind::DynamicImport)
        .map(|d| DynamicNamespace {
            dependency: d.id(),
            properties: visitor
                .imports
                .remove(&d.id().0)
                .and_then(|(mut properties, binding)| {
                    if let Some(binding) = binding {
                        if facts
                            .exports()
                            .iter()
                            .any(|e| *e.value() == ExportValue::Binding(binding))
                        {
                            return None;
                        }
                        for reference in facts
                            .references()
                            .iter()
                            .filter(|r| !r.type_only() && r.binding() == Some(binding))
                        {
                            if reference.write() {
                                return None;
                            }
                            properties.push(visitor.reads.get(&reference.span().start)?.clone());
                        }
                    }
                    properties.sort();
                    properties.dedup();
                    Some(properties)
                }),
        })
        .collect();
    (observations, visitor.uses_this)
}

type NamespaceBinding = (Vec<(String, bool)>, Option<BindingId>);

struct Observations<'f> {
    facts: &'f ModuleFacts,
    imports: HashMap<u32, NamespaceBinding>,
    reads: HashMap<u32, (String, bool)>,
    blocked: bool,
    receiver: Option<u32>,
    uses_this: bool,
}

fn awaited<'b, 'a>(expression: &'b Expression<'a>) -> Option<&'b ImportExpression<'a>> {
    let Expression::AwaitExpression(awaited) = expression.get_inner_expression() else {
        return None;
    };
    if let Expression::ImportExpression(import) = awaited.argument.get_inner_expression() {
        Some(import)
    } else {
        None
    }
}

impl Observations<'_> {
    fn pattern(&mut self, import: &ImportExpression<'_>, pattern: &BindingPattern<'_>) {
        let observation = match pattern {
            BindingPattern::BindingIdentifier(id) => {
                let Some(binding) = self
                    .facts
                    .bindings()
                    .iter()
                    .find(|b| b.declarations().contains(&id.span))
                else {
                    return;
                };
                (Vec::new(), Some(binding.id()))
            }
            BindingPattern::ObjectPattern(object) if object.rest.is_none() => {
                let mut properties = Vec::new();
                for property in &object.properties {
                    if property.computed
                        || !matches!(property.value, BindingPattern::BindingIdentifier(_))
                    {
                        return;
                    }
                    let Some(name) = property.key.static_name() else {
                        return;
                    };
                    properties.push((name.to_string(), false));
                }
                (properties, None)
            }
            _ => return,
        };
        self.imports.insert(import.span.start, observation);
    }
}

impl<'a> Visit<'a> for Observations<'_> {
    fn visit_this_expression(&mut self, _: &ThisExpression) {
        self.uses_this = true;
    }
    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        if declaration.kind == VariableDeclarationKind::Const {
            for declarator in &declaration.declarations {
                if let Some(import) = declarator.init.as_ref().and_then(awaited) {
                    self.pattern(import, &declarator.id);
                }
            }
        }
        walk::walk_variable_declaration(self, declaration);
    }

    fn visit_expression_statement(&mut self, statement: &ExpressionStatement<'a>) {
        if let Some(import) = awaited(&statement.expression) {
            self.imports.insert(import.span.start, (Vec::new(), None));
        } else if let Expression::ImportExpression(import) =
            statement.expression.get_inner_expression()
        {
            self.imports.insert(import.span.start, (Vec::new(), None));
        }
        walk::walk_expression_statement(self, statement);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if !self.blocked {
            if let Expression::Identifier(id) = member.object.get_inner_expression() {
                self.reads.insert(
                    id.span.start,
                    (
                        member.property.name.to_string(),
                        self.receiver == Some(member.span.start),
                    ),
                );
            } else if let Some(import) = awaited(&member.object) {
                self.imports.insert(
                    import.span.start,
                    (
                        vec![(
                            member.property.name.to_string(),
                            self.receiver == Some(member.span.start),
                        )],
                        None,
                    ),
                );
            }
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::StaticMemberExpression(member) = call.callee.get_inner_expression() {
            if member.property.name == "then" && call.arguments.len() == 1 {
                if let Expression::ImportExpression(import) = member.object.get_inner_expression() {
                    if let Argument::ArrowFunctionExpression(arrow) = &call.arguments[0] {
                        if arrow.params.items.len() == 1 && arrow.params.rest.is_none() {
                            self.pattern(import, &arrow.params.items[0].pattern);
                        }
                    }
                }
            }
        }
        let saved = self
            .receiver
            .replace(call.callee.get_inner_expression().span().start);
        self.visit_expression(&call.callee);
        self.receiver = saved;
        for argument in &call.arguments {
            self.visit_argument(argument);
        }
    }

    fn visit_tagged_template_expression(&mut self, expression: &TaggedTemplateExpression<'a>) {
        let saved = std::mem::replace(&mut self.blocked, true);
        self.visit_expression(&expression.tag);
        self.blocked = saved;
        self.visit_template_literal(&expression.quasi);
    }
    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        let saved = std::mem::replace(&mut self.blocked, true);
        walk::walk_assignment_target(self, target);
        self.blocked = saved;
    }

    fn visit_update_expression(&mut self, expression: &UpdateExpression<'a>) {
        let saved = std::mem::replace(&mut self.blocked, true);
        walk::walk_update_expression(self, expression);
        self.blocked = saved;
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        let saved = self.blocked;
        self.blocked |= expression.operator == UnaryOperator::Delete;
        walk::walk_unary_expression(self, expression);
        self.blocked = saved;
    }
}
