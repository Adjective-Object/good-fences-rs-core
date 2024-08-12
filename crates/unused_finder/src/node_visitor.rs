use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    iter::FromIterator,
    sync::Arc,
};

use swc_core::{
    common::{
        comments::{CommentKind, Comments, SingleThreadedComments},
        BytePos, Span, Spanned,
    },
    ecma::{
        ast::{
            BindingIdent, CallExpr, Callee, Decl, ExportAll, ExportDecl, ExportDefaultDecl,
            ExportDefaultExpr, ExportSpecifier, Id, ImportDecl, ImportSpecifier, Lit,
            ModuleExportName, NamedExport, Pat, Str, TsImportEqualsDecl,
        },
        visit::{Visit, VisitWith},
    },
};

#[derive(Debug, Default, Eq, PartialEq, Clone, Hash)]
pub struct ExportedItemMetadata {
    pub export_kind: ExportKind,
    pub span: Span,
    pub allow_unused: bool,
}

impl ExportedItemMetadata {
    pub fn new(export_type: ExportKind, span: Span, allow_unused: bool) -> Self {
        Self {
            export_kind: export_type,
            span,
            allow_unused,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ExportKind {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl Display for ExportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportKind::Named(name) => write!(f, "{}", name),
            ExportKind::Default => write!(f, "default"),
            ExportKind::Namespace => write!(f, "*"),
            ExportKind::ExecutionOnly => write!(f, "import '<path>'"),
        }
    }
}

impl Default for ExportKind {
    fn default() -> Self {
        Self::Default
    }
}

impl From<&ImportedItem> for ExportKind {
    fn from(i: &ImportedItem) -> Self {
        match i {
            ImportedItem::Named(named) => ExportKind::Named(named.clone()),
            ImportedItem::Default => ExportKind::Default,
            ImportedItem::Namespace => ExportKind::Namespace,
            ImportedItem::ExecutionOnly => ExportKind::ExecutionOnly,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ImportedItem {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl From<&ExportKind> for ImportedItem {
    fn from(e: &ExportKind) -> Self {
        match e {
            ExportKind::Named(name) => ImportedItem::Named(name.clone()),
            ExportKind::Default => ImportedItem::Default,
            ExportKind::Namespace => ImportedItem::Namespace,
            ExportKind::ExecutionOnly => ImportedItem::ExecutionOnly,
        }
    }
}

#[derive(Debug)]
pub struct ExportsCollector {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_ids_path_name: HashMap<String, HashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: HashSet<String>,
    // import('./foo') and import './foo' generates ["./foo"]
    pub imported_paths: HashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: HashMap<String, HashSet<ImportedItem>>,
    // IDs exported from this file, that were locally declared
    pub exported_ids: HashSet<ExportedItemMetadata>,
    // Side-effect-only imports.
    // import './foo';
    pub executed_paths: HashSet<String>,
    // exported from this file
    // const foo = require('foo') generates ["foo"]
    require_identifiers: HashSet<Id>,
    skipped_items: Arc<Vec<regex::Regex>>,
    pub comments: SingleThreadedComments,
}

impl ExportsCollector {
    pub fn new(skipped_items: Arc<Vec<regex::Regex>>, comments: SingleThreadedComments) -> Self {
        Self {
            imported_ids_path_name: HashMap::new(),
            require_paths: HashSet::new(),
            imported_paths: HashSet::new(),
            export_from_ids: HashMap::new(),
            executed_paths: HashSet::new(),
            require_identifiers: HashSet::new(),
            exported_ids: HashSet::new(),
            skipped_items,
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
    fn handle_export_from_specifiers(&mut self, export: &NamedExport, source: &Str) {
        let mut specifiers: Vec<ImportedItem> = export
            .specifiers
            .iter()
            .filter_map(|spec| -> Option<ImportedItem> {
                if spec.is_namespace() {
                    // export * as foo from 'foo;
                    return Some(ImportedItem::Namespace);
                }
                if let Some(named) = spec.as_named() {
                    // export { foo } from 'foo'
                    if let ModuleExportName::Ident(ident) = &named.orig {
                        // export { default as foo } from 'foo'
                        if ident.sym.to_string() == "default" {
                            return Some(ImportedItem::Default);
                        }
                        // export { foo } from 'foo'
                        if !self
                            .skipped_items
                            .iter()
                            .any(|skipped| skipped.is_match(&ident.sym.to_string()))
                        {
                            return Some(ImportedItem::Named(ident.sym.to_string()));
                        }
                    }
                }
                return None;
            })
            .collect();
        if let Some(entry) = self.export_from_ids.get_mut(&source.value.to_string()) {
            specifiers.drain(0..).for_each(|s| {
                entry.insert(s);
            })
        } else {
            self.export_from_ids
                .insert(source.value.to_string(), HashSet::from_iter(specifiers));
        }
    }

    /**
     * Extracts information from the ExportSpecifier to construct the map of exported items with its values.
     * supports `export { foo }` and aliased `export { foo as bar }`
     */
    fn handle_export_named_specifiers(
        &mut self,
        specs: &Vec<ExportSpecifier>,
        allow_unused: bool,
        span: Span,
    ) {
        specs.iter().for_each(|specifier| match specifier {
            ExportSpecifier::Named(named) => {
                // Handles `export { foo as bar }`
                if let Some(exported) = &named.exported {
                    if let ModuleExportName::Ident(id) = exported {
                        let sym = id.sym.to_string();
                        // export { foo as default }
                        if sym == "default" {
                            self.exported_ids.insert(ExportedItemMetadata {
                                export_kind: ExportKind::Default,
                                span,
                                allow_unused,
                            });
                        } else {
                            if !self
                                .skipped_items
                                .iter()
                                .any(|skipped| skipped.is_match(&sym))
                            {
                                self.exported_ids.insert(ExportedItemMetadata {
                                    export_kind: ExportKind::Named(id.sym.to_string()),
                                    span,
                                    allow_unused,
                                });
                            }
                        }
                    }
                } else if let ModuleExportName::Ident(id) = &named.orig {
                    // handles `export { foo }`
                    if !self
                        .skipped_items
                        .iter()
                        .any(|skipped| skipped.is_match(&id.sym.to_string()))
                    {
                        self.exported_ids.insert(ExportedItemMetadata {
                            export_kind: ExportKind::Named(id.sym.to_string()),
                            span,
                            allow_unused,
                        });
                    }
                }
            }
            _ => {}
        });
    }

    pub fn has_disable_export_comment(&self, lo: BytePos) -> bool {
        if let Some(comments) = self.comments.get_leading(lo) {
            return comments.iter().any(|c| {
                c.kind == CommentKind::Line && c.text.trim().starts_with("@ALLOW-UNUSED-EXPORT")
            });
        }
        false
    }
}

impl Visit for ExportsCollector {
    // Handles `export default foo`
    fn visit_export_default_expr(&mut self, expr: &ExportDefaultExpr) {
        expr.visit_children_with(self);
        if !self.has_disable_export_comment(expr.span_lo()) {
            self.exported_ids.insert(ExportedItemMetadata {
                export_kind: ExportKind::Default,
                span: expr.span(),
                allow_unused: self.has_disable_export_comment(expr.span_lo()),
            });
        }
    }

    /**
     * Handles scenarios where `export default` has an inline declaration, e.g. `export default class Foo {}` or `export default function foo() {}`
     */
    fn visit_export_default_decl(&mut self, decl: &ExportDefaultDecl) {
        decl.visit_children_with(self);
        if !self.has_disable_export_comment(decl.span_lo()) {
            self.exported_ids.insert(ExportedItemMetadata {
                export_kind: ExportKind::Default,
                span: decl.span(),
                allow_unused: self.has_disable_export_comment(decl.span_lo()),
            });
        }
    }

    // Handles scenarios `export` has an inline declaration, e.g. `export const foo = 1` or `export class Foo {}`
    fn visit_export_decl(&mut self, export: &ExportDecl) {
        export.visit_children_with(self);
        let allow_unused = self.has_disable_export_comment(export.span_lo());
        match &export.decl {
            Decl::Class(decl) => {
                // export class Foo {}
                if !self
                    .skipped_items
                    .iter()
                    .any(|skipped| skipped.is_match(&decl.ident.sym.to_string()))
                {
                    self.exported_ids.insert(ExportedItemMetadata {
                        export_kind: ExportKind::Named(decl.ident.sym.to_string()),
                        span: export.span(),
                        allow_unused,
                    });
                }
            }
            Decl::Fn(decl) => {
                // export function foo() {}
                if !self
                    .skipped_items
                    .iter()
                    .any(|skipped| skipped.is_match(&decl.ident.sym.to_string()))
                {
                    self.exported_ids.insert(ExportedItemMetadata {
                        export_kind: ExportKind::Named(decl.ident.sym.to_string()),
                        span: export.span(),
                        allow_unused,
                    });
                }
            }
            Decl::Var(decl) => {
                // export const foo = 1;
                if let Some(d) = decl.decls.first() {
                    if let Pat::Ident(ident) = &d.name {
                        if !self
                            .skipped_items
                            .iter()
                            .any(|skipped| skipped.is_match(&ident.sym.to_string()))
                        {
                            self.exported_ids.insert(ExportedItemMetadata {
                                export_kind: ExportKind::Named(ident.sym.to_string()),
                                span: export.span(),
                                allow_unused,
                            });
                        }
                    }
                }
            }
            Decl::TsInterface(decl) => {
                // export interface Foo {}
                if !self
                    .skipped_items
                    .iter()
                    .any(|skipped| skipped.is_match(&decl.id.sym.to_string()))
                {
                    self.exported_ids.insert(ExportedItemMetadata {
                        export_kind: ExportKind::Named(decl.id.sym.to_string()),
                        span: export.span(),
                        allow_unused,
                    });
                }
            }
            Decl::TsTypeAlias(decl) => {
                // export type foo = string
                if !self
                    .skipped_items
                    .iter()
                    .any(|skipped| skipped.is_match(&decl.id.sym.to_string()))
                {
                    self.exported_ids.insert(ExportedItemMetadata {
                        export_kind: ExportKind::Named(decl.id.sym.to_string()),
                        span: export.span(),
                        allow_unused,
                    });
                }
            }
            Decl::TsEnum(decl) => {
                // export enum Foo { foo, bar }
                if !self
                    .skipped_items
                    .iter()
                    .any(|skipped| skipped.is_match(&decl.id.sym.to_string()))
                {
                    self.exported_ids.insert(ExportedItemMetadata {
                        export_kind: ExportKind::Named(decl.id.sym.to_string()),
                        span: export.span(),
                        allow_unused,
                    });
                }
            }
            Decl::TsModule(_decl) => {
                // if let Some(module_name) = decl.id.as_str() {
                //     self.exported_ids.insert(ExportedItem::Named(module_name.value.to_string()));
                // }
            }
            Decl::Using(_) => {}
        }
    }

    // `export * from './foo'`; // TODO allow recursive import resolution
    fn visit_export_all(&mut self, export: &ExportAll) {
        export.visit_children_with(self);
        let source = export.src.value.to_string();
        if self.has_disable_export_comment(export.span_lo()) {
            return;
        }
        self.export_from_ids
            .insert(source, HashSet::from_iter(vec![ImportedItem::Namespace]));
    }

    // export {foo} from './foo';
    fn visit_named_export(&mut self, export: &NamedExport) {
        export.visit_children_with(self);
        if let Some(source) = &export.src {
            // In case we find `'./foo'` in `export { foo } from './foo'`
            if self.has_disable_export_comment(export.span_lo()) {
                return;
            }
            self.handle_export_from_specifiers(export, source);
        } else {
            self.handle_export_named_specifiers(
                &export.specifiers,
                self.has_disable_export_comment(export.span_lo()),
                export.span(),
            );
        }
    }

    // const foo = require; // <- Binding
    // const p = foo('./path')
    fn visit_binding_ident(&mut self, binding: &BindingIdent) {
        binding.visit_children_with(self);
        if binding.sym.to_string() == "require".to_string() {
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
                if ident.sym.to_string() == "require" {
                    if !self.require_identifiers.contains(&ident.to_id()) {
                        match extract_argument_value(expr) {
                            Some(import_path) => {
                                self.require_paths.insert(import_path);
                            }
                            None => return,
                        }
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
        let mut specifiers: Vec<ExportKind> =
            import
                .specifiers
                .iter()
                .filter_map(|spec| -> Option<ExportKind> {
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
                                                if !self.skipped_items.iter().any(|s| {
                                                    s.is_match(&named.local.sym.to_string())
                                                }) {
                                                    return Some(ExportKind::Default);
                                                }
                                            }
                                            if !self
                                                .skipped_items
                                                .iter()
                                                .any(|skipped| skipped.is_match(&sym_str))
                                            {
                                                return Some(ExportKind::Named(sym_str));
                                            }
                                            None
                                        }
                                        ModuleExportName::Str(s) => {
                                            if !self.skipped_items.iter().any(|skipped| {
                                                skipped.is_match(&s.value.to_string())
                                            }) {
                                                return Some(ExportKind::Named(
                                                    s.value.to_string(),
                                                ));
                                            }
                                            None
                                        }
                                    }
                                }
                                None => {
                                    // import { foo } from './foo'
                                    if !self.skipped_items.iter().any(|skipped| {
                                        skipped.is_match(&named.local.sym.to_string())
                                    }) {
                                        return Some(ExportKind::Named(
                                            named.local.sym.to_string(),
                                        ));
                                    }
                                    None
                                }
                            }
                        }
                        ImportSpecifier::Default(_) => {
                            // import foo from 'foo'
                            return Some(ExportKind::Default);
                        }
                        ImportSpecifier::Namespace(_) => {
                            // import * as foo from 'foo'
                            return Some(ExportKind::Namespace);
                        }
                    }
                })
                .collect();

        if let Some(entry) = self.imported_ids_path_name.get_mut(&src) {
            specifiers.drain(0..).for_each(|s| {
                entry.insert(ImportedItem::from(&s));
            });
        } else {
            self.imported_ids_path_name.insert(
                src,
                HashSet::from_iter(specifiers.iter().map(|s| ImportedItem::from(s))),
            );
        }
    }
}

fn extract_argument_value(expr: &CallExpr) -> Option<String> {
    let import_path = match expr.args.is_empty() {
        true => return None,
        false => expr.args.get(0),
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
