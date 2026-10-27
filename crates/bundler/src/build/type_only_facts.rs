//! Original TypeScript evidence only. No graph resolution or executable rewrites.
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use ast_segmenter::ModuleFormat;
use cache_flatbuffers::{hash128, type_only_wire as wire, SharedBytes, WireTypeOnlyFacts};
use eyre::{ensure, WrapErr};
use ftree_cache::inputs::PathInput;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_ecmascript::BoundNames;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, Span};
use oxc_syntax::symbol::SymbolId;
use segment_cache::CacheMode;

mod enums;
#[cfg(test)]
mod tests;

/// Selected TypeScript syntax is independent of the physical source extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    typescript_jsx: bool,
    declaration: bool,
    package: ModuleFormat,
    commonjs: bool,
    detect_esm: bool,
}

impl Settings {
    /// Use the resolved original parser config, before transformation.
    pub fn from_config(
        config: ast_segmenter::ParserConfig,
        package: ModuleFormat,
    ) -> eyre::Result<Self> {
        ensure!(
            config.typescript(),
            "TypeOnlyFacts requires selected TypeScript syntax"
        );
        Ok(Self {
            typescript_jsx: config.jsx(),
            declaration: config.definition(),
            package,
            commonjs: config.commonjs(),
            detect_esm: config.detect_esm(),
        })
    }

    fn identity(self) -> [u8; 16] {
        self.identity_for(enums::Seed::Semantic)
    }

    fn identity_for(self, seed: enums::Seed) -> [u8; 16] {
        static IMPLEMENTATION: std::sync::LazyLock<[u8; 16]> = std::sync::LazyLock::new(|| {
            let mut bytes = include_bytes!("type_only_facts.rs").to_vec();
            bytes.extend_from_slice(include_bytes!("type_only_facts/enums.rs"));
            bytes.extend_from_slice(b"oxc-73e6b564737a29ecebf428d276421dcc492836df");
            hash128(&bytes)
        });
        let mut bytes = IMPLEMENTATION.to_vec();
        bytes.extend_from_slice(
            &(cache_flatbuffers::TYPE_ONLY_STRING_VALUE_BYTES as u64).to_be_bytes(),
        );
        bytes.extend_from_slice(
            &(cache_flatbuffers::TYPE_ONLY_STRING_TOTAL_BYTES as u64).to_be_bytes(),
        );
        bytes.extend_from_slice(&[
            u8::from(self.typescript_jsx),
            u8::from(self.declaration),
            u8::from(self.package == ModuleFormat::CommonJs),
            u8::from(self.commonjs),
            u8::from(self.detect_esm),
            u8::from(seed == enums::Seed::Bounded),
        ]);
        hash128(&bytes)
    }
}

fn span(s: Span) -> wire::SpanT {
    wire::SpanT {
        start: s.start,
        end: s.end,
    }
}

fn name(value: &str, location: Span) -> Box<wire::NameT> {
    let mut n = wire::NameT::default();
    n.value = value.to_owned();
    n.span = span(location);
    Box::new(n)
}

fn module_name(n: &ModuleExportName<'_>) -> eyre::Result<Box<wire::NameT>> {
    let value = n.name();
    Ok(name(value.as_str(), n.span()))
}

fn request(n: &StringLiteral<'_>) -> eyre::Result<Box<wire::NameT>> {
    Ok(name(
        n.value
            .as_str()
            .ok_or_else(|| eyre::eyre!("non-UTF-8 TypeScript module request"))?,
        n.span,
    ))
}

/// Extract from an already parsed original AST and its checked semantic data.
/// Build semantics with `with_enum_eval(true)`; node storage is unnecessary.
/// This neither reparses nor mutates the program/scoping used by the transformer.
pub fn extract(
    directory: &Path,
    program: &Program<'_>,
    semantic: &Semantic<'_>,
    settings: Settings,
) -> eyre::Result<WireTypeOnlyFacts> {
    extract_inner(
        directory,
        program,
        semantic,
        settings,
        enums::Seed::Semantic,
    )
}

fn extract_inner(
    directory: &Path,
    program: &Program<'_>,
    semantic: &Semantic<'_>,
    settings: Settings,
    seed: enums::Seed,
) -> eyre::Result<WireTypeOnlyFacts> {
    check_program(program, settings)?;
    ensure!(
        std::ptr::eq(semantic.source_text(), program.source_text)
            && semantic.source_type() == &program.source_type,
        "TypeOnlyFacts semantic data does not belong to the original program"
    );
    let mut extraction = Extraction {
        semantic,
        source: program.source_text,
        facts: wire::FactsT::default(),
        imports: HashMap::new(),
        type_query: false,
    };
    for statement in &program.body {
        extraction.statement(statement)?;
    }
    enums::extract(program, semantic, &mut extraction.facts, seed)?;
    extraction.visit_program(program);
    extraction.finish();
    WireTypeOnlyFacts::encode_in(
        directory,
        program.source_text,
        settings.identity_for(seed),
        extraction.facts,
    )
    .wrap_err("sealing original TypeScript facts")
}

fn check_program(program: &Program<'_>, settings: Settings) -> eyre::Result<()> {
    ensure!(
        program.source_type.is_typescript(),
        "cannot extract TypeOnlyFacts from JavaScript"
    );
    ensure!(
        program.source_type.is_typescript_definition() == settings.declaration
            && program.source_type.is_jsx() == settings.typescript_jsx
            && program.source_type.is_commonjs() == settings.commonjs,
        "TypeOnlyFacts selected syntax mismatch"
    );
    Ok(())
}

struct Extraction<'s, 'a> {
    semantic: &'s Semantic<'a>,
    source: &'a str,
    facts: wire::FactsT,
    imports: HashMap<SymbolId, usize>,
    type_query: bool,
}

impl<'s, 'a> Extraction<'s, 'a> {
    fn local(
        &mut self,
        id: &BindingIdentifier<'_>,
        owner: Span,
        presence: wire::Presence,
        has_type: bool,
        ambient: bool,
    ) {
        let mut l = wire::LocalT::default();
        l.name = name(id.name.as_str(), id.span);
        l.span = span(owner);
        l.presence = presence;
        l.has_type = has_type;
        l.ambient = ambient;
        self.facts.locals.push(l);
    }

    fn declaration(
        &mut self,
        declaration: &Declaration<'a>,
        owner: Span,
        exported: bool,
    ) -> eyre::Result<()> {
        let before = self.facts.locals.len();
        match declaration {
            Declaration::TSInterfaceDeclaration(d) => {
                self.local(&d.id, owner, wire::Presence::OnlyType, true, d.declare)
            }
            Declaration::TSTypeAliasDeclaration(d) => {
                self.local(&d.id, owner, wire::Presence::OnlyType, true, d.declare)
            }
            Declaration::ClassDeclaration(d) => {
                if let Some(id) = &d.id {
                    self.local(id, owner, wire::Presence::RuntimeValue, true, d.declare);
                }
            }
            Declaration::FunctionDeclaration(d) => {
                if let Some(id) = &d.id {
                    self.local(id, owner, wire::Presence::RuntimeValue, false, d.declare);
                }
            }
            Declaration::VariableDeclaration(d) => {
                d.bound_names(&mut |id| {
                    self.local(id, owner, wire::Presence::RuntimeValue, false, d.declare)
                });
            }
            Declaration::TSEnumDeclaration(d) => {
                self.local(
                    &d.id,
                    owner,
                    if d.r#const {
                        wire::Presence::ConstEnum
                    } else {
                        wire::Presence::RuntimeValue
                    },
                    true,
                    d.declare,
                );
                // Private declarations remain available to exported aliases.
            }
            Declaration::TSNamespaceDeclaration(d) => {
                self.local(&d.id, owner, wire::Presence::RuntimeValue, true, d.declare);
                self.unsupported(
                    owner,
                    "nested namespace members are not top-level module facts",
                );
            }
            Declaration::TSExternalModuleDeclaration(_) => self.unsupported(
                owner,
                "ambient external module declarations require module-scoped metadata",
            ),
            Declaration::TSGlobalDeclaration(_) => {
                self.unsupported(owner, "global augmentation is not a top-level export")
            }
            Declaration::TSImportEqualsDeclaration(d) => {
                self.import_equals(d)?;
            }
        }
        if exported {
            for index in before..self.facts.locals.len() {
                let local = &self.facts.locals[index];
                let mut e = wire::ExportT::default();
                e.name = local.name.clone();
                e.local = Some(local.name.clone());
                e.span = span(owner);
                e.presence = local.presence;
                e.has_type = local.has_type;
                self.facts.exports.push(e);
            }
        }
        Ok(())
    }

    fn unsupported(&mut self, owner: Span, message: &str) {
        let mut d = wire::DiagnosticT::default();
        d.kind = wire::DiagnosticKind::UnsupportedDeclaration;
        d.span = span(owner);
        d.message = message.into();
        self.facts.diagnostics.push(d);
    }

    fn export(&mut self, e: wire::ExportT, kind: wire::OccurrenceKind) {
        let mut occurrence = wire::OccurrenceT::default();
        occurrence.kind = kind;
        occurrence.span = e.span.clone();
        occurrence.request = e.request.clone();
        occurrence.imported = e.imported.clone();
        occurrence.local = e.local.clone();
        occurrence.exported = Some(e.name.clone());
        occurrence.explicit_type = e.explicit_type;
        self.facts.occurrences.push(occurrence);
        self.facts.exports.push(e);
    }

    fn import(&mut self, d: &ImportDeclaration<'a>) -> eyre::Result<()> {
        if let Some(specifiers) = &d.specifiers {
            for specifier in specifiers {
                let (local, imported, explicit) = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => (
                        &s.local,
                        module_name(&s.imported)?,
                        s.import_kind == ImportOrExportKind::Type,
                    ),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        (&s.local, name("default", s.local.span), false)
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (
                        &s.local,
                        name("*", keyword(self.source, s.span, "*")?),
                        false,
                    ),
                };
                let mut o = wire::OccurrenceT::default();
                o.kind = wire::OccurrenceKind::Import;
                o.span = span(d.span);
                o.request = Some(request(&d.source)?);
                o.imported = Some(imported);
                o.local = Some(name(local.name.as_str(), local.span));
                o.explicit_type = explicit || d.import_kind == ImportOrExportKind::Type;
                if let Some(symbol) = local.symbol_id.get() {
                    self.imports.insert(symbol, self.facts.occurrences.len());
                }
                self.facts.occurrences.push(o);
                self.local(
                    local,
                    d.span,
                    if explicit || d.import_kind == ImportOrExportKind::Type {
                        wire::Presence::OnlyType
                    } else {
                        wire::Presence::Unresolved
                    },
                    explicit || d.import_kind == ImportOrExportKind::Type,
                    false,
                );
            }
        }
        if d.specifiers.as_ref().is_none_or(|s| s.is_empty()) {
            let mut o = wire::OccurrenceT::default();
            o.kind = wire::OccurrenceKind::Import;
            o.span = span(d.span);
            o.request = Some(request(&d.source)?);
            o.explicit_type = d.import_kind == ImportOrExportKind::Type;
            self.facts.occurrences.push(o);
        }
        Ok(())
    }

    fn import_equals(&mut self, d: &TSImportEqualsDeclaration<'a>) -> eyre::Result<()> {
        self.local(
            &d.id,
            d.span,
            if d.import_kind == ImportOrExportKind::Type {
                wire::Presence::OnlyType
            } else {
                wire::Presence::Unresolved
            },
            d.import_kind == ImportOrExportKind::Type,
            false,
        );
        if let TSModuleReference::ExternalModuleReference(reference) = &d.module_reference {
            let mut o = wire::OccurrenceT::default();
            o.kind = wire::OccurrenceKind::ImportEquals;
            o.span = span(d.span);
            o.request = Some(request(&reference.expression)?);
            o.imported = Some(name("*", reference.expression.span));
            o.local = Some(name(d.id.name.as_str(), d.id.span));
            o.explicit_type = d.import_kind == ImportOrExportKind::Type;
            if let Some(symbol) = d.id.symbol_id.get() {
                self.imports.insert(symbol, self.facts.occurrences.len());
            }
            self.facts.occurrences.push(o);
        } else {
            self.unsupported(
                d.span,
                "qualified import-equals alias requires namespace metadata",
            );
        }
        Ok(())
    }

    fn statement(&mut self, s: &Statement<'a>) -> eyre::Result<()> {
        match s {
            Statement::ExportDeclaration(d) => self.declaration(&d.declaration, d.span, true)?,
            Statement::ExportNamedDeclaration(d) => {
                for s in &d.specifiers {
                    let mut e = wire::ExportT::default();
                    e.name = module_name(&s.exported)?;
                    e.local = Some(module_name(&s.local)?);
                    e.span = span(d.span);
                    e.presence = wire::Presence::Unresolved;
                    e.explicit_type = d.export_kind == ImportOrExportKind::Type
                        || s.export_kind == ImportOrExportKind::Type;
                    self.export(e, wire::OccurrenceKind::LocalExport);
                }
            }
            Statement::ExportFromDeclaration(d) => {
                for s in &d.specifiers {
                    let mut e = wire::ExportT::default();
                    e.name = module_name(&s.exported)?;
                    e.imported = Some(module_name(&s.local)?);
                    e.request = Some(request(&d.source)?);
                    e.span = span(d.span);
                    e.presence = wire::Presence::Unresolved;
                    e.explicit_type = d.export_kind == ImportOrExportKind::Type
                        || s.export_kind == ImportOrExportKind::Type;
                    self.export(e, wire::OccurrenceKind::Reexport);
                }
                if d.specifiers.is_empty() {
                    let mut o = wire::OccurrenceT::default();
                    o.kind = wire::OccurrenceKind::Reexport;
                    o.span = span(d.span);
                    o.request = Some(request(&d.source)?);
                    o.explicit_type = d.export_kind == ImportOrExportKind::Type;
                    self.facts.occurrences.push(o);
                }
            }
            Statement::ExportAllDeclaration(d) => {
                let mut e = wire::ExportT::default();
                e.name = match &d.exported {
                    Some(n) => module_name(n)?,
                    None => name("*", keyword(self.source, d.span, "*")?),
                };
                e.imported = Some(name("*", keyword(self.source, d.span, "*")?));
                e.request = Some(request(&d.source)?);
                e.span = span(d.span);
                e.star = true;
                e.explicit_type = d.export_kind == ImportOrExportKind::Type;
                // Namespace reexports are real namespace values, unlike star routes.
                e.presence = if d.exported.is_some() {
                    wire::Presence::RuntimeValue
                } else {
                    wire::Presence::Unresolved
                };
                self.export(e, wire::OccurrenceKind::Reexport);
            }
            Statement::ExportDefaultDeclaration(d) => {
                let mut e = wire::ExportT::default();
                e.name = name("default", keyword(self.source, d.span, "default")?);
                e.span = span(d.span);
                e.presence = wire::Presence::RuntimeValue;
                match &d.declaration {
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
                        self.local(&i.id, d.span, wire::Presence::OnlyType, true, i.declare);
                        e.local = Some(name(i.id.name.as_str(), i.id.span));
                        e.presence = wire::Presence::OnlyType;
                        e.has_type = true;
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        if let Some(id) = &c.id {
                            self.local(id, d.span, wire::Presence::RuntimeValue, true, c.declare);
                            e.local = Some(name(id.name.as_str(), id.span));
                        }
                        e.has_type = true;
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        if let Some(id) = &f.id {
                            self.local(id, d.span, wire::Presence::RuntimeValue, false, f.declare);
                            e.local = Some(name(id.name.as_str(), id.span));
                        }
                    }
                    ExportDefaultDeclarationKind::Identifier(id) => {
                        e.local = Some(name(id.name.as_str(), id.span));
                        e.presence = wire::Presence::Unresolved;
                    }
                    _ => {}
                }
                self.export(e, wire::OccurrenceKind::LocalExport);
            }
            Statement::ImportDeclaration(d) => self.import(d)?,
            Statement::TSExportAssignment(d) => {
                let mut e = wire::ExportT::default();
                e.name = name("export=", d.span);
                e.span = span(d.span);
                e.presence = wire::Presence::RuntimeValue;
                if let Expression::Identifier(id) = &d.expression {
                    e.local = Some(name(id.name.as_str(), id.span));
                    e.presence = wire::Presence::Unresolved;
                }
                self.export(e, wire::OccurrenceKind::ExportAssignment);
            }
            _ => {
                if let Some(declaration) = s.as_declaration() {
                    self.declaration(declaration, s.span(), false)?;
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) {
        // A type declaration never overwrites a real value in a merged identity.
        let mut locals: HashMap<&str, (wire::Presence, bool)> = HashMap::new();
        for l in &self.facts.locals {
            locals
                .entry(&l.name.value)
                .and_modify(|(p, t)| {
                    *p = merge(*p, l.presence);
                    *t |= l.has_type;
                })
                .or_insert((l.presence, l.has_type));
        }
        for e in &mut self.facts.exports {
            if e.explicit_type {
                e.presence = wire::Presence::OnlyType;
                e.has_type = true;
            } else if let Some((presence, has_type)) =
                e.local.as_ref().and_then(|n| locals.get(n.value.as_str()))
            {
                e.presence = *presence;
                e.has_type |= *has_type;
            }
            for occurrence in &mut self.facts.occurrences {
                occurrence.uses.sort_by_key(|u| (u.span.start, u.span.end));
            }
        }
        self.facts
            .locals
            .sort_by_key(|l| (l.span.start, l.name.span.start));
        self.facts
            .exports
            .sort_by_key(|e| (e.span.start, e.name.span.start));
        self.facts
            .occurrences
            .sort_by_key(|o| (o.span.start, o.local.as_ref().map_or(0, |n| n.span.start)));
        self.facts.const_enums.sort_by_key(|e| e.span.start);
        self.facts.diagnostics.sort_by_key(|d| d.span.start);
    }
}

fn merge(a: wire::Presence, b: wire::Presence) -> wire::Presence {
    if a == wire::Presence::RuntimeValue || b == wire::Presence::RuntimeValue {
        wire::Presence::RuntimeValue
    } else if a == wire::Presence::ConstEnum || b == wire::Presence::ConstEnum {
        wire::Presence::ConstEnum
    } else if a == wire::Presence::Unresolved || b == wire::Presence::Unresolved {
        wire::Presence::Unresolved
    } else {
        wire::Presence::OnlyType
    }
}

impl<'a> Visit<'a> for Extraction<'_, 'a> {
    fn visit_ts_import_type(&mut self, d: &TSImportType<'a>) {
        if let Some(source) = d.source.value.as_str() {
            let mut occurrence = wire::OccurrenceT::default();
            occurrence.kind = wire::OccurrenceKind::TypeImport;
            occurrence.span = span(d.span);
            occurrence.request = Some(name(source, d.source.span));
            occurrence.explicit_type = true;
            let mut qualifier = d.qualifier.as_ref();
            while let Some(TSImportTypeQualifier::QualifiedName(q)) = qualifier {
                qualifier = Some(&q.left);
            }
            if let Some(TSImportTypeQualifier::Identifier(id)) = qualifier {
                occurrence.imported = Some(name(id.name.as_str(), id.span));
            }
            self.facts.occurrences.push(occurrence);
        } else {
            self.unsupported(d.span, "non-UTF-8 import-type request");
        }
        walk::walk_ts_import_type(self, d);
    }
    fn visit_export_named_declaration(&mut self, _: &ExportNamedDeclaration<'a>) {
        // Named export connectors are routes, not executable identifier uses.
    }
    fn visit_ts_type_query_expr_name(&mut self, query: &TSTypeQueryExprName<'a>) {
        let previous = std::mem::replace(&mut self.type_query, true);
        walk::walk_ts_type_query_expr_name(self, query);
        self.type_query = previous;
    }
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if let Some(reference) = id
            .reference_id
            .get()
            .map(|id| self.semantic.scoping().get_reference(id))
        {
            let symbol = reference.symbol_id().or_else(|| {
                self.semantic
                    .scoping()
                    .find_binding(reference.scope_id(), id.name)
            });
            if let Some(index) = symbol.and_then(|id| self.imports.get(&id)) {
                let flags = reference.flags();
                if reference.is_type() || reference.is_value() || flags.is_value_as_type() {
                    let mut usage = wire::UseT::default();
                    usage.span = span(id.span);
                    usage.type_use = reference.is_type() || self.type_query;
                    usage.value_use =
                        reference.is_value() && !flags.is_value_as_type() && !self.type_query;
                    usage.value_as_type = flags.is_value_as_type() || self.type_query;
                    self.facts.occurrences[*index].uses.push(usage);
                }
            }
        }
    }
}

// Oxc does not retain keyword spans. Skip comments rather
// than assuming canonical whitespace or matching a keyword inside a comment.
fn keyword(source: &str, owner: Span, value: &str) -> eyre::Result<Span> {
    let bytes = source.as_bytes();
    let mut at = owner.start as usize;
    let end = owner.end as usize;
    while at < end {
        let character = source[at..end].chars().next().unwrap();
        if character.is_whitespace() || character == '\u{feff}' {
            at += character.len_utf8();
            continue;
        }
        if source[at..end].starts_with("/*") {
            at += source[at + 2..end]
                .find("*/")
                .ok_or_else(|| eyre::eyre!("unterminated parser comment"))?
                + 4;
            continue;
        }
        if source[at..end].starts_with("//") {
            at = source[at..end]
                .find(['\r', '\n', '\u{2028}', '\u{2029}'])
                .map_or(end, |offset| at + offset);
            continue;
        }
        let start = at;
        if character == '*' {
            if value == "*" {
                return Ok(Span::new(at as u32, at as u32 + 1));
            }
            at += 1;
            continue;
        }
        while at < end && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
            at += 1;
        }
        if &source[start..at] == value {
            return Ok(Span::new(start as u32, at as u32));
        }
        ensure!(
            at > start,
            "unexpected token locating TypeScript {value} keyword"
        );
    }
    eyre::bail!("missing parsed TypeScript {value} keyword")
}

/// Companion selection is lazy and exact; this does not select executable loaders.
pub fn companion(path: &Path) -> Option<PathBuf> {
    Some(path.with_extension(match path.extension()?.to_str()? {
        "js" => "d.ts",
        "mjs" => "d.mts",
        "cjs" => "d.cts",
        _ => return None,
    }))
}

pub fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".types.seg.fb");
    name.into()
}

/// Original proof accompanies facts, including negative companion observations.
#[derive(Clone, Debug)]
pub struct Observation {
    input: PathInput,
    record: Option<WireTypeOnlyFacts>,
}

impl Observation {
    pub fn record(&self) -> Option<&WireTypeOnlyFacts> {
        self.record.as_ref()
    }
    pub fn input(&self) -> &PathInput {
        &self.input
    }
    pub fn validate(&self) -> eyre::Result<()> {
        self.input.validate()
    }
}

/// Scoped input boundary for metadata consumers. Implementations must reject
/// unadmitted routes before reading, retain present/missing/link observations,
/// and use a new snapshot at each build attempt. Inventory is the build adapter.
pub trait MetadataInputs {
    fn read_declaration(&self, path: &Path) -> eyre::Result<(PathInput, Option<Arc<[u8]>>)>;
}

impl MetadataInputs for super::inventory::Inventory {
    fn read_declaration(&self, path: &Path) -> eyre::Result<(PathInput, Option<Arc<[u8]>>)> {
        ensure!(
            self.can_admit(path),
            "declaration metadata outside reachable inventory: {}",
            path.display()
        );
        if self.probe_file(path)? {
            self.admit(path)?;
        }
        let (input, bytes) = self
            .snapshot()
            .read(path)
            .wrap_err_with(|| format!("reading declaration metadata {}", path.display()))?;
        self.merge_observations(vec![input.clone()])?;
        Ok((input, bytes))
    }
}

#[derive(Default)]
pub struct TypeOnlyCache {
    records: HashMap<(PathBuf, [u8; 16]), Observation>,
    extractions: usize,
}

impl TypeOnlyCache {
    /// Number of source extractions performed by this shared cache instance.
    pub fn extractions(&self) -> usize {
        self.extractions
    }

    /// Phase-4 entrypoint: use the already parsed original selected-TypeScript
    /// tree on a miss, or retain checked borrowed facts on memory/disk hits.
    /// `input` must come from the caller's scoped inventory/snapshot read.
    pub fn capture_original(
        &mut self,
        input: &PathInput,
        storage: &Path,
        program: &Program<'_>,
        semantic: &Semantic<'_>,
        settings: Settings,
        mode: CacheMode,
    ) -> eyre::Result<(Observation, Option<String>)> {
        check_program(program, settings)?;
        ensure!(
            input.content_seal() == Some(&hash128(program.source_text.as_bytes())),
            "TypeOnlyFacts original source does not match scoped input: {}",
            input.path().display()
        );
        input.validate()?;
        let key = (input.path().to_path_buf(), settings.identity());
        if let Some(old) = self.records.get(&key) {
            if old.input == *input {
                return Ok((old.clone(), None));
            }
        }
        let cache = sidecar(input.path());
        if mode != CacheMode::Disabled {
            if let Ok(file) = fs::File::open(&cache) {
                let bytes = SharedBytes::snapshot_in(storage, file).wrap_err_with(|| {
                    format!("snapshotting original TypeScript facts {}", cache.display())
                })?;
                if let Ok(record) =
                    WireTypeOnlyFacts::from_backing(bytes, program.source_text, settings.identity())
                {
                    let observation = Observation {
                        input: input.clone(),
                        record: Some(record),
                    };
                    observation.validate()?;
                    self.records.insert(key, observation.clone());
                    return Ok((observation, None));
                }
            }
        }
        let record = extract(storage, program, semantic, settings).wrap_err_with(|| {
            format!(
                "extracting original TypeScript facts {}",
                input.path().display()
            )
        })?;
        self.extractions += 1;
        input.validate()?;
        let warning = if mode == CacheMode::ReadWrite {
            publish(&cache, &record, input)
                .err()
                .map(|e| format!("{e:#}"))
        } else {
            None
        };
        let observation = Observation {
            input: input.clone(),
            record: Some(record),
        };
        observation.validate()?;
        self.records.insert(key, observation.clone());
        Ok((observation, warning))
    }

    /// Call on demand after a missing JS name; no companion is scanned eagerly.
    /// Each attempt supplies its current inventory epoch. Returned proofs must also
    /// be checked at final/no-op boundaries, independently of the live snapshot.
    pub fn load_companion(
        &mut self,
        inventory: &impl MetadataInputs,
        storage: &Path,
        javascript: &Path,
        package: ModuleFormat,
        mode: CacheMode,
    ) -> eyre::Result<Option<(Observation, Option<String>)>> {
        let Some(path) = companion(javascript) else {
            return Ok(None);
        };
        let (input, bytes) = inventory.read_declaration(&path)?;
        let Some(bytes) = bytes else {
            return Ok(Some((
                Observation {
                    input,
                    record: None,
                },
                None,
            )));
        };
        let config = ast_segmenter::ParserConfig::for_path(&path, package)?;
        let requested = Settings::from_config(config, package)?;
        // Ambiguous .d.ts follows package/static-ESM policy, just like original TS.
        // Disk records use the requested settings plus effective parser mode.
        let key = (path.clone(), requested.identity_for(enums::Seed::Bounded));
        if let Some(old) = self.records.get(&key) {
            if old.input == input {
                return Ok(Some((old.clone(), None)));
            }
        }
        let source = std::str::from_utf8(&bytes)
            .wrap_err_with(|| format!("decoding declaration metadata {}", path.display()))?;
        let cache = sidecar(&path);
        let mut warning = None;
        if mode != CacheMode::Disabled {
            if let Ok(file) = fs::File::open(&cache) {
                let backing = SharedBytes::snapshot_in(storage, file).wrap_err_with(|| {
                    format!("snapshotting declaration cache {}", cache.display())
                })?;
                for commonjs in [requested.commonjs, !requested.commonjs] {
                    if commonjs != requested.commonjs && !requested.detect_esm {
                        continue;
                    }
                    let effective = Settings {
                        commonjs,
                        ..requested
                    };
                    if let Ok(record) = WireTypeOnlyFacts::from_backing(
                        backing.clone(),
                        source,
                        effective.identity_for(enums::Seed::Bounded),
                    ) {
                        let observation = Observation {
                            input,
                            record: Some(record),
                        };
                        observation.validate()?;
                        self.records.insert(key, observation.clone());
                        return Ok(Some((observation, None)));
                    }
                }
            }
        }
        let allocator = Allocator::default();
        // Metadata parsing is explicit and separate from executable LoaderPolicy.
        let parsed = oxc_utils_parse::parse_bundler_file(&allocator, source, &path, package)?;
        ensure!(
            parsed.program.source_type.is_typescript_definition(),
            "expected declaration metadata syntax"
        );
        let semantic = SemanticBuilder::new()
            .with_check_syntax_error(true)
            // Pinned Oxc expands source-ordered strings without byte limits.
            // Standalone metadata must never enter that evaluator.
            .with_enum_eval(false)
            .build(&parsed.program);
        if let Some(error) = semantic.diagnostics.into_iter().next() {
            return Err(eyre::Report::new(error))
                .wrap_err_with(|| format!("analyzing declaration metadata {}", path.display()));
        }
        let effective =
            Settings::from_config(config.resolved(parsed.program.source_type), package)?;
        let record = extract_inner(
            storage,
            &parsed.program,
            &semantic.semantic,
            effective,
            enums::Seed::Bounded,
        )
        .wrap_err_with(|| format!("extracting declaration metadata {}", path.display()))?;
        self.extractions += 1;
        input.validate()?;
        if mode == CacheMode::ReadWrite {
            warning = publish(&cache, &record, &input)
                .err()
                .map(|e| format!("{e:#}"));
        }
        let observation = Observation {
            input,
            record: Some(record),
        };
        observation.validate()?;
        self.records.insert(key, observation.clone());
        Ok(Some((observation, warning)))
    }
}

fn publish(path: &Path, record: &WireTypeOnlyFacts, input: &PathInput) -> eyre::Result<()> {
    let write = || -> eyre::Result<()> {
        let mut staging = tempfile::Builder::new()
            .prefix(&format!(
                "{}.stage-",
                path.file_name().unwrap().to_string_lossy()
            ))
            .tempfile_in(path.parent().unwrap())?;
        staging.write_all(record.backing())?;
        staging.flush()?;
        staging.as_file().sync_all()?;
        let staged = staging.into_temp_path();
        input.validate()?;
        fs::rename(&staged, path)?;
        Ok(())
    };
    write().wrap_err_with(|| format!("publishing declaration facts {}", path.display()))
}
