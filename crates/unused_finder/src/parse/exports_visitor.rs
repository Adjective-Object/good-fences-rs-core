use super::{ExportedSymbol, ExportedSymbolMetadata, RawImportExportInfo, ReExportedSymbol};
use ahashmap::{AHashMap, AHashSet};
use std::{collections::HashSet, iter::FromIterator};
use swc_common::{
    comments::{CommentKind, Comments, SingleThreadedComments},
    BytePos, Span, Spanned,
};
use swc_ecma_ast::{
    BindingIdent, CallExpr, Callee, Decl, ExportAll, ExportDecl, ExportDefaultDecl,
    ExportDefaultExpr, ExportSpecifier, Id, ImportDecl, ImportSpecifier, Lit, ModuleExportName,
    NamedExport, Pat, Str, TsImportEqualsDecl, TsModuleName,
};
use swc_ecma_visit::{Visit, VisitWith};

// AST visitor that gathers information on file imports and exports from an SWC source tree.
#[derive(Debug)]
pub struct ExportsVisitor {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_ids_path_name: AHashMap<String, AHashSet<ExportedSymbol>>,
    // require('foo') generates ['foo']
    pub require_paths: AHashSet<String>,
    // import('./foo') and import './foo' generates ["./foo"]
    pub imported_paths: AHashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: AHashMap<String, AHashMap<ReExportedSymbol, ExportedSymbolMetadata>>,
    // IDs exported from this file, that were locally declared
    pub exported_ids: AHashMap<ExportedSymbol, ExportedSymbolMetadata>,
    // Side-effect-only imports.
    // import './foo';
    pub executed_paths: AHashSet<String>,
    // exported from this file
    // const foo = require('foo') generates ["foo"]
    require_identifiers: AHashSet<Id>,
    pub comments: SingleThreadedComments,
}

impl ExportsVisitor {
    pub fn new(comments: SingleThreadedComments) -> Self {
        Self {
            imported_ids_path_name: AHashMap::default(),
            require_paths: AHashSet::default(),
            imported_paths: AHashSet::default(),
            export_from_ids: AHashMap::default(),
            executed_paths: AHashSet::default(),
            require_identifiers: AHashSet::default(),
            exported_ids: AHashMap::default(),
            comments,
        }
    }

    /**
     * Extracts information from each specifier imported in source to treat it as an string
     * Supported sytax list:
     * - `export { foo as bar } from 'foo'`
     * - `export { default as foo } from 'foo'`
     * - `export { foo } from 'foo'`
     */
    fn handle_export_from_specifiers(
        &mut self,
        parent_allow_unused: bool,
        parent_is_type_only: bool,
        export: &NamedExport,
        source: &Str,
    ) {
        let borrowed_comments = &self.comments;
        let specifiers =
            export
                .specifiers
                .iter()
                .map(|spec| -> (ReExportedSymbol, ExportedSymbolMetadata) {
                    let allow_unused = parent_allow_unused
                        || has_disable_export_comment(borrowed_comments, spec.span_lo());
                    match spec {
                        ExportSpecifier::Namespace(spec) => (
                            ReExportedSymbol {
                                imported: ExportedSymbol::Namespace,
                                renamed_to: Some(ExportedSymbol::from(&spec.name)),
                            },
                            ExportedSymbolMetadata {
                                span: spec.span(),
                                allow_unused,
                                is_type_only: parent_is_type_only || export.type_only,
                            },
                        ),
                        ExportSpecifier::Default(spec) => (
                            ReExportedSymbol {
                                imported: ExportedSymbol::Default,
                                renamed_to: Some(ExportedSymbol::Named(spec.exported.to_string())),
                            },
                            ExportedSymbolMetadata {
                                span: spec.span(),
                                allow_unused,
                                is_type_only: parent_is_type_only || export.type_only,
                            },
                        ),
                        ExportSpecifier::Named(spec) => {
                            let imported_name = spec.orig.atom().to_string();
                            let imported = if imported_name == "default" {
                                ExportedSymbol::Default
                            } else {
                                ExportedSymbol::Named(imported_name)
                            };
                            (
                                ReExportedSymbol {
                                    imported,
                                    renamed_to: spec.exported.as_ref().map(ExportedSymbol::from),
                                },
                                ExportedSymbolMetadata {
                                    span: spec.span(),
                                    allow_unused,
                                    is_type_only: parent_is_type_only || export.type_only,
                                },
                            )
                        }
                    }
                });

        let entry = self
            .export_from_ids
            .entry(source.value.to_string())
            .or_default();
        for (re_exported_symbol, metadata) in specifiers {
            entry.insert(re_exported_symbol, metadata);
        }
    }

    /**
     * Extracts information from the ExportSpecifier to construct the map of exported items with its values.
     * supports `export { foo }` and aliased `export { foo as bar }`
     */
    fn handle_export_named_specifiers(
        &mut self,
        specs: &[ExportSpecifier],
        allow_unused: bool,
        parent_is_type_only: bool,
        span: Span,
    ) {
        specs.iter().for_each(|specifier| {
            println!("specifier: {:?}", specifier);
            if let ExportSpecifier::Named(named) = specifier {
                let is_type_only: bool = parent_is_type_only || named.is_type_only;
                // Handles `export { foo as bar }`
                if let Some(exported) = &named.exported {
                    if let ModuleExportName::Ident(id) = exported {
                        let sym = &id.sym;
                        // export { foo as default }
                        if sym == "default" {
                            self.exported_ids.insert(
                                ExportedSymbol::Default,
                                ExportedSymbolMetadata {
                                    span,
                                    allow_unused,
                                    is_type_only,
                                },
                            );
                        } else {
                            self.exported_ids.insert(
                                ExportedSymbol::Named(id.sym.to_string()),
                                ExportedSymbolMetadata {
                                    span,
                                    allow_unused,
                                    is_type_only,
                                },
                            );
                        }
                    }
                } else if let ModuleExportName::Ident(id) = &named.orig {
                    // handles `export { foo }`
                    self.exported_ids.insert(
                        ExportedSymbol::Named(id.sym.to_string()),
                        ExportedSymbolMetadata {
                            span,
                            allow_unused,
                            is_type_only,
                        },
                    );
                }
            }
        });
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

impl From<ExportsVisitor> for RawImportExportInfo {
    fn from(x: ExportsVisitor) -> Self {
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

impl Visit for ExportsVisitor {
    // Handles `export default foo`
    fn visit_export_default_expr(&mut self, expr: &ExportDefaultExpr) {
        expr.visit_children_with(self);
        self.exported_ids.insert(
            ExportedSymbol::Default,
            ExportedSymbolMetadata {
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
            ExportedSymbolMetadata {
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
                .map(|d| match &d.name {
                    Pat::Ident(ident) => ident.sym.to_string(),
                    _ => "".to_string(),
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
                ExportedSymbolMetadata {
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
            ReExportedSymbol {
                imported: ExportedSymbol::Namespace,
                renamed_to: None,
            },
            ExportedSymbolMetadata {
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
            // In case we find `'./foo'` in `export { foo } from './foo'`
            self.handle_export_from_specifiers(
                self.has_disable_export_comment(export.span_lo()),
                export.type_only,
                export,
                source,
            );
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
            .map(|spec| -> ExportedSymbol {
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
