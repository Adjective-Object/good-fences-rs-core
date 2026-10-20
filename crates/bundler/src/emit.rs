//! Retention-driven JavaScript ESM emission. Publication is the caller's job.
use std::collections::BTreeSet;

use ast_segmenter::{owned::DependencyKind, ModuleBody, ModuleFormat, ParsedModule};
use eyre::{ensure, OptionExt, WrapErr};
use oxc_span::Span;
use source_graph::{SegmentKey, SourceGraph};

use crate::{
    policy::EffectPolicy,
    rewrite::{self, Edit},
    RetentionPlan,
};

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;

/// Emit one isolated ESM module using an existing, current retention plan.
/// Imports require `link` rather than an implicit external or CJS policy.
pub fn single_module(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    policy: &EffectPolicy,
    file: u32,
    source: &str,
) -> eyre::Result<String> {
    ensure!(
        retention.is_current(graph, policy),
        "stale emission retention/policy"
    );
    let parsed = checked_source(graph, file, source)?;
    ensure!(
        parsed.body == ModuleBody::Esm,
        "single-module emission requires ESM; use link::esm for CJS"
    );
    if let Some(dependency) = parsed.facts.dependencies.iter().find(|d| !d.type_only) {
        eyre::bail!(
            "{}: dependency needs linked emission at bytes {}..{} ({:?})",
            graph.file_path(file).unwrap().display(),
            dependency.span.start,
            dependency.span.end,
            dependency.kind
        );
    }
    ensure!(
        retention.modules == [file],
        "single-module emission needs one evaluated module"
    );
    project(graph, retention, file, source, &parsed, &[])
}

pub(crate) fn checked_source(
    graph: &SourceGraph,
    file: u32,
    source: &str,
) -> eyre::Result<ParsedModule> {
    let path = graph
        .file_path(file)
        .ok_or_eyre("missing emission module")?;
    let format = if graph.file_facts(file).is_some_and(|f| f.commonjs) {
        ModuleFormat::CommonJs
    } else {
        ModuleFormat::Esm
    };
    let parsed = ast_segmenter::parse_module(&logger::StdioLogger::new(), source, path, format)?;
    ensure!(
        !parsed.source_type.is_typescript() && !parsed.source_type.is_jsx(),
        "{}: emission supports JavaScript only at bytes 0..{}; TS/JSX need later backends",
        path.display(),
        source.len()
    );
    ensure!(
        graph.file_facts(file) == Some(&parsed.facts)
            && graph.file_segments(file).is_some_and(|segments| segments
                .iter()
                .map(|s| s.span)
                .eq(parsed.segments.iter().map(|s| s.span))),
        "{}: source facts/ranges do not match graph; reparse and rebuild before emission",
        path.display()
    );
    Ok(parsed)
}

pub(crate) fn project(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    source: &str,
    parsed: &ParsedModule,
    dependency_edits: &[Edit],
) -> eyre::Result<String> {
    let edits = projection_edits(graph, retention, file, source, parsed, dependency_edits)?;
    let output = rewrite::apply(source, &edits)?;
    rewrite::validate(graph.file_path(file).unwrap(), &output)?;
    rewrite::apply(&output, &rewrite::source_hint_edits(&output))
}

fn projection_edits(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    source: &str,
    parsed: &ParsedModule,
    dependency_edits: &[Edit],
) -> eyre::Result<Vec<Edit>> {
    let path = graph.file_path(file).ok_or_eyre("missing emission path")?;
    let live: BTreeSet<_> = retention
        .segments
        .iter()
        .filter(|s| s.key.file_id == file)
        .map(|s| s.key.segment_idx)
        .collect();
    let mut edits = Vec::new();
    for (index, segment) in parsed.segments.iter().enumerate() {
        let key = SegmentKey::new(file, index as u32);
        if let Some(edit) = dependency_edits.iter().find(|e| e.span == segment.span) {
            edits.push(edit.clone());
            continue;
        }
        let dependencies: Vec<_> = parsed
            .facts
            .dependencies
            .iter()
            .filter(|d| {
                !d.type_only
                    && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
                    && contains(segment.span, d.span)
            })
            .collect();
        if !live.contains(&(index as u32)) {
            // Unused bindings do not cancel static dependency evaluation.
            let mut replacement = String::new();
            for dependency in dependencies {
                let specifier = dependency_edits
                    .iter()
                    .find(|e| e.span == dependency.specifier_span)
                    .map(|e| e.replacement.as_str())
                    .unwrap_or(
                        &source[dependency.specifier_span.start as usize
                            ..dependency.specifier_span.end as usize],
                    );
                replacement.push_str(&format!("import {specifier};"));
            }
            edits.push(Edit {
                span: segment.span,
                replacement,
            });
            continue;
        }
        let rewrites = retention
            .effects
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.summary.rewrites.as_slice())
            .unwrap_or_default();
        let mut renderer = CachedRenderer::new(
            source,
            &parsed.facts,
            &parsed.facts.emission.segments[index],
            rewrites,
        );
        renderer.dependencies(dependency_edits)?;
        let replacement = renderer.render(segment.span).wrap_err_with(|| {
            format!(
                "{}: PURE rewrite at bytes {}..{}",
                path.display(),
                segment.span.start,
                segment.span.end
            )
        })?;
        edits.push(Edit {
            span: segment.span,
            replacement,
        });
    }
    Ok(edits)
}

fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

struct Fresh {
    names: BTreeSet<String>,
    next: usize,
}

impl Fresh {
    fn new(facts: &ast_segmenter::owned::ModuleFacts) -> Self {
        Self {
            names: facts
                .bindings
                .iter()
                .map(|b| b.name.clone())
                .chain(facts.references.iter().map(|r| r.name.clone()))
                .collect(),
            next: 0,
        }
    }

    fn name(&mut self) -> String {
        loop {
            let name = format!("__gf${}", self.next);
            self.next += 1;
            if self.names.insert(name.clone()) {
                return name;
            }
        }
    }
}

/// Instantiate only checked original-coordinate operations. Replacing operations
/// own their children; source slots carry the precomputed child roots.
pub(crate) struct CachedRenderer<'a> {
    source: &'a str,
    facts: &'a ast_segmenter::owned::ModuleFacts,
    recipe: &'a ast_segmenter::emission::SegmentRecipe,
    rewrites: &'a [crate::effects::InvocationRewrite],
    replacements: std::collections::BTreeMap<u32, String>,
    edits: std::collections::BTreeMap<u32, Vec<Edit>>,
    fresh: Fresh,
}

impl<'a> CachedRenderer<'a> {
    pub(crate) fn new(
        source: &'a str,
        facts: &'a ast_segmenter::owned::ModuleFacts,
        recipe: &'a ast_segmenter::emission::SegmentRecipe,
        rewrites: &'a [crate::effects::InvocationRewrite],
    ) -> Self {
        Self {
            source,
            facts,
            recipe,
            rewrites,
            replacements: Default::default(),
            edits: Default::default(),
            fresh: Fresh::new(facts),
        }
    }

    pub(crate) fn replace(&mut self, operation: u32, replacement: String) {
        self.replacements.insert(operation, replacement);
    }

    pub(crate) fn edit(&mut self, operation: u32, edit: Edit) {
        self.edits.entry(operation).or_default().push(edit);
    }

    pub(crate) fn dependencies(&mut self, edits: &[Edit]) -> eyre::Result<()> {
        use ast_segmenter::emission::{ExportForm, OperationKind};
        for (index, operation) in self.recipe.operations.iter().enumerate() {
            if !matches!(
                operation.kind,
                OperationKind::Dependency { .. }
                    | OperationKind::Export {
                        form: ExportForm::Remove,
                        ..
                    }
            ) {
                continue;
            }
            let span = operation.kind.span(self.facts, self.recipe);
            let edits: Vec<_> = edits
                .iter()
                .filter(|edit| contains(span, edit.span))
                .cloned()
                .collect();
            if !edits.is_empty() {
                if let Some(edit) = edits.iter().find(|edit| edit.span == span) {
                    self.replace(index as u32, edit.replacement.clone());
                } else {
                    self.edits.insert(index as u32, edits);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn render(&mut self, range: Span) -> eyre::Result<String> {
        self.children(range, None)
    }

    pub(crate) fn slot(
        &mut self,
        slot: &ast_segmenter::emission::SourceSlot,
    ) -> eyre::Result<String> {
        self.roots(slot.span, &slot.operations, Vec::new())
    }

    pub(crate) fn children(&mut self, range: Span, parent: Option<u32>) -> eyre::Result<String> {
        let roots: Vec<_> = self
            .recipe
            .operations
            .iter()
            .enumerate()
            .filter(|(_, operation)| operation.parent == parent)
            .map(|(index, _)| index as u32)
            .collect();
        let edits = parent
            .and_then(|index| self.edits.get(&index))
            .cloned()
            .unwrap_or_default();
        self.roots(range, &roots, edits)
    }

    fn roots(&mut self, range: Span, roots: &[u32], mut edits: Vec<Edit>) -> eyre::Result<String> {
        for &index in roots {
            let operation = &self.recipe.operations[index as usize];
            let span = operation.kind.span(self.facts, self.recipe);
            edits.push(Edit {
                span,
                replacement: self.operation(index)?,
            });
        }
        rewrite::apply_range(self.source, range, &edits)
    }

    fn operation(&mut self, index: u32) -> eyre::Result<String> {
        use ast_segmenter::emission::OperationKind;
        if let Some(replacement) = self.replacements.get(&index) {
            return Ok(replacement.clone());
        }
        let operation = &self.recipe.operations[index as usize];
        let span = operation.kind.span(self.facts, self.recipe);
        if let OperationKind::SourceHint { .. } = operation.kind {
            return Ok(String::new());
        }
        if let OperationKind::Invocation {
            invocation,
            ref lowering,
        } = operation.kind
        {
            if let Some(recipe) = self.rewrites.iter().find(|rewrite| {
                rewrite.invocation == self.facts.invocations[invocation as usize].span
            }) {
                if !crosses_await_guard(self.source, recipe, &self.facts.top_level_await)? {
                    let callee = self.slot(&lowering.callee)?;
                    let mut arguments = Vec::new();
                    for (slot, argument) in lowering.arguments.iter().zip(&recipe.arguments) {
                        let value = self.slot(slot)?;
                        arguments.push(if argument.spread {
                            format!("[...({value})]")
                        } else {
                            format!("({value})")
                        });
                    }
                    arguments.push("void 0".into());
                    let tail = arguments.join(",");
                    return match recipe.guard {
                        crate::effects::ArgumentGuard::Always => Ok(format!("(({callee}),{tail})")),
                        crate::effects::ArgumentGuard::OriginalChain { optional_call } => {
                            pure_chain(&callee, optional_call, &tail, &mut self.fresh)
                        }
                    };
                }
            }
        }
        self.children(span, Some(index))
    }
}

fn crosses_await_guard(
    source: &str,
    recipe: &crate::effects::InvocationRewrite,
    awaits: &[Span],
) -> eyre::Result<bool> {
    let crate::effects::ArgumentGuard::OriginalChain { optional_call } = recipe.guard else {
        return Ok(false);
    };
    if !awaits.iter().any(|&span| contains(recipe.invocation, span)) {
        return Ok(false);
    }
    let allocator = oxc_allocator::Allocator::default();
    let callee = &source[recipe.callee.start as usize..recipe.callee.end as usize];
    rewrite::count_output_parse();
    let expression = oxc_parser::Parser::new(&allocator, callee, oxc_span::SourceType::mjs())
        .parse_expression()
        .map_err(|e| eyre::eyre!("PURE callee parser diagnostics: {e:?}"))?;
    let boundary = chain_guard_start(&expression)
        .map(|offset| recipe.callee.start + offset)
        .or_else(|| optional_call.then_some(recipe.callee.end));
    Ok(boundary.is_some_and(|start| {
        awaits
            .iter()
            .any(|span| start <= span.start && span.end <= recipe.invocation.end)
    }))
}

fn chain_guard_start(expression: &oxc_ast::ast::Expression<'_>) -> Option<u32> {
    use oxc_ast::ast::*;
    use oxc_span::GetSpan;
    // Match chain's lowering path; parentheses keep their own native chain boundary.
    let (object, optional) = match expression {
        Expression::ChainExpression(c) => match &c.expression {
            ChainElement::StaticMemberExpression(m) => (&m.object, m.optional),
            ChainElement::ComputedMemberExpression(m) => (&m.object, m.optional),
            _ => return None,
        },
        Expression::StaticMemberExpression(m) => (&m.object, m.optional),
        Expression::ComputedMemberExpression(m) => (&m.object, m.optional),
        Expression::CallExpression(c) if !c.optional => return chain_guard_start(&c.callee),
        _ => return None,
    };
    chain_guard_start(object).or_else(|| optional.then_some(object.span().end))
}

fn pure_chain(callee: &str, optional: bool, tail: &str, fresh: &mut Fresh) -> eyre::Result<String> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    let allocator = Allocator::default();
    rewrite::count_output_parse();
    let expression = Parser::new(&allocator, callee, SourceType::mjs())
        .parse_expression()
        .map_err(|e| eyre::eyre!("PURE callee parser diagnostics: {e:?}"))?;
    chain(&expression, callee, fresh, &|value, fresh| {
        if optional {
            let name = fresh.name();
            Ok(format!(
                "({name}=>{name}===null||{name}===void 0?void 0:({tail}))({value})"
            ))
        } else {
            Ok(format!("(({value}),{tail})"))
        }
    })
}

type Continuation<'a> = dyn Fn(String, &mut Fresh) -> eyre::Result<String> + 'a;

fn chain(
    expression: &oxc_ast::ast::Expression<'_>,
    source: &str,
    fresh: &mut Fresh,
    continuation: &Continuation<'_>,
) -> eyre::Result<String> {
    use oxc_ast::ast::*;
    use oxc_span::GetSpan;
    match expression {
        Expression::ChainExpression(c) => match &c.expression {
            ChainElement::StaticMemberExpression(m) => member(
                &m.object,
                &format!(".{}", m.property.name),
                m.optional,
                source,
                fresh,
                continuation,
            ),
            ChainElement::ComputedMemberExpression(m) => member(
                &m.object,
                &format!(
                    "[{}]",
                    &source[m.expression.span().start as usize..m.expression.span().end as usize]
                ),
                m.optional,
                source,
                fresh,
                continuation,
            ),
            ChainElement::CallExpression(_) | ChainElement::PrivateFieldExpression(_) => {
                eyre::bail!(
                    "unsupported nested optional call/private PURE callee at bytes {}..{}",
                    c.span.start,
                    c.span.end
                )
            }
            ChainElement::TSNonNullExpression(_) => {
                eyre::bail!("TypeScript PURE callee is unsupported")
            }
        },
        Expression::StaticMemberExpression(m) => member(
            &m.object,
            &format!(".{}", m.property.name),
            m.optional,
            source,
            fresh,
            continuation,
        ),
        Expression::ComputedMemberExpression(m) => member(
            &m.object,
            &format!(
                "[{}]",
                &source[m.expression.span().start as usize..m.expression.span().end as usize]
            ),
            m.optional,
            source,
            fresh,
            continuation,
        ),
        Expression::CallExpression(c) if !c.optional => {
            chain(&c.callee, source, fresh, &|value, fresh| {
                let args = &source[c.callee.span().end as usize..c.span.end as usize];
                continuation(format!("{value}{args}"), fresh)
            })
        }
        Expression::CallExpression(c) if c.optional => eyre::bail!(
            "unsupported nested optional PURE call at bytes {}..{}",
            c.span.start,
            c.span.end
        ),
        _ => continuation(
            source[expression.span().start as usize..expression.span().end as usize].into(),
            fresh,
        ),
    }
}

fn member(
    object: &oxc_ast::ast::Expression<'_>,
    suffix: &str,
    optional: bool,
    source: &str,
    fresh: &mut Fresh,
    continuation: &Continuation<'_>,
) -> eyre::Result<String> {
    chain(object, source, fresh, &|receiver, fresh| {
        if optional {
            let name = fresh.name();
            let body = continuation(format!("{name}{suffix}"), fresh)?;
            Ok(format!(
                "({name}=>{name}===null||{name}===void 0?void 0:({body}))({receiver})"
            ))
        } else {
            continuation(format!("{receiver}{suffix}"), fresh)
        }
    })
}
