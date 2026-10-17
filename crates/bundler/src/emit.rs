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

pub(crate) struct SourcePart {
    /// None is structural source (preamble, trivia or evaluation-only imports).
    pub segment: Option<u32>,
    pub source: String,
}

/// Keep one lexical module while storing its rewritten pieces in exact chunks.
pub(crate) fn project_parts(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    source: &str,
    parsed: &ParsedModule,
    dependency_edits: &[Edit],
) -> eyre::Result<Vec<SourcePart>> {
    let edits = projection_edits(graph, retention, file, source, parsed, dependency_edits)?;
    let mut parts = Vec::new();
    let mut end = 0;
    for (index, segment) in parsed.segments.iter().enumerate() {
        parts.push(SourcePart {
            segment: None,
            source: source[end..segment.span.start as usize].into(),
        });
        let local: Vec<_> = edits
            .iter()
            .filter(|e| contains(segment.span, e.span))
            .map(|e| Edit {
                span: Span::new(
                    e.span.start - segment.span.start,
                    e.span.end - segment.span.start,
                ),
                replacement: e.replacement.clone(),
            })
            .collect();
        let code = rewrite::apply(
            &source[segment.span.start as usize..segment.span.end as usize],
            &local,
        )?;
        parts.push(SourcePart {
            segment: retention
                .segments
                .iter()
                .any(|s| s.key == SegmentKey::new(file, index as u32))
                .then_some(index as u32),
            source: code,
        });
        end = segment.span.end as usize;
    }
    parts.push(SourcePart {
        segment: None,
        source: source[end..].into(),
    });
    let joined = parts.iter().map(|p| p.source.as_str()).collect::<String>();
    rewrite::validate(graph.file_path(file).unwrap(), &joined)?;
    let hints = rewrite::source_hint_edits(&joined);
    let mut start = 0;
    for part in &mut parts {
        let end = start + part.source.len() as u32;
        let local: Vec<_> = hints
            .iter()
            .filter(|e| contains(Span::new(start, end), e.span))
            .map(|e| Edit {
                span: Span::new(e.span.start - start, e.span.end - start),
                replacement: e.replacement.clone(),
            })
            .collect();
        part.source = rewrite::apply(&part.source, &local)?;
        start = end;
    }
    Ok(parts)
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
    let mut fresh = Fresh::new(&parsed.facts);
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
        let replacement =
            render_range(source, segment.span, rewrites, &mut fresh).wrap_err_with(|| {
                format!(
                    "{}: PURE rewrite at bytes {}..{}",
                    path.display(),
                    segment.span.start,
                    segment.span.end
                )
            })?;
        // Dependency edits and PURE invocation edits must not overlap.
        if rewrites.is_empty() {
            edits.extend(
                dependency_edits
                    .iter()
                    .filter(|e| contains(segment.span, e.span))
                    .cloned(),
            );
        } else {
            ensure!(!dependency_edits.iter().any(|e| contains(segment.span, e.span)),
                "{}: dependency inside PURE-rewritten statement needs composed linking at bytes {}..{}",
                path.display(), segment.span.start, segment.span.end);
            edits.push(Edit {
                span: segment.span,
                replacement,
            });
        }
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

fn render_range(
    source: &str,
    range: Span,
    recipes: &[crate::effects::InvocationRewrite],
    fresh: &mut Fresh,
) -> eyre::Result<String> {
    let mut edits = Vec::new();
    let mut end = range.start;
    for recipe in recipes.iter().filter(|r| contains(range, r.invocation)) {
        if recipe.invocation.start < end {
            continue;
        }
        let callee = render_range(source, recipe.callee, recipes, fresh)?;
        let mut arguments = Vec::new();
        for argument in &recipe.arguments {
            let value = render_range(source, argument.span, recipes, fresh)?;
            arguments.push(if argument.spread {
                format!("[...({value})]")
            } else {
                format!("({value})")
            });
        }
        arguments.push("void 0".into());
        let tail = arguments.join(",");
        let replacement = match recipe.guard {
            crate::effects::ArgumentGuard::Always => format!("(({callee}),{tail})"),
            crate::effects::ArgumentGuard::OriginalChain { optional_call } => {
                pure_chain(&callee, optional_call, &tail, fresh)?
            }
        };
        edits.push(Edit {
            span: Span::new(
                recipe.invocation.start - range.start,
                recipe.invocation.end - range.start,
            ),
            replacement,
        });
        end = recipe.invocation.end;
    }
    rewrite::apply(&source[range.start as usize..range.end as usize], &edits)
}

fn pure_chain(callee: &str, optional: bool, tail: &str, fresh: &mut Fresh) -> eyre::Result<String> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    let allocator = Allocator::default();
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
