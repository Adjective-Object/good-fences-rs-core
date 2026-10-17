//! Policy-derived statement effects and source-preserving, non-emitting recipes.
use crate::policy::EffectPolicy;
use ast_segmenter::{
    effects::{EffectReason, EffectSite},
    invocation::{CalleeShape, Invocation, InvocationArgument, ValueUse},
    owned::ModuleFacts,
};
use oxc_span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentGuard {
    Always,
    /// Preserve the original callee chain's completion state separately from its
    /// value. `undefined` returned by a getter is not chain short-circuiting.
    OriginalChain {
        optional_call: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationRewrite {
    /// Replace this invocation only, never treat its enclosing statement as pure.
    pub invocation: Span,
    /// Evaluate the original callee/reference exactly once, including receiver,
    /// getters and computed keys; preserve its optional-chain completion state.
    pub callee: Span,
    pub reference: CalleeShape,
    /// Then evaluate arguments left-to-right under this original guard. A spread
    /// must consume its iterator, not merely evaluate the iterable expression.
    pub arguments: Vec<InvocationArgument>,
    pub guard: ArgumentGuard,
    pub construct: bool,
    pub value_use: ValueUse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementEffects {
    pub span: Span,
    pub sites: Vec<EffectSite>,
    pub rewrites: Vec<InvocationRewrite>,
    pub known_effects: Vec<KnownEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownEffect {
    pub invocation: Span,
    /// Canonical structural rule, deterministic even if configuration order changes.
    pub pattern: String,
}

impl StatementEffects {
    pub fn is_effectful(&self) -> bool {
        !self.sites.is_empty() || !self.known_effects.is_empty()
    }
}

pub(crate) fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn removable(call: &Invocation, value_demanded: bool, policy: &EffectPolicy) -> bool {
    call.deferred.is_none()
        && call.pure.is_some()
        && policy.honors_pure_annotations()
        && policy.matching_patterns(call).next().is_none()
        && match call.value_use {
            ValueUse::Discarded => true,
            ValueUse::StatementValue => !value_demanded,
            ValueUse::Required => false,
        }
}

pub fn summarize(facts: &ModuleFacts, statement: Span, value_demanded: bool) -> StatementEffects {
    summarize_with_policy(facts, statement, value_demanded, &EffectPolicy::default())
}

pub fn summarize_with_policy(
    facts: &ModuleFacts,
    statement: Span,
    value_demanded: bool,
    policy: &EffectPolicy,
) -> StatementEffects {
    let first = facts
        .invocations
        .partition_point(|c| c.span.start < statement.start);
    let calls: Vec<_> = facts.invocations[first..]
        .iter()
        .take_while(|c| c.span.start <= statement.end)
        .filter(|c| contains(statement, c.span) && removable(c, value_demanded, policy))
        .collect();
    let start = facts
        .effects
        .partition_point(|s| s.span.start < statement.start);
    let sites = facts.effects[start..]
        .iter()
        .take_while(|s| s.span.start <= statement.end)
        .filter(|s| s.deferred.is_none() && contains(statement, s.span))
        .filter(|s| {
            !matches!(
                s.reason,
                EffectReason::Invocation | EffectReason::Construction
            ) || !calls.iter().any(|c| c.span == s.span)
        })
        .cloned()
        .collect();
    let rewrites = calls
        .into_iter()
        .map(|call| InvocationRewrite {
            invocation: call.span,
            callee: call.callee,
            reference: call.shape.clone(),
            arguments: call.arguments.clone(),
            guard: if call.optional_call || call.optional_chain {
                ArgumentGuard::OriginalChain {
                    optional_call: call.optional_call,
                }
            } else {
                ArgumentGuard::Always
            },
            construct: call.construct,
            value_use: call.value_use,
        })
        .collect();
    let known_effects = facts.invocations[first..]
        .iter()
        .take_while(|c| c.span.start <= statement.end)
        .filter(|c| c.deferred.is_none() && contains(statement, c.span))
        .flat_map(|c| {
            policy.matching_patterns(c).map(move |pattern| KnownEffect {
                invocation: c.span,
                pattern: pattern.to_owned(),
            })
        })
        .collect();
    StatementEffects {
        span: statement,
        sites,
        rewrites,
        known_effects,
    }
}
