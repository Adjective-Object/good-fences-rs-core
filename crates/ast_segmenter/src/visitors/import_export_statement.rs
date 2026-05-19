use std::convert::TryInto;

use crate::{
    raw_module_deps::{ExportBinding, RawModuleDeps, Symbol, SymbolTags, TaggedSymbol},
    ExportedSymbol, ReExportedSymbol,
};

use logger_srcfile::SrcFileLogger;
use oxc_ast::ast::{
    BindingPattern, Comment, CommentKind, Declaration, ExportAllDeclaration,
    ExportDefaultDeclaration, ExportDefaultDeclarationKind, ExportNamedDeclaration,
    ImportDeclaration, ImportDeclarationSpecifier, ImportOrExportKind,
    TSImportEqualsDeclaration, TSModuleDeclarationName, TSModuleReference,
};
use oxc_semantic::Semantic;
use oxc_utils_parse::leading_comments_at;

// AST visitor that gathers information on file imports and exports from an OXC source tree.
pub struct ExportsVisitor<'a, TLogger: SrcFileLogger> {
    pub logger: &'a TLogger,
    comments: &'a [Comment],
    source: &'a str,
    #[allow(dead_code)]
    semantic: &'a Semantic<'a>,
    pub module_deps: RawModuleDeps,
}

impl<'a, TLogger: SrcFileLogger> ExportsVisitor<'a, TLogger> {
    pub fn new(logger: &'a TLogger, semantic: &'a Semantic<'a>) -> Self {
        let source = semantic.source_text();
        let comments = semantic.comments();
        Self {
            logger,
            comments,
            source,
            semantic,
            module_deps: Default::default(),
        }
    }
}

impl<'a, T: SrcFileLogger> From<ExportsVisitor<'a, T>> for RawModuleDeps {
    fn from(x: ExportsVisitor<'a, T>) -> Self {
        x.module_deps
    }
}

/// Returns `true` if the leading comments before `lo` contain `@ALLOW-UNUSED-EXPORT`.
pub fn has_disable_export_comment(comments: &[Comment], source: &str, lo: u32) -> bool {
    leading_comments_at(comments, source, lo).iter().any(|c| {
        if c.kind != CommentKind::Line {
            return false;
        }
        let cs = c.content_span();
        let text = &source[cs.start as usize..cs.end as usize];
        text.trim().starts_with("@ALLOW-UNUSED-EXPORT")
    })
}

/// Recursively collect all binding identifiers from a pattern (for `export const { a, b } = ...`).
fn binding_pattern_names(pat: &BindingPattern) -> Vec<String> {
    match pat {
        BindingPattern::BindingIdentifier(id) => vec![id.name.to_string()],
        BindingPattern::ObjectPattern(obj) => {
            let mut names: Vec<String> = obj
                .properties
                .iter()
                .flat_map(|prop| binding_pattern_names(&prop.value))
                .collect();
            if let Some(rest) = &obj.rest {
                names.extend(binding_pattern_names(&rest.argument));
            }
            names
        }
        BindingPattern::ArrayPattern(arr) => {
            let mut names: Vec<String> = arr
                .elements
                .iter()
                .flatten()
                .flat_map(binding_pattern_names)
                .collect();
            if let Some(rest) = &arr.rest {
                names.extend(binding_pattern_names(&rest.argument));
            }
            names
        }
        BindingPattern::AssignmentPattern(assign) => {
            binding_pattern_names(&assign.left)
        }
    }
}

impl<'a, TLogger: SrcFileLogger> ExportsVisitor<'a, TLogger> {
    /// Handle `import foo from './foo'` and `import { foo } from './foo'` and `import './foo'`.
    pub fn process_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        let src = import.source.value.to_string();

        // `import './foo'` — side-effect only, no specifiers
        if import.specifiers.is_none() {
            self.module_deps.executed_paths.insert(src);
            return;
        }

        let specifiers = import.specifiers.as_ref().unwrap();

        // `import {} from './foo'` — also side-effect (no names imported)
        if specifiers.is_empty() {
            self.module_deps.executed_paths.insert(src);
            return;
        }

        let mut parent_tags = SymbolTags::from_comments(self.comments, self.source, import.span.start);
        parent_tags.is_type_only |= import.import_kind == ImportOrExportKind::Type;

        let entry = self.module_deps.imports.entry(src).or_default();
        for spec in specifiers {
            let mut tags = parent_tags.clone();
            let sym = match spec {
                ImportDeclarationSpecifier::ImportSpecifier(named) => {
                    tags.is_type_only |= named.import_kind == ImportOrExportKind::Type;
                    Symbol::from_module_export_name(&named.imported)
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => Symbol::Default,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => Symbol::Namespace,
            };
            entry.insert(TaggedSymbol::new(sym, tags));
        }
    }

    /// Handle `export { foo } from './source'` re-exports.
    fn process_export_named_reexports(
        &mut self,
        export: &ExportNamedDeclaration<'a>,
        source_str: String,
    ) {
        let mut parent_tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
        parent_tags.is_type_only |= export.export_kind == ImportOrExportKind::Type;

        for spec in &export.specifiers {
            let mut spec_tags = SymbolTags::from_comments_parent(
                parent_tags.clone(),
                self.comments,
                self.source,
                spec.span.start,
            );
            spec_tags.is_type_only |= spec.export_kind == ImportOrExportKind::Type;

            let original_sym = Symbol::from_module_export_name(&spec.local);
            let exported_sym = ExportedSymbol::from_module_export_name(&spec.exported);

            // Normalize: if local == exported (no rename), set exported_as to None
            let exported_as = if spec.local.name() == spec.exported.name() {
                None
            } else {
                Some(exported_sym)
            };

            let binding = ExportBinding {
                original: TaggedSymbol::with_span(original_sym, spec_tags, export.span),
                exported_as,
            };
            if let Ok(re_export) = TryInto::<ReExportedSymbol>::try_into(binding) {
                self.module_deps
                    .exports_from
                    .entry(source_str.clone())
                    .or_default()
                    .insert(re_export);
            }
        }
    }

    /// Handle `export const foo = 1`, `export function bar() {}`, etc.
    fn process_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>, decl: &Declaration<'a>) {
        let mut tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
        tags.is_type_only |= matches!(
            decl,
            Declaration::TSTypeAliasDeclaration(_) | Declaration::TSInterfaceDeclaration(_)
        );

        let idents: Vec<String> = match decl {
            Declaration::ClassDeclaration(c) => {
                c.id.as_ref().map(|id| id.name.to_string()).into_iter().collect()
            }
            Declaration::FunctionDeclaration(f) => {
                f.id.as_ref().map(|id| id.name.to_string()).into_iter().collect()
            }
            Declaration::VariableDeclaration(vd) => vd
                .declarations
                .iter()
                .flat_map(|d| binding_pattern_names(&d.id))
                .collect(),
            Declaration::TSInterfaceDeclaration(t) => vec![t.id.name.to_string()],
            Declaration::TSTypeAliasDeclaration(t) => vec![t.id.name.to_string()],
            Declaration::TSEnumDeclaration(t) => vec![t.id.name.to_string()],
            Declaration::TSModuleDeclaration(m) => match &m.id {
                TSModuleDeclarationName::Identifier(id) => vec![id.name.to_string()],
                TSModuleDeclarationName::StringLiteral(s) => vec![s.value.to_string()],
            },
            Declaration::TSImportEqualsDeclaration(t) => {
                // `export import Foo = require('./foo')` — add to dynamic_imports too
                self.process_ts_import_equals(t);
                vec![t.id.name.to_string()]
            }
            Declaration::TSGlobalDeclaration(_) => vec![],
        };

        for ident in idents {
            self.module_deps.exports_locals.insert(
                ExportedSymbol::from(ident.as_str()),
                TaggedSymbol::with_span(Symbol::from(ident.as_str()), tags.clone(), export.span),
            );
        }
    }

    /// Handle `export { val }` (local specifiers, no source).
    fn process_export_named_locals(&mut self, export: &ExportNamedDeclaration<'a>) {
        let lo = export.span.start;
        let allow_unused = has_disable_export_comment(self.comments, self.source, lo);
        let is_type_only = export.export_kind == ImportOrExportKind::Type;

        for spec in &export.specifiers {
            let local_sym = Symbol::from_module_export_name(&spec.local);
            let exported_key = ExportedSymbol::from_module_export_name(&spec.exported);
            let mut tags = SymbolTags::from_comments(self.comments, self.source, spec.span.start);
            tags.allow_unused_comment |= allow_unused;
            tags.is_type_only |= is_type_only || spec.export_kind == ImportOrExportKind::Type;
            self.module_deps.exports_locals.insert(
                exported_key,
                TaggedSymbol::with_span(local_sym, tags, export.span),
            );
        }
    }

    /// Handle `import foo = require('./foo')` (adds to dynamic_imports as Namespace).
    pub fn process_ts_import_equals(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(emr) = &decl.module_reference {
            self.module_deps
                .dynamic_imports
                .entry(emr.expression.value.to_string())
                .or_default()
                .insert(Symbol::Namespace);
        }
    }

    /// Visit an `ExportNamedDeclaration` — dispatches to the 3 sub-cases.
    pub fn process_export_named(&mut self, export: &ExportNamedDeclaration<'a>) {
        if let Some(source) = &export.source {
            self.process_export_named_reexports(export, source.value.to_string());
        } else if let Some(decl) = &export.declaration {
            self.process_export_named_declaration(export, decl);
        } else {
            self.process_export_named_locals(export);
        }
    }

    /// Visit `export * from './foo'` and `export * as Foo from './foo'`.
    pub fn process_export_all(&mut self, export: &ExportAllDeclaration<'a>) {
        let source = export.source.value.to_string();
        let mut tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
        tags.is_type_only |= export.export_kind == ImportOrExportKind::Type;

        let binding: ReExportedSymbol = ReExportedSymbol {
            imported_as: crate::ImportTarget::Namespace,
            exported_as: export.exported.as_ref().map(ExportedSymbol::from_module_export_name),
            tags,
            span: export.span,
        };
        self.module_deps
            .exports_from
            .entry(source)
            .or_default()
            .insert(binding);
    }

    /// Visit `export default foo`, `export default function() {}`, etc.
    pub fn process_export_default(&mut self, decl: &ExportDefaultDeclaration<'a>) {
        let mut tags = SymbolTags::from_comments(self.comments, self.source, decl.span.start);
        tags.is_type_only = matches!(
            &decl.declaration,
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
        );
        self.module_deps.exports_locals.insert(
            ExportedSymbol::Default,
            TaggedSymbol::with_span(Symbol::Default, tags, decl.span),
        );
    }
}

impl<'a, TLogger: SrcFileLogger> oxc_ast_visit::Visit<'a> for ExportsVisitor<'a, TLogger> {
    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        self.process_import_declaration(import);
    }

    fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
        self.process_export_named(export);
    }

    fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
        self.process_export_all(export);
    }

    fn visit_export_default_declaration(&mut self, decl: &ExportDefaultDeclaration<'a>) {
        self.process_export_default(decl);
    }

    fn visit_ts_import_equals_declaration(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        self.process_ts_import_equals(decl);
    }
}
