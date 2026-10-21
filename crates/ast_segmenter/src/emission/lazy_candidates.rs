//! Shape candidates only. Constructor identity and all-reference eligibility stay linked.
use super::*;
use crate::owned::{DependencyKind, ImportedName};

pub(super) fn extract(
    program: &Program<'_>,
    facts: &ModuleFacts,
    recipes: &[SegmentRecipe],
) -> (Vec<LazyModuleCandidate>, Vec<LazySelectorCandidate>) {
    let mut visitor = Candidates {
        facts,
        recipes,
        modules: Vec::new(),
        selectors: Vec::new(),
    };
    visitor.visit_program(program);
    visitor.modules.sort_by_key(|c| c.construction().start);
    visitor.selectors.sort_by_key(|c| c.construction().start);
    (visitor.modules, visitor.selectors)
}

struct Candidates<'f> {
    facts: &'f ModuleFacts,
    recipes: &'f [SegmentRecipe],
    modules: Vec<LazyModuleCandidate>,
    selectors: Vec<LazySelectorCandidate>,
}

fn unparenthesized<'b, 'a>(mut expression: &'b Expression<'a>) -> &'b Expression<'a> {
    while let Expression::ParenthesizedExpression(parenthesized) = expression {
        expression = &parenthesized.expression;
    }
    expression
}

fn concise<'b, 'a>(arrow: &'b ArrowFunctionExpression<'a>) -> Option<&'b Expression<'a>> {
    if arrow.r#async || !arrow.expression || arrow.body.statements.len() != 1 {
        return None;
    }
    match &arrow.body.statements[0] {
        Statement::ExpressionStatement(s) => Some(unparenthesized(&s.expression)),
        _ => None,
    }
}

fn options_properties(new: &NewExpression<'_>) -> Option<Vec<LazyOptionProperty>> {
    let Expression::ObjectExpression(object) = new.arguments.get(1)?.as_expression()? else {
        return None;
    };
    // The borrowed boundary proof does not lex regexps or division tokens.
    struct UnsupportedValue(bool);
    impl<'a> Visit<'a> for UnsupportedValue {
        fn visit_expression(&mut self, expression: &Expression<'a>) {
            if matches!(expression, Expression::RegExpLiteral(_))
                || matches!(expression, Expression::BinaryExpression(binary) if binary.operator.as_str() == "/")
                || matches!(expression, Expression::AssignmentExpression(assignment) if assignment.operator.as_str() == "/=")
            {
                self.0 = true;
            }
            walk::walk_expression(self, expression);
        }
    }
    let mut unsupported = UnsupportedValue(false);
    unsupported.visit_object_expression(object);
    if unsupported.0 {
        return None;
    }
    object
        .properties
        .iter()
        .map(|property| {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                return None;
            };
            if property.computed || property.method || property.kind != PropertyKind::Init {
                return None;
            }
            // Escaped string keys stay unsupported; identifiers retain their decoded name.
            if let PropertyKey::StringLiteral(key) = &property.key {
                if key.raw.as_ref().is_none_or(|raw| raw.contains('\\')) {
                    return None;
                }
            } else if !matches!(property.key, PropertyKey::StaticIdentifier(_)) {
                return None;
            }
            let name = property.key.static_name()?;
            if matches!(property.key, PropertyKey::StaticIdentifier(_))
                && property.key.span().size() as usize != name.len()
            {
                return None;
            }
            if matches!(name.as_ref(), "initializer" | "__proto__") {
                return None;
            }
            Some(LazyOptionProperty::new(
                property.key.span(),
                property.value.span(),
            ))
        })
        .collect()
}

impl Candidates<'_> {
    fn reference(&self, expression: &Expression<'_>) -> Option<u32> {
        let Expression::Identifier(id) = unparenthesized(expression) else {
            return None;
        };
        self.facts
            .references()
            .iter()
            .position(|r| r.span() == id.span && !r.type_only())
            .map(|i| i as u32)
    }

    fn binding(&self, span: Span) -> Option<BindingId> {
        self.facts
            .bindings()
            .iter()
            .find(|b| b.declarations().contains(&span))
            .map(|b| b.id())
    }

    fn segment(&self, construction: Span) -> Option<u32> {
        self.recipes
            .iter()
            .position(|r| {
                r.source()
                    .as_slice()
                    .iter()
                    .any(|&s| contains(s, construction))
            })
            .map(|i| i as u32)
    }

    fn arguments(&self, new: &NewExpression<'_>, segment: u32) -> Vec<SourceSlot> {
        let recipe = &self.recipes[segment as usize];
        let ranges: Vec<_> = recipe
            .operations()
            .iter()
            .map(|o| o.kind().span(self.facts, recipe))
            .collect();
        new.arguments
            .iter()
            .map(|argument| {
                let span = argument.span();
                let operations = ranges
                    .iter()
                    .enumerate()
                    .filter(|&(i, &range)| {
                        contains(span, range)
                            && !recipe.operations()[i]
                                .parent()
                                .is_some_and(|p| contains(span, ranges[p as usize]))
                    })
                    .map(|(i, _)| i as u32)
                    .collect();
                SourceSlot::new(span, operations)
            })
            .collect()
    }

    fn wrapper_kind(&self, constructor: u32) -> LazyWrapperKind {
        let reference = &self.facts.references()[constructor as usize];
        let name = self
            .facts
            .imports()
            .iter()
            .find(|i| Some(i.binding()) == reference.binding())
            .and_then(|i| match i.imported() {
                ImportedName::Name(name) => Some(name.as_str()),
                ImportedName::Namespace => None,
            })
            .unwrap_or(reference.name());
        LazyWrapperKind::from_name(name)
    }
}

impl<'a> Visit<'a> for Candidates<'_> {
    fn visit_variable_declarator(&mut self, declaration: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &declaration.id {
            if let Some(Expression::NewExpression(new)) =
                declaration.init.as_ref().map(unparenthesized)
            {
                let shape = || {
                    let binding = self.binding(id.span)?;
                    let constructor = self.reference(&new.callee)?;
                    let segment = self.segment(new.span)?;
                    let Expression::ArrowFunctionExpression(arrow) =
                        unparenthesized(new.arguments.first()?.as_expression()?)
                    else {
                        return None;
                    };
                    if !arrow.params.items.is_empty() || arrow.params.rest.is_some() {
                        return None;
                    }
                    let Expression::ImportExpression(import) = concise(arrow)? else {
                        return None;
                    };
                    if !matches!(
                        unparenthesized(&import.source),
                        Expression::StringLiteral(_)
                    ) || import.options.is_some()
                        || import.phase.is_some()
                    {
                        return None;
                    }
                    let dependency = self.facts.dependencies().iter().find(|d| {
                        d.kind() == DependencyKind::DynamicImport
                            && d.span() == import.span
                            && d.pattern().is_none()
                    })?;
                    Some(LazyModuleCandidate::new(
                        segment,
                        binding,
                        constructor,
                        (dependency.id(), import.span),
                        declaration.span,
                        new.span,
                        (self.arguments(new, segment), options_properties(new)),
                    ))
                };
                if let Some(candidate) = shape() {
                    self.modules.push(candidate);
                }
            }
        }
        walk::walk_variable_declarator(self, declaration);
    }

    fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
        let shape = || {
            let constructor = self.reference(&new.callee)?;
            let segment = self.segment(new.span)?;
            let module_reference = self.reference(new.arguments.first()?.as_expression()?)?;
            let module_binding = self.facts.references()[module_reference as usize].binding()?;
            let Expression::ArrowFunctionExpression(arrow) =
                unparenthesized(new.arguments.get(1)?.as_expression()?)
            else {
                return None;
            };
            if arrow.params.items.len() != 1
                || arrow.params.rest.is_some()
                || arrow.params.items[0].initializer.is_some()
            {
                return None;
            }
            let BindingPattern::BindingIdentifier(parameter) = &arrow.params.items[0].pattern
            else {
                return None;
            };
            let parameter_binding = self.binding(parameter.span)?;
            let Expression::StaticMemberExpression(member) = concise(arrow)? else {
                return None;
            };
            if member.optional {
                return None;
            }
            let getter_reference = self.reference(&member.object)?;
            if self.facts.references()[getter_reference as usize].binding()
                != Some(parameter_binding)
            {
                return None;
            }
            Some(LazySelectorCandidate::new(
                segment,
                constructor,
                (module_reference, module_binding),
                (
                    getter_reference,
                    member.property.name.to_string(),
                    member.property.span,
                ),
                new.span,
                self.arguments(new, segment),
                self.wrapper_kind(constructor),
            ))
        };
        if let Some(candidate) = shape() {
            self.selectors.push(candidate);
        }
        walk::walk_new_expression(self, new);
    }
}
