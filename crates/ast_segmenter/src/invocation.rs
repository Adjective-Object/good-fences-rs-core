//! Owned invocation inputs and precise syntax annotations for later rewriting.
use oxc_ast::{ast::*, Comment};
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Serialize};

use crate::owned::{BindingId, ModuleFacts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueUse {
    Discarded,
    /// A variable initializer/default export; demand for its statement keeps the result.
    StatementValue,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifierCallee {
    pub name: String,
    pub binding: Option<BindingId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalleeShape {
    Identifier(IdentifierCallee),
    Member {
        #[serde(with = "crate::owned::span_serde")]
        receiver: Span,
        receiver_identifier: Option<IdentifierCallee>,
        /// Static name or computed string literal, not an inferred runtime key.
        property: Option<String>,
        computed: bool,
    },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationArgument {
    #[serde(with = "crate::owned::span_serde")]
    pub span: Span,
    /// True requires iteration, not just evaluation of the iterable expression.
    pub spread: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    #[serde(with = "crate::owned::span_serde")]
    pub span: Span,
    #[serde(with = "crate::owned::span_serde")]
    pub callee: Span,
    pub shape: CalleeShape,
    pub arguments: Vec<InvocationArgument>,
    pub construct: bool,
    pub optional_call: bool,
    /// The callee's original chain can short-circuit before argument evaluation.
    pub optional_chain: bool,
    #[serde(with = "crate::owned::span_serde::optional")]
    pub pure: Option<Span>,
    #[serde(with = "crate::owned::span_serde::optional")]
    pub deferred: Option<Span>,
    pub value_use: ValueUse,
}

pub(crate) fn identifier(
    expression: &Expression<'_>,
    facts: &ModuleFacts,
) -> Option<IdentifierCallee> {
    let Expression::Identifier(id) = expression.get_inner_expression() else {
        return None;
    };
    let binding = facts
        .references
        .binary_search_by_key(&(id.span.start, id.span.end), |r| {
            (r.span.start, r.span.end)
        })
        .ok()
        .and_then(|i| facts.references[i].binding);
    Some(IdentifierCallee {
        name: id.name.to_string(),
        binding,
    })
}

pub(crate) fn shape(callee: &Expression<'_>, facts: &ModuleFacts) -> CalleeShape {
    let member = match callee.get_inner_expression() {
        Expression::StaticMemberExpression(m) => {
            Some((&m.object, Some(m.property.name.to_string()), false))
        }
        Expression::ComputedMemberExpression(m) => Some((
            &m.object,
            match m.expression.get_inner_expression() {
                Expression::StringLiteral(s) => Some(s.value.to_string()),
                _ => None,
            },
            true,
        )),
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::StaticMemberExpression(m) => {
                Some((&m.object, Some(m.property.name.to_string()), false))
            }
            ChainElement::ComputedMemberExpression(m) => Some((
                &m.object,
                match m.expression.get_inner_expression() {
                    Expression::StringLiteral(s) => Some(s.value.to_string()),
                    _ => None,
                },
                true,
            )),
            _ => None,
        },
        _ => None,
    };
    if let Some((receiver, property, computed)) = member {
        CalleeShape::Member {
            receiver: receiver.span(),
            receiver_identifier: identifier(receiver, facts),
            property,
            computed,
        }
    } else {
        identifier(callee, facts).map_or(CalleeShape::Other, CalleeShape::Identifier)
    }
}

pub(crate) fn optional_chain(expression: &Expression<'_>) -> bool {
    // Parenthesized/finished chains are values, not short-circuit guards for outer calls.
    match expression {
        Expression::StaticMemberExpression(m) => m.optional || optional_chain(&m.object),
        Expression::ComputedMemberExpression(m) => m.optional || optional_chain(&m.object),
        Expression::PrivateFieldExpression(m) => m.optional || optional_chain(&m.object),
        Expression::CallExpression(c) => c.optional || optional_chain(&c.callee),
        _ => false,
    }
}

pub(crate) fn arguments(arguments: &[Argument<'_>]) -> Vec<InvocationArgument> {
    arguments
        .iter()
        .map(|argument| match argument {
            Argument::SpreadElement(spread) => InvocationArgument {
                span: spread.argument.span(),
                spread: true,
            },
            _ => InvocationArgument {
                span: argument.span(),
                spread: false,
            },
        })
        .collect()
}

pub(crate) fn pure_comment(
    comments: &[Comment],
    source: &str,
    start: u32,
    parser_pure: bool,
) -> Option<Span> {
    if !parser_pure {
        return None;
    }
    let end = comments.partition_point(|c| c.span.end <= start);
    let mut boundary = start as usize;
    for comment in comments[..end].iter().rev() {
        let gap = source.get(comment.span.end as usize..boundary)?;
        if !gap.chars().all(|c| c.is_whitespace() || c == '(') {
            break;
        }
        let text = source.get(comment.span.start as usize..comment.span.end as usize)?;
        let marker = text
            .strip_prefix("/*")
            .and_then(|s| s.strip_suffix("*/"))
            .map(str::trim);
        if matches!(marker, Some("@__PURE__" | "#__PURE__")) {
            return Some(comment.span);
        }
        boundary = comment.span.start as usize;
    }
    None
}
