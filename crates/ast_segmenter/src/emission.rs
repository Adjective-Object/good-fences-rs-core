//! Owned source-local recipes. Names, retention, targets and transform policy stay derived.
use std::collections::HashMap;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Serialize};

use crate::owned::{BindingId, DependencyId, ExportValue, ModuleFacts};

#[cfg(test)]
#[path = "emission_tests.rs"]
mod tests;

mod records;
pub use records::*;

#[path = "dynamic_namespace.rs"]
mod dynamic_namespace;

impl OperationKind {
    /// Resolve ranges through existing occurrence IDs wherever possible.
    pub fn span(&self, facts: &ModuleFacts, recipe: &SegmentRecipe) -> Span {
        match self {
            Self::Binding {
                binding,
                declaration,
                ..
            } => facts
                .bindings()
                .iter()
                .find(|b| b.id() == *binding)
                .expect("validated binding")
                .declarations()[*declaration as usize],
            Self::Reference { reference, .. } => facts.references()[*reference as usize].span(),
            Self::Namespace { usage } => match recipe.namespaces()[*usage as usize].access() {
                NamespaceAccess::DirectRead { member, .. } => *member,
                NamespaceAccess::Native(_) => unreachable!("only direct reads have operations"),
            },
            Self::Dependency { dependency } => facts
                .dependencies()
                .iter()
                .find(|d| d.id() == *dependency)
                .expect("validated dependency")
                .span(),
            Self::Export { span, .. } => *span,
            Self::SourceHint { span } => *span,
            Self::Invocation { invocation, .. } => facts.invocations()[*invocation as usize].span(),
        }
    }
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn slot(span: Span) -> SourceSlot {
    SourceSlot::new(span, Vec::new())
}

fn chain(expression: &Expression<'_>) -> Option<CalleePlan> {
    let (object, step) = match expression {
        Expression::ChainExpression(c) => {
            return match &c.expression {
                ChainElement::StaticMemberExpression(m) => member_chain(
                    &m.object,
                    ChainStep::Static {
                        property: m.property.name.to_string(),
                        optional: m.optional,
                    },
                ),
                ChainElement::ComputedMemberExpression(m) => member_chain(
                    &m.object,
                    ChainStep::Computed {
                        property: slot(m.expression.span()),
                        optional: m.optional,
                    },
                ),
                _ => None,
            }
        }
        Expression::StaticMemberExpression(m) => (
            &m.object,
            ChainStep::Static {
                property: m.property.name.to_string(),
                optional: m.optional,
            },
        ),
        Expression::ComputedMemberExpression(m) => (
            &m.object,
            ChainStep::Computed {
                property: slot(m.expression.span()),
                optional: m.optional,
            },
        ),
        Expression::CallExpression(c) if !c.optional => (
            &c.callee,
            ChainStep::Call {
                suffix: slot(Span::new(c.callee.span().end, c.span.end)),
            },
        ),
        Expression::CallExpression(_) => return None,
        _ => return Some(CalleePlan::new(slot(expression.span()), Vec::new())),
    };
    member_chain(object, step)
}

fn member_chain(object: &Expression<'_>, step: ChainStep) -> Option<CalleePlan> {
    let mut plan = chain(object)?;
    plan.steps_mut().push(step);
    Some(plan)
}

fn guard_start(expression: &Expression<'_>) -> Option<u32> {
    let (object, optional) = match expression {
        Expression::ChainExpression(c) => match &c.expression {
            ChainElement::StaticMemberExpression(m) => (&m.object, m.optional),
            ChainElement::ComputedMemberExpression(m) => (&m.object, m.optional),
            _ => return None,
        },
        Expression::StaticMemberExpression(m) => (&m.object, m.optional),
        Expression::ComputedMemberExpression(m) => (&m.object, m.optional),
        Expression::CallExpression(c) if !c.optional => return guard_start(&c.callee),
        _ => return None,
    };
    guard_start(object).or_else(|| optional.then_some(object.span().end))
}

pub(crate) fn extract(
    program: &Program<'_>,
    facts: &ModuleFacts,
    segments: &[(Span, crate::MultiSpan)],
) -> EmissionFacts {
    let mut visitor = Extractor {
        facts,
        shorthand: HashMap::new(),
        operations: Vec::new(),
        names: Vec::new(),
        hoisted: Vec::new(),
        namespace_uses: HashMap::new(),
        blocked: NamespaceReason::Other,
        awaits: Vec::new(),
        enclosing_function: None,
    };
    if !facts.commonjs() {
        visitor.visit_program(program);
    }
    let awaits = &visitor.awaits;
    for operation in &mut visitor.operations {
        if let OperationKind::Invocation {
            invocation,
            lowering,
        } = operation
        {
            let call = &facts.invocations()[*invocation as usize];
            *lowering.crosses_await_guard_mut() = lowering.guard_start().is_some_and(|start| {
                awaits.iter().any(|(span, function)| {
                    *function == call.deferred()
                        && start <= span.start
                        && span.end <= call.span().end
                })
            });
        }
    }
    let (dynamic_namespaces, uses_this) = dynamic_namespace::extract(program, facts);
    let mut result = EmissionFacts::new(
        crate::execution::extract(program, facts),
        Default::default(),
        facts
            .bindings()
            .iter()
            .filter(|b| !b.top_level())
            .map(|b| b.id())
            .collect(),
        facts
            .references()
            .iter()
            .enumerate()
            .filter(|(_, r)| r.binding().is_none())
            .map(|(i, _)| i as u32)
            .collect(),
        dynamic_namespaces,
        uses_this,
    );
    for (span, source) in segments {
        let span = *span;
        let mut recipe = SegmentRecipe::new(
            source.clone(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        for &piece in source.as_slice() {
            source_operations(&mut recipe, program, facts, piece);
        }
        if facts.commonjs() {
            compose(&mut recipe, facts);
            result.segments_mut().push(recipe);
            continue;
        }
        let statement = program
            .body
            .get(program.body.partition_point(|s| s.span().end <= span.start));
        *recipe.connector_only_mut() =
            statement
                .filter(|s| s.span() == span)
                .is_some_and(|s| match s {
                    Statement::ImportDeclaration(_)
                    | Statement::ExportAllDeclaration(_)
                    | Statement::EmptyStatement(_) => true,
                    Statement::ExportNamedDeclaration(e) => e.declaration.is_none(),
                    _ => false,
                });
        for binding in facts.bindings() {
            if binding.top_level()
                && !binding.import()
                && binding.declarations().iter().any(|&s| contains(span, s))
            {
                recipe.placement_mut().declarations_mut().push(binding.id());
                if binding
                    .declarations()
                    .iter()
                    .filter(|&&s| contains(span, s))
                    .all(|s| visitor.hoisted.contains(&s.start))
                {
                    recipe.placement_mut().hoisted_mut().push(binding.id());
                }
                if facts
                    .references()
                    .iter()
                    .any(|r| r.binding() == Some(binding.id()) && r.span().start < span.start)
                {
                    recipe
                        .placement_mut()
                        .forward_references_mut()
                        .push(binding.id());
                }
            }
            for (i, &declaration) in binding
                .declarations()
                .iter()
                .enumerate()
                .filter(|(_, s)| contains(span, **s))
            {
                recipe.operations_mut().push(Operation::new(
                    None,
                    OperationKind::Binding {
                        binding: binding.id(),
                        declaration: i as u32,
                        shorthand: visitor
                            .shorthand
                            .get(&declaration.start)
                            .copied()
                            .unwrap_or_default(),
                    },
                ));
            }
        }
        *recipe.placement_mut().instantiable_mut() = !recipe.placement().declarations().is_empty()
            && recipe.placement().declarations() == recipe.placement().hoisted();
        let declaration = statement.and_then(|s| match s {
            Statement::ExportNamedDeclaration(e) => e.declaration.as_ref(),
            _ => None,
        });
        let variables = match statement {
            Some(Statement::VariableDeclaration(d)) => Some(d.as_ref()),
            _ => match declaration {
                Some(Declaration::VariableDeclaration(d)) => Some(d.as_ref()),
                _ => None,
            },
        };
        *recipe.deferred_mut() = if let Some(variables) = variables {
            variables
                .declarations
                .iter()
                .find(|d| contains(span, d.id.span()))
                .filter(|d| matches!(d.id, BindingPattern::BindingIdentifier(_)))
                .and_then(|d| {
                    facts
                        .bindings()
                        .iter()
                        .find(|b| !b.function() && b.declarations().contains(&d.id.span()))
                        .map(|b| DeferredForm::Variable {
                            binding: b.id(),
                            initializer: d.init.as_ref().map(|e| slot(e.span())),
                        })
                })
                .unwrap_or_default()
        } else if recipe.placement().instantiable()
            && (matches!(statement, Some(Statement::FunctionDeclaration(_)))
                || matches!(declaration, Some(Declaration::FunctionDeclaration(_))))
        {
            DeferredForm::Function
        } else if recipe.placement().declarations().is_empty()
            && !matches!(statement, Some(Statement::ExportDefaultDeclaration(_)))
        {
            DeferredForm::Effect
        } else {
            DeferredForm::Unsupported
        };
        *recipe.placement_mut().default_expression_mut() = facts
            .exports()
            .iter()
            .any(|e| matches!(e.value(), ExportValue::Expression(s) if contains(span, *s)));
        for (i, reference) in facts
            .references()
            .iter()
            .enumerate()
            .filter(|(_, r)| contains(span, r.span()))
        {
            let index = i as u32;
            if result
                .execution()
                .is_module_evaluation(result.execution().references()[i])
            {
                if reference.read() {
                    recipe.placement_mut().eager_reads_mut().push(index);
                }
                if reference.write() {
                    recipe.placement_mut().eager_writes_mut().push(index);
                }
            }
            if reference.write() {
                recipe.placement_mut().writes_mut().push(index);
            }
            recipe.operations_mut().push(Operation::new(
                None,
                OperationKind::Reference {
                    reference: index,
                    shorthand: visitor
                        .shorthand
                        .get(&reference.span().start)
                        .copied()
                        .unwrap_or_default(),
                },
            ));
            if let Some(access) = visitor.namespace_uses.get(&index) {
                let usage = recipe.namespaces().len() as u32;
                recipe
                    .namespaces_mut()
                    .push(NamespaceUse::new(index, access.clone()));
                if matches!(access, NamespaceAccess::DirectRead { .. }) {
                    recipe
                        .operations_mut()
                        .push(Operation::new(None, OperationKind::Namespace { usage }));
                }
            }
        }
        for (name_span, binding) in &visitor.names {
            if contains(span, *name_span) {
                if let Some(binding) = binding {
                    recipe.names_mut().named_mut().push(*binding);
                } else {
                    *recipe.names_mut().inference_sensitive_mut() = true;
                }
            }
        }
        recipe.names_mut().named_mut().sort_by_key(|b| b.0);
        recipe.names_mut().named_mut().dedup();
        let syntax: Vec<_> = visitor
            .operations
            .iter()
            .filter(|kind| contains(span, kind.span(facts, &recipe)))
            .cloned()
            .map(|kind| Operation::new(None, kind))
            .collect();
        recipe.operations_mut().extend(syntax);
        if source.as_slice().len() > 1 {
            let prefix = source.as_slice()[0];
            if let Some(Statement::ExportNamedDeclaration(export)) = statement {
                let declaration = export.declaration.as_ref().expect("split declaration");
                recipe.operations_mut().push(Operation::new(
                    None,
                    OperationKind::Export {
                        span: prefix,
                        form: ExportForm::Declaration {
                            declaration: Span::new(declaration.span().start, prefix.end),
                        },
                    },
                ));
            }
        }
        compose(&mut recipe, facts);
        result.segments_mut().push(recipe);
    }
    result
}

fn source_operations(
    recipe: &mut SegmentRecipe,
    program: &Program<'_>,
    facts: &ModuleFacts,
    span: Span,
) {
    for dependency in facts.dependencies().iter().filter(|d| {
        contains(span, d.span())
            && matches!(
                d.kind(),
                crate::owned::DependencyKind::DynamicImport | crate::owned::DependencyKind::Require
            )
    }) {
        recipe.operations_mut().push(Operation::new(
            None,
            OperationKind::Dependency {
                dependency: dependency.id(),
            },
        ));
    }
    for comment in &program.comments {
        if !contains(span, comment.span) {
            continue;
        }
        let text = &program.source_text[comment.span.start as usize..comment.span.end as usize];
        let body = text
            .strip_prefix("//")
            .or_else(|| text.strip_prefix("/*").and_then(|t| t.strip_suffix("*/")));
        if body
            .and_then(|b| b.strip_prefix('#').or_else(|| b.strip_prefix('@')))
            .is_some_and(|b| {
                b.trim_start().starts_with("sourceMappingURL=")
                    || b.trim_start().starts_with("sourceURL=")
            })
        {
            recipe.operations_mut().push(Operation::new(
                None,
                OperationKind::SourceHint { span: comment.span },
            ));
        }
    }
}

fn compose(recipe: &mut SegmentRecipe, facts: &ModuleFacts) {
    let ranges: Vec<_> = recipe
        .operations()
        .iter()
        .map(|o| o.kind().span(facts, recipe))
        .collect();
    let mut order: Vec<_> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (ranges[i].start, std::cmp::Reverse(ranges[i].end)));
    *recipe.operations_mut() = order
        .into_iter()
        .map(|i| recipe.operations()[i].clone())
        .collect();
    let ranges: Vec<_> = recipe
        .operations()
        .iter()
        .map(|o| o.kind().span(facts, recipe))
        .collect();
    let mut stack: Vec<usize> = Vec::new();
    for (i, &range) in ranges.iter().enumerate() {
        while stack.last().is_some_and(|&p| !contains(ranges[p], range)) {
            stack.pop();
        }
        *recipe.operations_mut()[i].parent_mut() = stack.last().map(|&p| p as u32);
        stack.push(i);
    }
    let parents: Vec<_> = recipe.operations().iter().map(|o| o.parent()).collect();
    if let DeferredForm::Variable {
        initializer: Some(slot),
        ..
    } = recipe.deferred_mut()
    {
        *slot.operations_mut() = ranges
            .iter()
            .enumerate()
            .filter(|&(j, &range)| {
                contains(slot.span(), range)
                    && !parents[j].is_some_and(|p| contains(slot.span(), ranges[p as usize]))
            })
            .map(|(j, _)| j as u32)
            .collect();
    }
    for (i, operation) in recipe.operations_mut().iter_mut().enumerate() {
        let fill = |slot: &mut SourceSlot| {
            *slot.operations_mut() = ranges
                .iter()
                .enumerate()
                .filter(|&(j, &range)| {
                    j != i
                        && contains(slot.span(), range)
                        && !parents[j].is_some_and(|p| {
                            p as usize != i && contains(slot.span(), ranges[p as usize])
                        })
                })
                .map(|(j, _)| j as u32)
                .collect();
        };
        match operation.kind_mut() {
            OperationKind::Export {
                form: ExportForm::Default { value, .. },
                ..
            } => fill(value),
            OperationKind::Invocation { lowering, .. } => {
                fill(lowering.callee_mut());
                for argument in lowering.arguments_mut() {
                    fill(argument);
                }
                if let Some(chain) = lowering.chain_mut() {
                    fill(chain.base_mut());
                    for step in chain.steps_mut() {
                        match step {
                            ChainStep::Computed { property, .. } => fill(property),
                            ChainStep::Call { suffix } => fill(suffix),
                            ChainStep::Static { .. } => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

struct Extractor<'f> {
    facts: &'f ModuleFacts,
    shorthand: HashMap<u32, Shorthand>,
    operations: Vec<OperationKind>,
    names: Vec<(Span, Option<BindingId>)>,
    hoisted: Vec<u32>,
    namespace_uses: HashMap<u32, NamespaceAccess>,
    blocked: NamespaceReason,
    awaits: Vec<(Span, Option<Span>)>,
    enclosing_function: Option<Span>,
}

impl Extractor<'_> {
    fn binding(&self, span: Span) -> Option<BindingId> {
        self.facts
            .bindings()
            .iter()
            .find(|b| b.declarations().contains(&span))
            .map(|b| b.id())
    }

    fn namespace(&self, span: Span) -> Option<u32> {
        let i = self
            .facts
            .references()
            .binary_search_by_key(&(span.start, span.end), |r| (r.span().start, r.span().end))
            .ok()?;
        self.facts
            .imports()
            .iter()
            .any(|import| Some(import.binding()) == self.facts.references()[i].binding())
            .then_some(i as u32)
    }

    fn export(&mut self, span: Span, form: ExportForm) {
        self.operations.push(OperationKind::Export { span, form });
    }

    fn invocation(&mut self, span: Span, callee: &Expression<'_>) {
        let Ok(i) = self
            .facts
            .invocations()
            .binary_search_by_key(&(span.start, span.end), |c| (c.span().start, c.span().end))
        else {
            return;
        };
        let call = &self.facts.invocations()[i];
        let boundary =
            guard_start(callee).or_else(|| call.optional_call().then_some(call.callee().end));
        self.operations.push(OperationKind::Invocation {
            invocation: i as u32,
            lowering: PureLowering::new(
                slot(call.callee()),
                chain(callee),
                call.arguments().iter().map(|a| slot(a.span())).collect(),
                boundary,
                false,
            ),
        });
    }
}

impl<'a> Visit<'a> for Extractor<'_> {
    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        self.operations.push(OperationKind::Dependency {
            dependency: DependencyId(import.span.start),
        });
        walk::walk_import_declaration(self, import);
    }
    fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
        self.export(
            export.span,
            export
                .declaration
                .as_ref()
                .map_or(ExportForm::Remove, |d| ExportForm::Declaration {
                    declaration: d.span(),
                }),
        );
        if let Some(Declaration::FunctionDeclaration(f)) = &export.declaration {
            if let Some(id) = &f.id {
                self.hoisted.push(id.span.start);
            }
        }
        walk::walk_export_named_declaration(self, export);
    }
    fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
        self.export(export.span, ExportForm::Remove);
    }
    fn visit_export_default_declaration(&mut self, export: &ExportDefaultDeclaration<'a>) {
        let anonymous_function = matches!(&export.declaration, ExportDefaultDeclarationKind::FunctionDeclaration(f) if f.id.is_none());
        let expression =
            self.facts.exports().iter().any(|e| {
                e.span() == export.span && matches!(e.value(), ExportValue::Expression(_))
            });
        self.export(
            export.span,
            if expression {
                ExportForm::Default {
                    value: slot(export.declaration.span()),
                    hoisted: anonymous_function,
                }
            } else {
                ExportForm::Declaration {
                    declaration: export.declaration.span(),
                }
            },
        );
        if let ExportDefaultDeclarationKind::FunctionDeclaration(f) = &export.declaration {
            if let Some(id) = &f.id {
                self.hoisted.push(id.span.start);
            }
        }
        walk::walk_export_default_declaration(self, export);
    }
    fn visit_statement(&mut self, statement: &Statement<'a>) {
        if let Statement::FunctionDeclaration(f) = statement {
            if let Some(id) = &f.id {
                if self
                    .facts
                    .bindings()
                    .iter()
                    .any(|b| b.id() == BindingId(id.span.start) && b.top_level())
                {
                    self.hoisted.push(id.span.start);
                }
            }
        }
        walk::walk_statement(self, statement);
    }
    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        let saved = self.enclosing_function.replace(function.span);
        self.names.push((
            function.span,
            function.id.as_ref().and_then(|id| self.binding(id.span)),
        ));
        walk::walk_function(self, function, flags);
        self.enclosing_function = saved;
    }
    fn visit_class(&mut self, class: &Class<'a>) {
        self.names.push((
            class.span,
            class.id.as_ref().and_then(|id| self.binding(id.span)),
        ));
        walk::walk_class(self, class);
    }
    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        let saved = self.enclosing_function.replace(arrow.span);
        self.names.push((arrow.span, None));
        walk::walk_arrow_function_expression(self, arrow);
        self.enclosing_function = saved;
    }
    fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
        self.awaits.push((expression.span, self.enclosing_function));
        walk::walk_await_expression(self, expression);
    }
    fn visit_assignment_target_property_identifier(
        &mut self,
        p: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.shorthand
            .insert(p.binding.span.start, Shorthand::Property);
        walk::walk_assignment_target_property_identifier(self, p);
    }
    fn visit_binding_property(&mut self, p: &BindingProperty<'a>) {
        if p.shorthand {
            self.shorthand
                .insert(p.value.span().start, Shorthand::Property);
        }
        walk::walk_binding_property(self, p);
    }
    fn visit_object_property(&mut self, p: &ObjectProperty<'a>) {
        if p.shorthand {
            self.shorthand.insert(
                p.value.span().start,
                if matches!(&p.key, PropertyKey::StaticIdentifier(k) if k.name == "__proto__") {
                    Shorthand::Proto
                } else {
                    Shorthand::Property
                },
            );
        }
        walk::walk_object_property(self, p);
    }
    fn visit_identifier_reference(&mut self, reference: &IdentifierReference<'a>) {
        if let Some(i) = self.namespace(reference.span) {
            self.namespace_uses
                .entry(i)
                .or_insert(NamespaceAccess::Native(self.blocked));
        }
    }
    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if self.blocked == NamespaceReason::Other {
            if let Expression::Identifier(object) = &member.object {
                if let Some(i) = self.namespace(object.span) {
                    self.namespace_uses.insert(
                        i,
                        NamespaceAccess::DirectRead {
                            member: member.span,
                            property: member.property.name.to_string(),
                        },
                    );
                }
            }
        }
        walk::walk_static_member_expression(self, member);
    }
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        self.invocation(call.span, &call.callee);
        let saved = self.blocked;
        self.blocked = NamespaceReason::Receiver;
        self.visit_expression(&call.callee);
        self.blocked = saved;
        for argument in &call.arguments {
            self.visit_argument(argument);
        }
    }
    fn visit_new_expression(&mut self, call: &NewExpression<'a>) {
        self.invocation(call.span, &call.callee);
        walk::walk_new_expression(self, call);
    }
    fn visit_tagged_template_expression(&mut self, tagged: &TaggedTemplateExpression<'a>) {
        let saved = self.blocked;
        self.blocked = NamespaceReason::Tag;
        self.visit_expression(&tagged.tag);
        self.blocked = saved;
        self.visit_template_literal(&tagged.quasi);
    }
    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        let saved = self.blocked;
        self.blocked = NamespaceReason::Write;
        walk::walk_assignment_target(self, target);
        self.blocked = saved;
    }
    fn visit_update_expression(&mut self, update: &UpdateExpression<'a>) {
        let saved = self.blocked;
        self.blocked = NamespaceReason::Write;
        walk::walk_update_expression(self, update);
        self.blocked = saved;
    }
    fn visit_unary_expression(&mut self, unary: &UnaryExpression<'a>) {
        let saved = self.blocked;
        if unary.operator == UnaryOperator::Delete {
            self.blocked = NamespaceReason::Delete;
        }
        walk::walk_unary_expression(self, unary);
        self.blocked = saved;
    }
}
