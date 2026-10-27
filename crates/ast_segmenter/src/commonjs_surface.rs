//! Syntax-local potential names, not an export evaluator or recovery policy.
use std::collections::{BTreeMap, BTreeSet};

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::{GetSpan, Span};

use crate::owned::{
    CommonJsExportAccess, CommonJsExportName, CommonJsGlobal, CommonJsSurface, ModuleFacts,
};

pub(crate) fn extract(program: &Program<'_>, facts: &ModuleFacts) -> CommonJsSurface {
    if !facts.commonjs() {
        return CommonJsSurface::default();
    }
    let mut visitor = Surface {
        facts,
        references: facts
            .references()
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                matches!(
                    r.commonjs(),
                    Some(CommonJsGlobal::Module | CommonJsGlobal::Exports)
                )
            })
            .map(|(i, r)| (r.span().start, i as u32))
            .collect(),
        accesses: BTreeMap::new(),
        unknown: BTreeSet::new(),
        discarded_assignments: BTreeSet::new(),
    };
    visitor.visit_program(program);
    for (index, reference) in facts.references().iter().enumerate() {
        if (reference.commonjs().is_some() && !visitor.accesses.contains_key(&(index as u32)))
            || (reference.binding().is_none() && reference.name() == "arguments")
        {
            visitor.unknown(reference.span());
        }
    }
    // Dependencies may expose the partially initialized object through a cycle.
    for dependency in facts.dependencies() {
        visitor.unknown(dependency.span());
    }
    for violation in facts.policy_violations() {
        visitor.unknown(violation.span());
    }
    for binding in facts
        .bindings()
        .iter()
        .filter(|b| b.top_level() && b.name() == "module")
    {
        for &declaration in binding.declarations() {
            visitor.unknown(declaration);
        }
    }
    CommonJsSurface::new(
        visitor.accesses.into_values().collect(),
        visitor
            .unknown
            .into_iter()
            .map(|(start, end)| Span::new(start, end))
            .collect(),
    )
}

struct Surface<'f> {
    facts: &'f ModuleFacts,
    references: BTreeMap<u32, u32>,
    accesses: BTreeMap<u32, CommonJsExportAccess>,
    unknown: BTreeSet<(u32, u32)>,
    discarded_assignments: BTreeSet<(u32, u32)>,
}

fn member<'s, 'a>(expression: &'s Expression<'a>) -> Option<(&'s Expression<'a>, Span, String)> {
    match expression.get_inner_expression() {
        Expression::StaticMemberExpression(m) if !m.optional => {
            Some((&m.object, m.property.span, m.property.name.to_string()))
        }
        Expression::ComputedMemberExpression(m) if !m.optional => {
            let Expression::StringLiteral(name) = m.expression.get_inner_expression() else {
                return None;
            };
            Some((&m.object, name.span, name.value.as_str()?.to_owned()))
        }
        _ => None,
    }
}

impl Surface<'_> {
    fn unknown(&mut self, span: Span) {
        self.unknown.insert((span.start, span.end));
    }

    fn identifier(&self, expression: &Expression<'_>, global: CommonJsGlobal) -> Option<u32> {
        let Expression::Identifier(identifier) = expression.get_inner_expression() else {
            return None;
        };
        let index = *self.references.get(&identifier.span.start)?;
        (self.facts.references()[index as usize].commonjs() == Some(global)).then_some(index)
    }

    fn module_exports(&self, expression: &Expression<'_>) -> Option<u32> {
        let (object, _, name) = member(expression)?;
        (name == "exports")
            .then(|| self.identifier(object, CommonJsGlobal::Module))
            .flatten()
    }

    fn root(&self, expression: &Expression<'_>) -> Option<u32> {
        self.identifier(expression, CommonJsGlobal::Exports)
            .or_else(|| self.module_exports(expression))
    }

    fn property(&mut self, object: &Expression<'_>, span: Span, key: Span, name: String) {
        if let Some(reference) = self.root(object) {
            // __proto__ can expose an arbitrary inherited surface, even without a root escape.
            if name == "__proto__" {
                self.unknown(span);
            }
            self.accesses.insert(
                reference,
                CommonJsExportAccess::new(
                    reference,
                    span,
                    false,
                    vec![CommonJsExportName::new(name, key)],
                ),
            );
        }
    }
}

impl<'a> Visit<'a> for Surface<'_> {
    fn visit_expression_statement(&mut self, statement: &ExpressionStatement<'a>) {
        if let Expression::AssignmentExpression(assignment) =
            statement.expression.get_inner_expression()
        {
            self.discarded_assignments
                .insert((assignment.span.start, assignment.span.end));
        }
        walk::walk_expression_statement(self, statement);
    }

    fn visit_static_member_expression(&mut self, expression: &StaticMemberExpression<'a>) {
        if !expression.optional {
            self.property(
                &expression.object,
                expression.span,
                expression.property.span,
                expression.property.name.to_string(),
            );
        }
        walk::walk_static_member_expression(self, expression);
    }

    fn visit_computed_member_expression(&mut self, expression: &ComputedMemberExpression<'a>) {
        if !expression.optional {
            if let Expression::StringLiteral(name) = expression.expression.get_inner_expression() {
                if let Some(name_value) = name.value.as_str() {
                    self.property(
                        &expression.object,
                        expression.span,
                        name.span,
                        name_value.to_owned(),
                    );
                }
            }
        }
        walk::walk_computed_member_expression(self, expression);
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        let root = match &expression.left {
            AssignmentTarget::StaticMemberExpression(m)
                if !m.optional && m.property.name == "exports" =>
            {
                self.identifier(&m.object, CommonJsGlobal::Module)
            }
            AssignmentTarget::ComputedMemberExpression(m) if !m.optional => {
                match m.expression.get_inner_expression() {
                    Expression::StringLiteral(name) if name.value.as_str() == Some("exports") => {
                        self.identifier(&m.object, CommonJsGlobal::Module)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(reference) = root {
            let mut names = Vec::new();
            let mut plain = expression.operator.is_assign()
                && self
                    .discarded_assignments
                    .contains(&(expression.span.start, expression.span.end));
            if let Expression::ObjectExpression(object) = expression.right.get_inner_expression() {
                for property in &object.properties {
                    let ObjectPropertyKind::ObjectProperty(property) = property else {
                        plain = false;
                        continue;
                    };
                    let name = match &property.key {
                        PropertyKey::StaticIdentifier(key) if !property.computed => {
                            Some(key.name.to_string())
                        }
                        PropertyKey::StringLiteral(key) => key.value.as_str().map(str::to_owned),
                        _ => None,
                    };
                    if property.kind != PropertyKind::Init {
                        plain = false;
                    }
                    if let Some(name) = name {
                        if name == "__proto__" && !property.computed {
                            plain = false;
                        }
                        names.push(CommonJsExportName::new(name, property.key.span()));
                    } else {
                        plain = false;
                    }
                }
            } else {
                plain = false;
            }
            if plain {
                self.accesses.insert(
                    reference,
                    CommonJsExportAccess::new(reference, expression.span, true, names),
                );
            } else {
                self.unknown(expression.span);
            }
        }
        walk::walk_assignment_expression(self, expression);
    }

    fn visit_call_expression(&mut self, expression: &CallExpression<'a>) {
        if member(&expression.callee).is_some_and(|(object, _, _)| self.root(object).is_some()) {
            self.unknown(expression.callee.span());
        }
        walk::walk_call_expression(self, expression);
    }

    fn visit_tagged_template_expression(&mut self, expression: &TaggedTemplateExpression<'a>) {
        if member(&expression.tag).is_some_and(|(object, _, _)| self.root(object).is_some()) {
            self.unknown(expression.tag.span());
        }
        walk::walk_tagged_template_expression(self, expression);
    }

    fn visit_this_expression(&mut self, expression: &ThisExpression) {
        self.unknown(expression.span);
    }
}
