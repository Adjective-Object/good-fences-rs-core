use crate::{
    raw_module_deps::{ExportBinding, RawModuleDeps, Symbol, SymbolTags, TaggedSymbol},
    ExportedSymbol, ReExportedSymbol,
};

use logger_srcfile::SrcFileLogger;
use std::{collections::HashSet, iter::FromIterator};
use swc_common::{
    comments::{CommentKind, Comments, SingleThreadedComments},
    BytePos, Spanned,
};
use swc_ecma_ast::{
    BindingIdent, CallExpr, Callee, Decl, ExportAll, ExportDecl, ExportDefaultDecl,
    ExportDefaultExpr, ExportSpecifier, ImportDecl, ImportSpecifier, Lit, ModuleExportName,
    NamedExport, Str, TsImportEqualsDecl, TsModuleName,
};
use swc_ecma_visit::{Visit, VisitWith};

// AST visitor that gathers information on file imports and exports from an SWC source tree.
#[derive(Debug)]
pub struct ExportsVisitor<TLogger: SrcFileLogger> {
    pub logger: TLogger,
    pub comments: SingleThreadedComments,
    pub module_deps: RawModuleDeps,
}

/**
 * Extracts information from each specifier imported in source to treat it as an string
 * Supported sytax list:
 * - `export { foo as bar } from 'foo'`
 * - `export { default as foo } from 'foo'`
 * - `export { foo } from 'foo'`
 */
fn get_export_bindings(
    comments: impl Comments,
    parent_tags: SymbolTags,
    export: &NamedExport,
    source: &Str,
) -> impl Iterator<Item = ExportBinding> {
    // local copy of the parent tags that considers the 'type' field in `export type`
    let mut parent_tags = parent_tags;
    parent_tags.is_type_only |= export.type_only;

    let borrowed_comments = &comments;
    let specifiers = export.specifiers.iter().map(|spec| -> ExportBinding {
        // allow overriding tags on a per-specifier basis
        let mut specifier_tags =
            SymbolTags::from_comments_parent(parent_tags, borrowed_comments, spec.span().lo());
        match spec {
            // export * as v from 'mod';
            ExportSpecifier::Namespace(spec) => ExportBinding {
                original: TaggedSymbol::new(Symbol::Namespace, specifier_tags),
                exported_as: Some(ExportedSymbol::from_module_export_name(&spec.name)),
            },
            // export v from 'mod';
            ExportSpecifier::Default(spec) => ExportBinding {
                original: TaggedSymbol::new(Symbol::Default, specifier_tags),
                exported_as: Some(ExportedSymbol::from(spec.exported)),
            },
            // export { v as w } from 'mod';
            ExportSpecifier::Named(spec) => {
                let imported_name = spec.orig.atom().to_string();
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
    });
    specifiers
}

impl<TLogger: SrcFileLogger> ExportsVisitor<TLogger> {
    pub fn new(logger: TLogger, comments: SingleThreadedComments) -> Self {
        Self {
            logger,
            comments,
            module_deps: Default::default(),
        }
    }

    pub fn has_disable_export_comment(&self, lo: BytePos) -> bool {
        has_disable_export_comment(&self.comments, lo)
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

impl<T: SrcFileLogger> From<ExportsVisitor<T>> for RawImportExportInfo {
    fn from(x: ExportsVisitor<T>) -> Self {
        Self {
            imported_path_ids: x.imported_ids_path_name,
            require_paths: x.require_paths,
            imported_paths: x.imported_paths,
            export_from_ids: x.export_from_ids, // TODO replace with Exportx maps
            exported_ids: x.exported_ids,
            executed_paths: x.executed_paths,
        }
    }
}

impl<T: SrcFileLogger> Visit for ExportsVisitor<T> {
    // Handles `export default foo`
    fn visit_export_default_expr(&mut self, expr: &ExportDefaultExpr) {
        expr.visit_children_with(self);
        self.exported_ids.insert(
            ExportedSymbol::Default,
            TaggedSymbol {
                span: expr.span(),
                allow_unused: self.has_disable_export_comment(expr.span_lo()),
                is_type_only: false,
            },
        );
    }

    /**
     * Handles scenarios where `export default` has an inline declaration, e.g. `export default class Foo {}` or `export default function foo() {}`
     */
    fn visit_export_default_decl(&mut self, decl: &ExportDefaultDecl) {
        decl.visit_children_with(self);
        let is_type_only = decl.decl.is_ts_interface_decl();
        self.exported_ids.insert(
            ExportedSymbol::Default,
            TaggedSymbol {
                span: decl.span(),
                allow_unused: self.has_disable_export_comment(decl.span_lo()),
                is_type_only,
            },
        );
    }

    // Handles scenarios `export` has an inline declaration, e.g. `export const foo = 1` or `export class Foo {}`
    fn visit_export_decl(&mut self, export: &ExportDecl) {
        export.visit_children_with(self);
        let allow_unused = self.has_disable_export_comment(export.span_lo());
        let is_type_only = export.decl.is_ts_interface() || export.decl.is_ts_type_alias();
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
                    let child_scope = ast_name_tracker::find_names(&self.logger, d);
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
            self.exported_ids.insert(
                ExportedSymbol::Named(ident),
                TaggedSymbol {
                    span: export.span(),
                    allow_unused,
                    is_type_only,
                },
            );
        }
    }

    // `export * from './foo'`; // TODO allow recursive import resolution
    fn visit_export_all(&mut self, export: &ExportAll) {
        export.visit_children_with(self);
        let source = export.src.value.to_string();
        let allow_unused = self.has_disable_export_comment(export.span_lo());
        self.export_from_ids.entry(source).or_default().insert(
            ExportBinding {
                imported: ExportedSymbol::Namespace,
                renamed_to: None,
            },
            TaggedSymbol {
                span: export.span(),
                allow_unused,
                is_type_only: export.type_only,
            },
        );
    }

    // export {foo} from './foo';
    fn visit_named_export(&mut self, export: &NamedExport) {
        export.visit_children_with(self);
        if let Some(source) = &export.src {
            // TODO: track tags
            let tags = SymbolTags::from_comments(self.comments, export.span_lo());
            // In case we find `'./foo'` in `export { foo } from './foo'`
            get_export_bindings(self.comments, tags, export, source).for_each(|binding| {
                let as_export_binding: ReExportedSymbol = binding.try_as_re_export();
            });
        } else {
            self.handle_export_named_specifiers(
                &export.specifiers,
                self.has_disable_export_comment(export.span_lo()),
                export.type_only,
                export.span(),
            );
        }
    }

    // const foo = require; // <- Binding
    // const p = foo('./path')
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
            self.imported_paths
                .insert(module_ref.expr.value.to_string());
        }
    }

    // import('foo')
    // or
    // require('foo')
    fn visit_call_expr(&mut self, expr: &CallExpr) {
        expr.visit_children_with(self);
        if let Callee::Import(_) = &expr.callee {
            match extract_argument_value(expr) {
                Some(import_path) => {
                    self.imported_paths.insert(import_path);
                }
                None => return,
            }
        }
        if let Callee::Expr(callee) = &expr.callee {
            if let Some(ident) = callee.as_ident() {
                if ident.sym == "require" && !self.require_identifiers.contains(&ident.to_id()) {
                    if let Some(import_path) = extract_argument_value(expr) {
                        self.require_paths.insert(import_path);
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
            self.executed_paths.insert(src);
            return;
        }
        // import .. from ..
        let mut specifiers: Vec<ExportedSymbol> = import
            .specifiers
            .iter()
            .map(|spec| -> ExportBinding {
                match spec {
                    ImportSpecifier::Named(named) => {
                        match &named.imported {
                            Some(module_name) => {
                                // import { foo as bar } from './foo'
                                match module_name {
                                    ModuleExportName::Ident(ident) => {
                                        // sym_str = foo in `import { foo as bar } from './foo'`
                                        let sym_str = ident.sym.to_string();
                                        if sym_str == "default" {
                                            // import { default as foo } from 'foo'
                                            return ExportedSymbol::Default;
                                        }
                                        ExportedSymbol::Named(sym_str)
                                    }
                                    ModuleExportName::Str(s) => {
                                        ExportedSymbol::Named(s.value.to_string())
                                    }
                                }
                            }
                            None => {
                                // import { foo } from './foo'
                                ExportedSymbol::Named(named.local.sym.to_string())
                            }
                        }
                    }
                    ImportSpecifier::Default(_) => {
                        // import foo from 'foo'
                        ExportedSymbol::Default
                    }
                    ImportSpecifier::Namespace(_) => {
                        // import * as foo from 'foo'
                        ExportedSymbol::Namespace
                    }
                }
            })
            .collect();

        if let Some(entry) = self.imported_ids_path_name.get_mut(&src) {
            specifiers.drain(0..).for_each(|s| {
                entry.insert(s);
            });
        } else {
            self.imported_ids_path_name
                .insert(src, HashSet::from_iter(specifiers));
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
