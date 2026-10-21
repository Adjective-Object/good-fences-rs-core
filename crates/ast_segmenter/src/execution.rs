//! Owned lexical execution contexts, not an interprocedural effect or call solver.
use std::collections::HashMap;

use eyre::OptionExt;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Serialize};

use crate::owned::ModuleFacts;

mod records;
pub use records::*;

impl ExecutionFacts {
    /// Resolve the effect's recorded owner, then follow immediate-call ancestry.
    pub fn effect_is_module_evaluation(
        &self,
        site: &crate::effects::EffectSite,
    ) -> eyre::Result<bool> {
        let context = site
            .deferred()
            .map(|span| {
                // The operand can itself be a callable literal. Its context is
                // not the context of the operation evaluating/coercing it.
                self.contexts()
                    .iter()
                    .rposition(|context| context.span() == span)
                    .map(|index| index as u32)
                    .ok_or_eyre("missing recorded effect execution context")
            })
            .transpose()?;
        Ok(self.is_module_evaluation(context))
    }

    pub fn is_module_evaluation(&self, mut context: Option<u32>) -> bool {
        while let Some(index) = context {
            let current = &self.contexts()[index as usize];
            if current.immediate().is_none() {
                return false;
            }
            context = current.parent();
        }
        true
    }

    pub fn context_at(&self, span: Span) -> Option<u32> {
        self.contexts()
            .iter()
            .rposition(|c| c.span().start <= span.start && span.end <= c.span().end)
            .map(|i| i as u32)
    }
}

pub(crate) fn extract(program: &Program<'_>, facts: &ModuleFacts) -> ExecutionFacts {
    if facts.commonjs() {
        return ExecutionFacts::default();
    }
    let mut visitor = Contexts {
        facts,
        result: ExecutionFacts::default(),
        parent: None,
        immediate: HashMap::new(),
    };
    visitor.visit_program(program);
    let mut result = visitor.result;
    *result.references_mut() = facts
        .references()
        .iter()
        .map(|reference| result.context_at(reference.span()))
        .collect();
    *result.invocations_mut() = facts
        .invocations()
        .iter()
        .map(|call| result.context_at(call.span()))
        .collect();
    result
}

struct Contexts<'f> {
    facts: &'f ModuleFacts,
    result: ExecutionFacts,
    parent: Option<u32>,
    immediate: HashMap<u32, u32>,
}

impl Contexts<'_> {
    fn enter(&mut self, span: Span, kind: ContextKind, synchronous: bool) -> Option<u32> {
        let parent = self.parent;
        self.parent = Some(self.result.contexts().len() as u32);
        let immediate = synchronous
            .then(|| self.immediate.get(&span.start).copied())
            .flatten();
        self.result
            .contexts_mut()
            .push(ExecutionContext::new(span, parent, kind, immediate));
        parent
    }
}

impl<'a> Visit<'a> for Contexts<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if matches!(
            call.callee.get_inner_expression(),
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
        ) {
            if let Ok(index) = self
                .facts
                .invocations()
                .binary_search_by_key(&(call.span.start, call.span.end), |c| {
                    (c.span().start, c.span().end)
                })
            {
                self.immediate.insert(
                    call.callee.get_inner_expression().span().start,
                    index as u32,
                );
            }
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        let parent = self.enter(
            function.span,
            ContextKind::Callable,
            !function.r#async && !function.generator,
        );
        walk::walk_function(self, function, flags);
        self.parent = parent;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        let parent = self.enter(function.span, ContextKind::Callable, !function.r#async);
        walk::walk_arrow_function_expression(self, function);
        self.parent = parent;
    }

    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        if property.declare {
            return;
        }
        self.visit_property_key(&property.key);
        if let Some(value) = &property.value {
            if property.r#static {
                self.visit_expression(value);
            } else {
                let parent = self.enter(value.span(), ContextKind::InstanceInitializer, false);
                self.visit_expression(value);
                self.parent = parent;
            }
        }
    }
}
