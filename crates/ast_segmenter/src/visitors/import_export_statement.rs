use crate::{
    raw_module_deps::{ExportBinding, RawModuleDeps, Symbol, SymbolTags, TaggedSymbol},
    ExportedSymbol, ReExportedSymbol,
};

use logger_srcfile::SrcFileLogger;
use std::convert::TryInto;
use swc_common::{
    comments::{CommentKind, Comments, SingleThreadedComments},
    BytePos, Spanned,
};
use swc_ecma_ast::{
    BindingIdent, CallExpr, Callee, Decl, ExportAll, ExportDecl, ExportDefaultDecl,
    ExportDefaultExpr, ExportSpecifier, Id, ImportDecl, ImportSpecifier, Lit,
    NamedExport, Str, TsImportEqualsDecl, TsModuleName,
};
use swc_ecma_visit::{Visit, VisitWith};

// AST visitor that gathers information on file imports and exports from an SWC source tree.
#[derive(Debug)]
pub struct ExportsVisitor<'a, TLogger: SrcFileLogger> {
    pub logger: &'a TLogger,
    pub comments: &'a SingleThreadedComments,
    pub module_deps: RawModuleDeps,
    require_identifiers: ahashmap::AHashSet<Id>,
}

/**
 * Extracts information from each specifier imported in source to treat it as an string
 * Supported sytax list:
 * - `export { foo as bar } from 'foo'`
 * - `export { default as foo } from 'foo'`
 * - `export { foo } from 'foo'`
 */
fn get_export_bindings(
    comments: &SingleThreadedComments,
    parent_tags: SymbolTags,
    export: &NamedExport,
    _source: &Str,
) -> Vec<ExportBinding> {
    // local copy of the parent tags that considers the 'type' field in `export type`
    let mut parent_tags = parent_tags;
    parent_tags.is_type_only |= export.type_only;

    export.specifiers.iter().map(|spec| -> ExportBinding {
        // allow overriding tags on a per-specifier basis
        let specifier_tags =
            SymbolTags::from_comments_parent(parent_tags.clone(), comments, spec.span().lo());
        match spec {
            // export * as v from 'mod';
            ExportSpecifier::Namespace(spec) => ExportBinding {
                original: TaggedSymbol::new(Symbol::Namespace, specifier_tags),
                exported_as: Some(ExportedSymbol::from_module_export_name(&spec.name)),
            },
            // export v from 'mod';
            ExportSpecifier::Default(spec) => ExportBinding {
                original: TaggedSymbol::new(Symbol::Default, specifier_tags),
                exported_as: Some(ExportedSymbol::from(spec.exported.sym.as_ref())),
            },
            // export { v as w } from 'mod';
            ExportSpecifier::Named(spec) => {
                let imported_name = spec.orig.atom().to_string();
                let mut specifier_tags = specifier_tags;
                // export { type v as w } from 'mod';
                specifier_tags.is_type_only |= spec.is_type_only;
                ExportBinding {
                    original: TaggedSymbol::new(Symbol::from(imported_name), specifier_tags),
                    exported_as: spec
                        .exported
                        .as_ref()
                        .map(ExportedSymbol::from_module_export_name),
                }
            }
        }
    }).collect()
}

impl<'a, TLogger: SrcFileLogger> ExportsVisitor<'a, TLogger> {
    pub fn new(logger: &'a TLogger, comments: &'a SingleThreadedComments) -> Self {
        Self {
            logger,
            comments,
            module_deps: Default::default(),
            require_identifiers: Default::default(),
        }
    }

    pub fn has_disable_export_comment(&self, lo: BytePos) -> bool {
        has_disable_export_comment(self.comments, lo)
    }
}

pub fn has_disable_export_comment(comments: &SingleThreadedComments, lo: BytePos) -> bool {
    if let Some(comments) = comments.get_leading(lo) {
        return comments.iter().any(|c| {
            c.kind == CommentKind::Line && c.text.trim().starts_with("@ALLOW-UNUSED-EXPORT")
        });
    }
    false
}

impl<'a, T: SrcFileLogger> From<ExportsVisitor<'a, T>> for RawModuleDeps {
    fn from(x: ExportsVisitor<'a, T>) -> Self {
        x.module_deps
    }
}

impl<'a, T: SrcFileLogger> Visit for ExportsVisitor<'a, T> {
    // Handles `export default foo`
    fn visit_export_default_expr(&mut self, expr: &ExportDefaultExpr) {
        expr.visit_children_with(self);
        let tags = SymbolTags::from_comments(self.comments, expr.span_lo());
        self.module_deps.exports_locals.insert(
            ExportedSymbol::Default,
            TaggedSymbol::new(Symbol::Default, tags),
        );
    }

    /// Handles `export default class Foo {}` or `export default function foo() {}`
    fn visit_export_default_decl(&mut self, decl: &ExportDefaultDecl) {
        decl.visit_children_with(self);
        let mut tags = SymbolTags::from_comments(self.comments, decl.span_lo());
        tags.is_type_only = decl.decl.is_ts_interface_decl();
        self.module_deps.exports_locals.insert(
            ExportedSymbol::Default,
            TaggedSymbol::new(Symbol::Default, tags),
        );
    }

    // Handles `export const foo = 1` or `export class Foo {}`
    fn visit_export_decl(&mut self, export: &ExportDecl) {
        export.visit_children_with(self);
        let mut tags = SymbolTags::from_comments(self.comments, export.span_lo());
        tags.is_type_only = export.decl.is_ts_interface() || export.decl.is_ts_type_alias();
        let idents = match &export.decl {
            Decl::Class(decl) => {
                vec![decl.ident.sym.to_string()]
            }
            Decl::Fn(decl) => {
                vec![decl.ident.sym.to_string()]
            }
            Decl::Var(decl) => decl
                .decls
                .iter()
                .flat_map(|d: &swc_ecma_ast::VarDeclarator| -> Vec<String> {
                    let child_scope = ast_name_tracker::find_names(self.logger, d);
                    child_scope
                        .get_locals()
                        .map(|k| k.to_string())
                        .collect::<Vec<_>>()
                })
                .collect(),
            Decl::TsInterface(decl) => {
                vec![decl.id.sym.to_string()]
            }
            Decl::TsTypeAlias(decl) => {
                vec![decl.id.sym.to_string()]
            }
            Decl::TsEnum(decl) => {
                vec![decl.id.sym.to_string()]
            }
            Decl::TsModule(decl) => match &decl.id {
                TsModuleName::Ident(ident) => vec![ident.sym.to_string()],
                TsModuleName::Str(str) => vec![str.value.to_string()],
            },
            Decl::Using(_) => {
                vec![]
            }
        };

        for ident in idents {
            self.module_deps.exports_locals.insert(
                ExportedSymbol::from(ident.as_str()),
                TaggedSymbol::new(Symbol::from(ident.as_str()), tags.clone()),
            );
        }
    }

    // `export * from './foo'`
    fn visit_export_all(&mut self, export: &ExportAll) {
        export.visit_children_with(self);
        let source = export.src.value.to_string();
        let binding: ReExportedSymbol = ReExportedSymbol {
            imported_as: crate::ImportTarget::Namespace,
            exported_as: None,
        };
        self.module_deps
            .exports_from
            .entry(source)
            .or_default()
            .insert(binding);
    }

    // export {foo} from './foo';
    fn visit_named_export(&mut self, export: &NamedExport) {
        export.visit_children_with(self);
        if let Some(source) = &export.src {
            let tags = SymbolTags::from_comments(self.comments, export.span_lo());
            let source_str = source.value.to_string();
            get_export_bindings(self.comments, tags, export, source).into_iter().for_each(|binding| {
                if let Ok(re_export) = TryInto::<ReExportedSymbol>::try_into(binding) {
                    self.module_deps
                        .exports_from
                        .entry(source_str.clone())
                        .or_default()
                        .insert(re_export);
                }
            });
        } else {
            let allow_unused = self.has_disable_export_comment(export.span_lo());
            let is_type_only = export.type_only;
            for spec in &export.specifiers {
                match spec {
                    ExportSpecifier::Named(named) => {
                        let local_name = named.orig.atom().to_string();
                        let exported_as = named
                            .exported
                            .as_ref()
                            .map(ExportedSymbol::from_module_export_name);
                        let exported_key = exported_as
                            .clone()
                            .unwrap_or_else(|| ExportedSymbol::from(local_name.as_str()));
                        let mut tags = SymbolTags::from_comments(self.comments, named.span().lo());
                        tags.allow_unused_comment |= allow_unused;
                        tags.is_type_only |= is_type_only || named.is_type_only;
                        self.module_deps.exports_locals.insert(
                            exported_key,
                            TaggedSymbol::new(Symbol::from(local_name.as_str()), tags),
                        );
                    }
                    ExportSpecifier::Default(default_spec) => {
                        let mut tags = SymbolTags::default();
                        tags.allow_unused_comment = allow_unused;
                        tags.is_type_only = is_type_only;
                        self.module_deps.exports_locals.insert(
                            ExportedSymbol::Default,
                            TaggedSymbol::new(Symbol::Default, tags),
                        );
                    }
                    ExportSpecifier::Namespace(ns) => {
                        let mut tags = SymbolTags::default();
                        tags.allow_unused_comment = allow_unused;
                        tags.is_type_only = is_type_only;
                        self.module_deps.exports_locals.insert(
                            ExportedSymbol::from_module_export_name(&ns.name),
                            TaggedSymbol::new(Symbol::Namespace, tags),
                        );
                    }
                }
            }
        }
    }

    // const foo = require; // <- Binding
    fn visit_binding_ident(&mut self, binding: &BindingIdent) {
        binding.visit_children_with(self);
        if binding.sym == *"require" {
            self.require_identifiers.insert(binding.id.to_id());
        }
    }

    // import foo = require('./foo')
    fn visit_ts_import_equals_decl(&mut self, decl: &TsImportEqualsDecl) {
        decl.visit_children_with(self);
        if let Some(module_ref) = decl.module_ref.as_ts_external_module_ref() {
            self.module_deps
                .dynamic_imports
                .entry(module_ref.expr.value.to_string())
                .or_default()
                .insert(Symbol::Namespace);
        }
    }

    // import('foo') or require('foo')
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        if let Callee::Import(_) = &expr.callee {
            if let Some(import_path) = extract_argument_value(expr) {
                self.module_deps
                    .dynamic_imports
                    .entry(import_path)
                    .or_default()
                    .insert(Symbol::Namespace);
            }
        }
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                if ident.sym == "require" && !self.require_identifiers.contains(&ident.to_id()) {
                    if let Some(import_path) = extract_argument_value(expr) {
                        self.module_deps.requires.insert(import_path);
                    }
                }
            }
        }
    }

    // import foo from './foo';
    fn visit_import_decl(&mut self, import: &ImportDecl) {
        import.visit_children_with(self);

        let src = import.src.value.to_string();
        // import './foo';
        if import.specifiers.is_empty() {
            self.module_deps.executed_paths.insert(src);
            return;
        }
        // import .. from ..
        let mut parent_tags = SymbolTags::from_comments(self.comments, import.span_lo());
        parent_tags.is_type_only |= import.type_only;

        let tagged_symbols: Vec<TaggedSymbol> = import
            .specifiers
            .iter()
            .map(|spec| -> TaggedSymbol {
                let mut tags = parent_tags.clone();
                match spec {
                    ImportSpecifier::Named(named) => {
                        tags.is_type_only |= named.is_type_only;
                        match &named.imported {
                            Some(module_name) => {
                                TaggedSymbol::new(Symbol::from_module_export_name(module_name), tags)
                            }
                            None => {
                                TaggedSymbol::new(Symbol::from(named.local.sym.as_ref()), tags)
                            }
                        }
                    }
                    ImportSpecifier::Default(_) => {
                        TaggedSymbol::new(Symbol::Default, tags)
                    }
                    ImportSpecifier::Namespace(_) => {
                        TaggedSymbol::new(Symbol::Namespace, tags)
                    }
                }
            })
            .collect();

        let entry = self.module_deps.imports.entry(src).or_default();
        for sym in tagged_symbols {
            entry.insert(sym);
        }
    }
}

fn extract_argument_value(expr: &CallExpr) -> Option<String> {
    let import_path = match expr.args.is_empty() {
        true => return None,
        false => expr.args.first(),
    };
    if let Some(path) = import_path {
        if let Some(path_lit) = path.expr.as_lit() {
            match path_lit {
                Lit::Str(value) => {
                    return Some(value.value.to_string());
                }
                _ => return None,
            }
        }
    }
    None
}
