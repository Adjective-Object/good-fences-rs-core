//! Retention-driven JavaScript ESM emission. Publication is the caller's job.
use std::collections::BTreeSet;

use ast_segmenter::{owned::DependencyKind, ModuleBody, ModuleFormat, ParsedModule};
use eyre::{ensure, OptionExt, WrapErr};
use oxc_span::Span;
use source_graph::{SegmentKey, SourceGraph};

use crate::{
    assembly::{self, Code, Edit},
    policy::EffectPolicy,
    rewrite, RetentionPlan,
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
        parsed.body() == ModuleBody::Esm,
        "single-module emission requires ESM; use link::esm for CJS"
    );
    if let Some(dependency) = parsed
        .facts()
        .dependencies()
        .iter()
        .find(|d| !d.type_only())
    {
        eyre::bail!(
            "{}: dependency needs linked emission at bytes {}..{} ({:?})",
            graph.file_path(file).unwrap().display(),
            dependency.span().start,
            dependency.span().end,
            dependency.kind()
        );
    }
    ensure!(
        retention.modules == [file],
        "single-module emission needs one evaluated module"
    );
    let path = graph.file_path(file).ok_or_eyre("missing emission path")?;
    let output = project(graph, retention, file, source, &parsed)?.render(
        path,
        path,
        &Default::default(),
    )?;
    rewrite::validate(path, &output)?;
    Ok(output)
}

pub(crate) fn checked_source(
    graph: &SourceGraph,
    file: u32,
    source: &str,
) -> eyre::Result<ParsedModule> {
    let path = graph
        .file_path(file)
        .ok_or_eyre("missing emission module")?;
    let format = if graph.file_facts(file).is_some_and(|f| f.commonjs()) {
        ModuleFormat::CommonJs
    } else {
        ModuleFormat::Esm
    };
    let provenance = graph.file_provenance(file).ok_or_else(|| {
        eyre::eyre!(
            "{}: missing source extraction provenance; reparse and rebuild before emission",
            path.display()
        )
    })?;
    let config = if let Some(prepared) = graph.file_prepared_resource(file) {
        prepared.config(path, provenance)?
    } else {
        ast_segmenter::ParserConfig::for_path(&crate::resource::parser_path(path), format)?
    };
    ensure!(
        provenance.config() == config,
        "{}: source parser configuration mismatch; reparse and rebuild before emission",
        path.display()
    );
    ensure!(
        config.commonjs() == (format == ModuleFormat::CommonJs),
        "{}: source parser configuration mismatch with module facts",
        path.display()
    );
    ensure!(
        !config.typescript() && !config.jsx(),
        "{}: emission supports JavaScript only at bytes 0..{}; TS/JSX need later backends",
        path.display(),
        source.len()
    );
    let facts = graph
        .file_facts(file)
        .ok_or_eyre("missing emission facts")?;
    let segments = graph
        .file_segments(file)
        .ok_or_eyre("missing emission ranges")?;
    let body = if config.commonjs() {
        ModuleBody::CommonJs {
            span: segments.first().ok_or_eyre("missing opaque CJS body")?.span,
        }
    } else {
        ModuleBody::Esm
    };
    ParsedModule::from_provenance(
        source,
        provenance.clone(),
        body,
        segments.to_vec(),
        facts.clone(),
    )
    .wrap_err_with(|| {
        format!(
            "{}: source facts/ranges do not match graph; reparse and rebuild before emission",
            path.display()
        )
    })
}

fn project(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    source: &str,
    parsed: &ParsedModule,
) -> eyre::Result<Code> {
    let edits = projection_edits(graph, retention, file, source, parsed)?;
    assembly::apply(source, &edits)
}

fn projection_edits(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    file: u32,
    source: &str,
    parsed: &ParsedModule,
) -> eyre::Result<Vec<Edit>> {
    let path = graph.file_path(file).ok_or_eyre("missing emission path")?;
    let live: BTreeSet<_> = retention
        .segments
        .iter()
        .filter(|s| s.key.file_id == file)
        .map(|s| s.key.segment_idx)
        .collect();
    let mut edits = Vec::new();
    for (index, segment) in parsed.segments().iter().enumerate() {
        let key = SegmentKey::new(file, index as u32);
        let recipe = &parsed.facts().emission().segments()[index];
        let edit_span = if recipe.source().as_slice().len() > 1 {
            let prefix = recipe.source().as_slice()[0];
            let start = if index > 0
                && parsed.facts().emission().segments()[index - 1]
                    .source()
                    .as_slice()
                    .first()
                    == Some(&prefix)
            {
                parsed.segments()[index - 1].span.end
            } else {
                prefix.start
            };
            Span::new(start, segment.span.end)
        } else {
            segment.span
        };
        if !live.contains(&(index as u32)) {
            edits.push(Edit {
                span: edit_span,
                replacement: Code::default(),
            });
            continue;
        }
        let rewrites = retention
            .effects
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.summary.rewrites.as_slice())
            .unwrap_or_default();
        let mut renderer = CachedRenderer::new(source, &parsed.facts(), recipe, rewrites);
        let replacement = renderer.materialize().wrap_err_with(|| {
            format!(
                "{}: PURE rewrite at bytes {}..{}",
                path.display(),
                segment.span.start,
                segment.span.end
            )
        })?;
        edits.push(Edit {
            span: edit_span,
            replacement,
        });
    }
    Ok(edits)
}

struct Fresh {
    names: BTreeSet<String>,
    next: usize,
}

impl Fresh {
    fn new(facts: &ast_segmenter::owned::ModuleFacts) -> Self {
        Self {
            names: facts
                .bindings()
                .iter()
                .map(|b| b.name().to_owned())
                .chain(facts.references().iter().map(|r| r.name().to_owned()))
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
    replacements: std::collections::BTreeMap<u32, Code>,
    edits: std::collections::BTreeMap<u32, Vec<Edit>>,
    contexts: std::collections::BTreeMap<u32, (Span, crate::context::Dispatch)>,
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
            contexts: Default::default(),
            fresh: Fresh::new(facts),
        }
    }

    pub(crate) fn replace(&mut self, operation: u32, replacement: Code) {
        self.replacements.insert(operation, replacement);
    }

    pub(crate) fn relocate_urls(
        &mut self,
        relocations: &[crate::css_loading::UrlRelocation],
    ) -> eyre::Result<()> {
        for relocation in relocations {
            if !self
                .recipe
                .source()
                .as_slice()
                .iter()
                .any(|span| crate::effects::contains(*span, relocation.operation))
            {
                continue;
            }
            let index = self
                .recipe
                .operations()
                .iter()
                .position(|operation| {
                    matches!(operation.kind(),
                        ast_segmenter::emission::OperationKind::Invocation { invocation, .. }
                        if self.facts.invocations()[*invocation as usize].span() == relocation.operation
                            && self.facts.invocations()[*invocation as usize].construct()
                    )
                })
                .ok_or_eyre(
                    "generated URL relocation does not match a cached construction operation",
                )?;
            self.replace(
                index as u32,
                Code::build_root_url(relocation.public_url.clone()),
            );
        }
        Ok(())
    }

    pub(crate) fn edit(&mut self, operation: u32, edit: Edit) {
        self.edits.entry(operation).or_default().push(edit);
    }

    pub(crate) fn context(
        &mut self,
        dependency: &ast_segmenter::owned::Dependency,
        dispatch: crate::context::Dispatch,
    ) {
        if let Some(index) = self.recipe.operations().iter().position(|operation| matches!(
            operation.kind(), ast_segmenter::emission::OperationKind::Dependency { dependency: id } if *id == dependency.id()
        )) {
            self.contexts.insert(index as u32, (dependency.specifier_span(), dispatch));
        }
    }

    pub(crate) fn dependency(
        &mut self,
        dependency: &ast_segmenter::owned::Dependency,
        replacement: Code,
        whole: bool,
    ) {
        use ast_segmenter::emission::{ExportForm, OperationKind};
        let operation =
            self.recipe
                .operations()
                .iter()
                .position(|operation| match operation.kind() {
                    OperationKind::Dependency { dependency: id } => *id == dependency.id(),
                    // Reexports cache their removable wrapper, which is the request occurrence.
                    OperationKind::Export {
                        span,
                        form: ExportForm::Remove,
                    } => {
                        dependency.kind() == DependencyKind::ReExport && *span == dependency.span()
                    }
                    _ => false,
                });
        if let Some(index) = operation {
            if whole {
                self.replace(index as u32, replacement);
            } else {
                self.edit(
                    index as u32,
                    Edit {
                        span: dependency.specifier_span(),
                        replacement,
                    },
                );
            }
        }
    }

    pub(crate) fn render(&mut self, range: Span) -> eyre::Result<Code> {
        self.children(range, None)
    }

    pub(crate) fn materialize(&mut self) -> eyre::Result<Code> {
        let mut output = Code::default();
        for &piece in self.recipe.source().as_slice() {
            output.push(self.render(piece)?);
        }
        if self.recipe.source().as_slice().len() > 1 {
            output.push_str(";\n");
        }
        Ok(output)
    }

    pub(crate) fn slot(
        &mut self,
        slot: &ast_segmenter::emission::SourceSlot,
    ) -> eyre::Result<Code> {
        self.roots(slot.span(), &slot.operations(), Vec::new())
    }

    pub(crate) fn children(&mut self, range: Span, parent: Option<u32>) -> eyre::Result<Code> {
        let roots: Vec<_> = self
            .recipe
            .operations()
            .iter()
            .enumerate()
            .filter(|(_, operation)| {
                operation.parent() == parent
                    && crate::link::contains(range, operation.kind().span(self.facts, self.recipe))
            })
            .map(|(index, _)| index as u32)
            .collect();
        let edits = parent
            .and_then(|index| self.edits.get(&index))
            .cloned()
            .unwrap_or_default();
        self.roots(range, &roots, edits)
    }

    fn roots(&mut self, range: Span, roots: &[u32], mut edits: Vec<Edit>) -> eyre::Result<Code> {
        for &index in roots {
            let operation = &self.recipe.operations()[index as usize];
            let span = operation.kind().span(self.facts, self.recipe);
            edits.push(Edit {
                span,
                replacement: self.operation(index)?,
            });
        }
        assembly::apply_range(self.source, range, &edits)
    }

    fn operation(&mut self, index: u32) -> eyre::Result<Code> {
        use ast_segmenter::emission::OperationKind;
        if let Some(replacement) = self.replacements.get(&index) {
            return Ok(replacement.clone());
        }
        if let Some((span, dispatch)) = self.contexts.get(&index).cloned() {
            let argument = self.children(span, Some(index))?;
            return Ok(dispatch.emit(argument));
        }
        let operation = &self.recipe.operations()[index as usize];
        let span = operation.kind().span(self.facts, self.recipe);
        if let OperationKind::SourceHint { .. } = operation.kind() {
            return Ok(Code::default());
        }
        if let OperationKind::Invocation {
            invocation,
            ref lowering,
        } = operation.kind()
        {
            if let Some(recipe) = self.rewrites.iter().find(|rewrite| {
                rewrite.invocation == self.facts.invocations()[*invocation as usize].span()
            }) {
                if !matches!(
                    recipe.guard,
                    crate::effects::ArgumentGuard::OriginalChain { .. }
                ) || !lowering.crosses_await_guard()
                {
                    let mut tail = Code::default();
                    for (slot, argument) in lowering.arguments().iter().zip(&recipe.arguments) {
                        let value = self.slot(slot)?;
                        tail.push(if argument.spread() {
                            value.surrounded("[...(", ")],")
                        } else {
                            value.surrounded("(", "),")
                        });
                    }
                    tail.push_str("void 0");
                    return match recipe.guard {
                        crate::effects::ArgumentGuard::Always => {
                            let callee = self.slot(&lowering.callee())?;
                            let mut output = callee.surrounded("((", "),");
                            output.push(tail);
                            output.push_str(")");
                            Ok(output)
                        }
                        crate::effects::ArgumentGuard::OriginalChain { optional_call } => {
                            let chain = lowering.chain().ok_or_eyre(
                                "unsupported nested optional call/private PURE callee",
                            )?;
                            let base = self.slot(&chain.base())?;
                            self.chain(&chain.steps(), base, optional_call, &tail)
                        }
                    };
                }
            }
        }
        self.children(span, Some(index))
    }

    fn chain(
        &mut self,
        steps: &[ast_segmenter::emission::ChainStep],
        mut value: Code,
        optional_call: bool,
        tail: &Code,
    ) -> eyre::Result<Code> {
        use ast_segmenter::emission::ChainStep;
        let Some((step, rest)) = steps.split_first() else {
            if optional_call {
                let name = self.fresh.name();
                let mut output = tail.clone().surrounded(
                    format!("({name}=>{name}===null||{name}===void 0?void 0:("),
                    "))(",
                );
                output.push(value);
                output.push_str(")");
                return Ok(output);
            } else {
                let mut output = value.surrounded("((", "),");
                output.push(tail.clone());
                output.push_str(")");
                return Ok(output);
            }
        };
        let (suffix, optional) = match step {
            ChainStep::Static { property, optional } => {
                (Code::from(format!(".{property}")), *optional)
            }
            ChainStep::Computed { property, optional } => {
                (self.slot(property)?.surrounded("[", "]"), *optional)
            }
            ChainStep::Call { suffix } => (self.slot(suffix)?, false),
        };
        if optional {
            let name = self.fresh.name();
            let mut receiver = Code::from(name.clone());
            receiver.push(suffix);
            let body = self.chain(rest, receiver, optional_call, tail)?;
            let mut output = body.surrounded(
                format!("({name}=>{name}===null||{name}===void 0?void 0:("),
                "))(",
            );
            output.push(value);
            output.push_str(")");
            Ok(output)
        } else {
            value.push(suffix);
            self.chain(rest, value, optional_call, tail)
        }
    }
}
