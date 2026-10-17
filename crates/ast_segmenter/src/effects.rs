//! Conservative syntax facts, not user-policy-derived purity conclusions.
#[cfg(test)]
#[path = "effects_tests.rs"]
mod tests;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::ScopeFlags;
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Serialize};

use crate::owned::ModuleFacts;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EffectReason {
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
    OpaqueCommonJs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectSite {
    #[serde(with = "crate::owned::span_serde")]
    pub span: Span,
    pub reason: EffectReason,
    /// None means module evaluation; otherwise execution waits for this function
    /// or instance field initializer. Lexical containment is not a call graph.
    #[serde(with = "crate::owned::span_serde::optional")]
    pub deferred: Option<Span>,
}

pub(crate) fn extract(program: &Program<'_>, facts: &ModuleFacts) -> Vec<EffectSite> {
    let mut visitor = Effects {
        facts,
        sites: Vec::new(),
        deferred: None,
    };
    if program.source_type.is_commonjs() {
        visitor.add(
            Span::new(
                program.hashbang.as_ref().map_or(0, |h| h.span.end),
                program.span.end,
            ),
            EffectReason::OpaqueCommonJs,
        );
    } else {
        visitor.visit_program(program);
    }
    visitor
        .sites
        .sort_by_key(|s| (s.span.start, s.span.end, s.reason));
    visitor.sites.dedup();
    visitor.sites
}

struct Effects<'f> {
    facts: &'f ModuleFacts,
    sites: Vec<EffectSite>,
    deferred: Option<Span>,
}

impl Effects<'_> {
    fn add(&mut self, span: Span, reason: EffectReason) {
        self.sites.push(EffectSite {
            span,
            reason,
            deferred: self.deferred,
        });
    }
}

impl<'a> Visit<'a> for Effects<'_> {
    fn visit_expression(&mut self, expression: &Expression<'a>) {
        use EffectReason::*;
        let reason = match expression {
            Expression::BinaryExpression(_) | Expression::UnaryExpression(_) => Some(Coercion),
            Expression::TemplateLiteral(t) if !t.expressions.is_empty() => Some(Coercion),
            Expression::AssignmentExpression(_) | Expression::UpdateExpression(_) => Some(Mutation),
            Expression::BooleanLiteral(_)
            | Expression::CallExpression(_)
            | Expression::NewExpression(_)
            | Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
            | Expression::PrivateFieldExpression(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::Identifier(_)
            | Expression::ThisExpression(_)
            | Expression::ArrayExpression(_)
            | Expression::ObjectExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_)
            | Expression::ConditionalExpression(_)
            | Expression::LogicalExpression(_)
            | Expression::SequenceExpression(_)
            | Expression::ParenthesizedExpression(_)
            | Expression::ChainExpression(_)
            | Expression::TSAsExpression(_)
            | Expression::TSSatisfiesExpression(_)
            | Expression::TSTypeAssertion(_)
            | Expression::TSNonNullExpression(_)
            | Expression::TSInstantiationExpression(_) => None,
            _ => Some(UnknownExpression),
        };
        if let Some(reason) = reason {
            self.add(expression.span(), reason);
        }
        walk::walk_expression(self, expression);
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        let Ok(index) = self
            .facts
            .references
            .binary_search_by_key(&(identifier.span.start, identifier.span.end), |r| {
                (r.span.start, r.span.end)
            })
        else {
            return;
        };
        let reference = &self.facts.references[index];
        if reference.type_only || !reference.read {
            return;
        }
        if let Some(binding) = reference.binding {
            // Even a lexical local can throw in its TDZ. Hoisted function reads cannot.
            if !self
                .facts
                .bindings
                .binary_search_by_key(&binding.0, |b| b.id.0)
                .is_ok_and(|i| self.facts.bindings[i].function)
            {
                self.add(identifier.span, EffectReason::BindingRead);
            }
        } else {
            self.add(identifier.span, EffectReason::UnresolvedRead);
        }
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        self.add(call.span, EffectReason::Invocation);
        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, call: &NewExpression<'a>) {
        self.add(call.span, EffectReason::Construction);
        walk::walk_new_expression(self, call);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        self.add(member.span, EffectReason::PropertyRead);
        walk::walk_static_member_expression(self, member);
    }

    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        self.add(member.span, EffectReason::PropertyRead);
        walk::walk_computed_member_expression(self, member);
    }

    fn visit_private_field_expression(&mut self, member: &PrivateFieldExpression<'a>) {
        self.add(member.span, EffectReason::PropertyRead);
        walk::walk_private_field_expression(self, member);
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::EmptyStatement(_)
            | Statement::BlockStatement(_)
            | Statement::ExpressionStatement(_)
            | Statement::VariableDeclaration(_)
            | Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportDefaultDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_) => {}
            _ => self.add(statement.span(), EffectReason::UnknownControl),
        }
        walk::walk_statement(self, statement);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        let previous = self.deferred.replace(function.span);
        walk::walk_function(self, function, flags);
        self.deferred = previous;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let previous = self.deferred.replace(function.span);
        walk::walk_arrow_function_expression(self, function);
        self.deferred = previous;
    }

    fn visit_spread_element(&mut self, spread: &SpreadElement<'a>) {
        self.add(spread.span, EffectReason::SpreadOrIteration);
        walk::walk_spread_element(self, spread);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        self.add(statement.span, EffectReason::SpreadOrIteration);
        walk::walk_for_of_statement(self, statement);
    }

    fn visit_binding_pattern(&mut self, pattern: &BindingPattern<'a>) {
        if matches!(
            pattern,
            BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
        ) {
            self.add(pattern.span(), EffectReason::Destructuring);
        }
        walk::walk_binding_pattern(self, pattern);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if class.declare {
            return;
        }
        if let Some(super_class) = &class.super_class {
            self.add(super_class.span(), EffectReason::ClassHeritage);
        }
        walk::walk_class(self, class);
    }

    fn visit_property_key(&mut self, key: &PropertyKey<'a>) {
        if key.is_expression() {
            self.add(key.span(), EffectReason::ComputedKey);
        }
        walk::walk_property_key(self, key);
    }

    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        if property.declare {
            return;
        }
        self.visit_property_key(&property.key);
        if let Some(value) = &property.value {
            let previous = self.deferred;
            if !property.r#static {
                self.deferred = Some(value.span());
            }

            self.visit_expression(value);
            self.deferred = previous;
        }
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.add(block.span, EffectReason::StaticBlock);
        walk::walk_static_block(self, block);
    }
}
