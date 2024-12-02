// use super::{ExportedSymbol, ExportedSymbolMetadata, RawImportExportInfo, ReExportedSymbol};
use ahashmap::{AHashMap, AHashSet};
// use std::{collect`io`ns::HashSet, iter::FromIterator};
// use swc_common::{
//     comments::{CommentKind, Comments, SingleThreadedComments},
//     BytePos, Span, Spanned,
// };
// use swc_ecma_ast::{
//     BindingIdent, CallExpr, Callee, Decl, ExportAll, ExportDecl, ExportDefaultDecl,
//     ExportDefaultExpr, ExportSpecifier, Id, ImportDecl, ImportSpecifier, Lit, ModuleExportName,
//     NamedExport, Pat, Str, TsImportEqualsDecl,
// };
// use swc_ecma_visit::{Visit, VisitWith};

// // AST visitor that gathers information on file imports and exports from an SWC source tree.
// #[derive(Debug)]
// pub struct ExportsVisitor {
//     // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
//     pub imported_ids_path_name: AHashMap<String, AHashSet<ExportedSymbol>>,
//     // require('foo') generates ['foo']
//     pub require_paths: AHashSet<String>,
//     // import('./foo') and import './foo' generates ["./foo"]
//     pub imported_paths: AHashSet<String>,
//     // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
//     pub export_from_ids: AHashMap<String, AHashSet<ReExportedSymbol>>,
//     // IDs exported from this file, that were locally declared
//     pub exported_ids: AHashMap<ExportedSymbol, ExportedSymbolMetadata>,
//     // Side-effect-only imports.
//     // import './foo';
//     pub executed_paths: AHashSet<String>,
//     // exported from this file
//     // const foo = require('foo') generates ["foo"]
//     require_identifiers: AHashSet<Id>,
//     pub comments: SingleThreadedComments,
// }

// impl ExportsVisitor {
//     pub fn new(comments: SingleThreadedComments) -> Self {
//         Self {
//             imported_ids_path_name: AHashMap::default(),
//             require_paths: AHashSet::default(),
//             imported_paths: AHashSet::default(),
//             export_from_ids: AHashMap::default(),
//             executed_paths: AHashSet::default(),
//             require_identifiers: AHashSet::default(),
//             exported_ids: AHashMap::default(),
//             comments,
//         }
//     }

//     /**
//      * Extracts information from each specifier imported in source to treat it as an string
//      * Supported sytax list:
//      * - `export { foo as bar } from 'foo'`
//      * - `export { default as foo } from 'foo'`
//      * - `export { foo } from 'foo'`
//      */
//     fn handle_export_from_specifiers(&mut self, export: &NamedExport, source: &Str) {
//         let mut specifiers: Vec<ReExportedSymbol> = export
//             .specifiers
//             .iter()
//             .map(|spec| -> ReExportedSymbol {
//                 match spec {
//                     ExportSpecifier::Namespace(spec) => ReExportedSymbol {
//                         imported: ExportedSymbol::Namespace,
//                         renamed_to: Some(ExportedSymbol::from(&spec.name)),
//                     },
//                     ExportSpecifier::Default(spec) => ReExportedSymbol {
//                         imported: ExportedSymbol::Default,
//                         renamed_to: Some(ExportedSymbol::Named(spec.exported.to_string())),
//                     },
//                     ExportSpecifier::Named(spec) => {
//                         let imported_name = spec.orig.atom().to_string();
//                         let imported = if imported_name == "default" {
//                             ExportedSymbol::Default
//                         } else {
//                             ExportedSymbol::Named(imported_name)
//                         };
//                         ReExportedSymbol {
//                             imported,
//                             renamed_to: spec.exported.as_ref().map(ExportedSymbol::from),
//                         }
//                     }
//                 }
//             })
//             .collect();
//         if let Some(entry) = self.export_from_ids.get_mut(&source.value.to_string()) {
//             specifiers.drain(0..).for_each(|s| {
//                 entry.insert(s);
//             })
//         } else {
//             self.export_from_ids
//                 .insert(source.value.to_string(), HashSet::from_iter(specifiers));
//         }
//     }

//     /**
//      * Extracts information from the ExportSpecifier to construct the map of exported items with its values.
//      * supports `export { foo }` and aliased `export { foo as bar }`
//      */
//     fn handle_export_named_specifiers(
//         &mut self,
//         specs: &[ExportSpecifier],
//         allow_unused: bool,
//         span: Span,
//     ) {
//         specs.iter().for_each(|specifier| {
//             if let ExportSpecifier::Named(named) = specifier {
//                 // Handles `export { foo as bar }`
//                 if let Some(exported) = &named.exported {
//                     if let ModuleExportName::Ident(id) = exported {
//                         let sym = id.sym.to_string();
//                         // export { foo as default }
//                         if sym == "default" {
//                             self.exported_ids.insert(
//                                 ExportedSymbol::Default,
//                                 ExportedSymbolMetadata { span, allow_unused },
//                             );
//                         } else {
//                             self.exported_ids.insert(
//                                 ExportedSymbol::Named(id.sym.to_string()),
//                                 ExportedSymbolMetadata { span, allow_unused },
//                             );
//                         }
//                     }
//                 } else if let ModuleExportName::Ident(id) = &named.orig {
//                     // handles `export { foo }`
//                     self.exported_ids.insert(
//                         ExportedSymbol::Named(id.sym.to_string()),
//                         ExportedSymbolMetadata { span, allow_unused },
//                     );
//                 }
//             }
//         });
//     }

//     pub fn has_disable_export_comment(&self, lo: BytePos) -> bool {
//         if let Some(comments) = self.comments.get_leading(lo) {
//             return comments.iter().any(|c| {
//                 c.kind == CommentKind::Line && c.text.trim().starts_with("@ALLOW-UNUSED-EXPORT")
//             });
//         }
//         false
//     }
// }

// impl From<ExportsVisitor> for RawImportExportInfo {
//     fn from(x: ExportsVisitor) -> Self {
//         Self {
//             imported_path_ids: x.imported_ids_path_name,
//             require_paths: x.require_paths,
//             imported_paths: x.imported_paths,
//             export_from_ids: x.export_from_ids, // TODO replace with Exportx maps
//             exported_ids: x.exported_ids,
//             executed_paths: x.executed_paths,
//         }
//     }
// }

// impl Visit for ExportsVisitor {
//     // Handles `export default foo`
//     fn visit_export_default_expr(&mut self, expr: &ExportDefaultExpr) {
//         expr.visit_children_with(self);
//         if !self.has_disable_export_comment(expr.span_lo()) {
//             self.exported_ids.insert(
//                 ExportedSymbol::Default,
//                 ExportedSymbolMetadata {
//                     span: expr.span(),
//                     allow_unused: self.has_disable_export_comment(expr.span_lo()),
//                 },
//             );
//         }
//     }

//     /**
//      * Handles scenarios where `export default` has an inline declaration, e.g. `export default class Foo {}` or `export default function foo() {}`
//      */
//     fn visit_export_default_decl(&mut self, decl: &ExportDefaultDecl) {
//         decl.visit_children_with(self);
//         if !self.has_disable_export_comment(decl.span_lo()) {
//             self.exported_ids.insert(
//                 ExportedSymbol::Default,
//                 ExportedSymbolMetadata {
//                     span: decl.span(),
//                     allow_unused: self.has_disable_export_comment(decl.span_lo()),
//                 },
//             );
//         }
//     }

//     // Handles scenarios `export` has an inline declaration, e.g. `export const foo = 1` or `export class Foo {}`
//     fn visit_export_decl(&mut self, export: &ExportDecl) {
//         export.visit_children_with(self);
//         let allow_unused = self.has_disable_export_comment(export.span_lo());
//         match &export.decl {
//             Decl::Class(decl) => {
//                 // export class Foo {}
//                 self.exported_ids.insert(
//                     ExportedSymbol::Named(decl.ident.sym.to_string()),
//                     ExportedSymbolMetadata {
//                         span: export.span(),
//                         allow_unused,
//                     },
//                 );
//             }
//             Decl::Fn(decl) => {
//                 // export function foo() {}
//                 self.exported_ids.insert(
//                     ExportedSymbol::Named(decl.ident.sym.to_string()),
//                     ExportedSymbolMetadata {
//                         span: export.span(),
//                         allow_unused,
//                     },
//                 );
//             }
//             Decl::Var(decl) => {
//                 // export const foo = 1;
//                 if let Some(d) = decl.decls.first() {
//                     if let Pat::Ident(ident) = &d.name {
//                         self.exported_ids.insert(
//                             ExportedSymbol::Named(ident.sym.to_string()),
//                             ExportedSymbolMetadata {
//                                 span: export.span(),
//                                 allow_unused,
//                             },
//                         );
//                     }
//                 }
//             }
//             Decl::TsInterface(decl) => {
//                 // export interface Foo {}
//                 self.exported_ids.insert(
//                     ExportedSymbol::Named(decl.id.sym.to_string()),
//                     ExportedSymbolMetadata {
//                         span: export.span(),
//                         allow_unused,
//                     },
//                 );
//             }
//             Decl::TsTypeAlias(decl) => {
//                 // export type foo = string
//                 self.exported_ids.insert(
//                     ExportedSymbol::Named(decl.id.sym.to_string()),
//                     ExportedSymbolMetadata {
//                         span: export.span(),
//                         allow_unused,
//                     },
//                 );
//             }
//             Decl::TsEnum(decl) => {
//                 // export enum Foo { foo, bar }
//                 self.exported_ids.insert(
//                     ExportedSymbol::Named(decl.id.sym.to_string()),
//                     ExportedSymbolMetadata {
//                         span: export.span(),
//                         allow_unused,
//                     },
//                 );
//             }
//             Decl::TsModule(_decl) => {
//                 // if let Some(module_name) = decl.id.as_str() {
//                 //     self.exported_ids.insert(ExportedItem::Named(module_name.value.to_string()));
//                 // }
//             }
//             Decl::Using(_) => {}
//         }
//     }

//     // `export * from './foo'`; // TODO allow recursive import resolution
//     fn visit_export_all(&mut self, export: &ExportAll) {
//         export.visit_children_with(self);
//         let source = export.src.value.to_string();
//         if self.has_disable_export_comment(export.span_lo()) {
//             return;
//         }
//         self.export_from_ids.insert(
//             source,
//             HashSet::from_iter([ReExportedSymbol {
//                 imported: ExportedSymbol::Namespace,
//                 renamed_to: None,
//             }]),
//         );
//     }

//     // export {foo} from './foo';
//     fn visit_named_export(&mut self, export: &NamedExport) {
//         export.visit_children_with(self);
//         if let Some(source) = &export.src {
//             // In case we find `'./foo'` in `export { foo } from './foo'`
//             if self.has_disable_export_comment(export.span_lo()) {
//                 return;
//             }
//             self.handle_export_from_specifiers(export, source);
//         } else {
//             self.handle_export_named_specifiers(
//                 &export.specifiers,
//                 self.has_disable_export_comment(export.span_lo()),
//                 export.span(),
//             );
//         }
//     }

//     // const foo = require; // <- Binding
//     // const p = foo('./path')
//     fn visit_binding_ident(&mut self, binding: &BindingIdent) {
//         binding.visit_children_with(self);
//         if binding.sym == *"require" {
//             self.require_identifiers.insert(binding.id.to_id());
//         }
//     }

//     // import foo = require('./foo')
//     fn visit_ts_import_equals_decl(&mut self, decl: &TsImportEqualsDecl) {
//         decl.visit_children_with(self);
//         if let Some(module_ref) = decl.module_ref.as_ts_external_module_ref() {
//             self.imported_paths
//                 .insert(module_ref.expr.value.to_string());
//         }
//     }

//     // import('foo')
//     // or
//     // require('foo')
//     fn visit_call_expr(&mut self, expr: &CallExpr) {
//         expr.visit_children_with(self);
//         if let Callee::Import(_) = &expr.callee {
//             match extract_argument_value(expr) {
//                 Some(import_path) => {
//                     self.imported_paths.insert(import_path);
//                 }
//                 None => return,
//             }
//         }
//         if let Callee::Expr(callee) = &expr.callee {
//             if let Some(ident) = callee.as_ident() {
//                 if ident.sym == "require" && !self.require_identifiers.contains(&ident.to_id()) {
//                     if let Some(import_path) = extract_argument_value(expr) {
//                         self.require_paths.insert(import_path);
//                     }
//                 }
//             }
//         }
//     }

//     // import foo from './foo';
//     fn visit_import_decl(&mut self, import: &ImportDecl) {
//         import.visit_children_with(self);

//         let src = import.src.value.to_string();
//         // import './foo';
//         if import.specifiers.is_empty() {
//             self.executed_paths.insert(src);
//             return;
//         }
//         // import .. from ..
//         let mut specifiers: Vec<ExportedSymbol> = import
//             .specifiers
//             .iter()
//             .map(|spec| -> ExportedSymbol {
//                 match spec {
//                     ImportSpecifier::Named(named) => {
//                         match &named.imported {
//                             Some(module_name) => {
//                                 // import { foo as bar } from './foo'
//                                 match module_name {
//                                     ModuleExportName::Ident(ident) => {
//                                         // sym_str = foo in `import { foo as bar } from './foo'`
//                                         let sym_str = ident.sym.to_string();
//                                         if sym_str == "default" {
//                                             // import { default as foo } from 'foo'
//                                             return ExportedSymbol::Default;
//                                         }
//                                         ExportedSymbol::Named(sym_str)
//                                     }
//                                     ModuleExportName::Str(s) => {
//                                         ExportedSymbol::Named(s.value.to_string())
//                                     }
//                                 }
//                             }
//                             None => {
//                                 // import { foo } from './foo'
//                                 ExportedSymbol::Named(named.local.sym.to_string())
//                             }
//                         }
//                     }
//                     ImportSpecifier::Default(_) => {
//                         // import foo from 'foo'
//                         ExportedSymbol::Default
//                     }
//                     ImportSpecifier::Namespace(_) => {
//                         // import * as foo from 'foo'
//                         ExportedSymbol::Namespace
//                     }
//                 }
//             })
//             .collect();

//         if let Some(entry) = self.imported_ids_path_name.get_mut(&src) {
//             specifiers.drain(0..).for_each(|s| {
//                 entry.insert(s);
//             });
//         } else {
//             self.imported_ids_path_name
//                 .insert(src, HashSet::from_iter(specifiers));
//         }
//     }
// }

// fn extract_argument_value(expr: &CallExpr) -> Option<String> {
//     let import_path = match expr.args.is_empty() {
//         true => return None,
//         false => expr.args.first(),
//     };
//     if let Some(path) = import_path {
//         if let Some(path_lit) = path.expr.as_lit() {
//             match path_lit {
//                 Lit::Str(value) => {
//                     return Some(value.value.to_string());
//                 }
//                 _ => return None,
//             }
//         }
//     }
//     None
// }

use logger_srcfile::SrcFileLogger;
use swc_atoms::Atom;
use swc_common::{Span, Spanned};
use swc_ecma_visit::{Visit, VisitWith};

use crate::Segment;

struct Visitor {
    segments: Vec<Segment>,
}

impl Visit for Visitor {
    fn visit_module(&mut self, node: &swc_ecma_ast::Module) {
        for module_item in node.body.iter() {
            let stmt = match module_item {
                swc_ecma_ast::ModuleItem::Stmt(stmt) => stmt,
                swc_ecma_ast::ModuleItem::ModuleDecl(_) => {
                    // typescript module declarations have no concrete output, so we skip them
                    continue;
                }
            };

            let segment: Segment = match stmt {
                swc_ecma_ast::Stmt::Decl(decl) => {
                    // TODO: declare a segment with one or more names
                    continue;
                }
                swc_ecma_ast::Stmt::Expr(_) => {
                    // TODO: side-effect segment
                    continue;
                }
                swc_ecma_ast::Stmt::Block(_) => {
                    // TODO: visit the block children?
                    continue;
                }
                swc_ecma_ast::Stmt::Empty(_) => {
                    // noop
                    continue;
                }
                swc_ecma_ast::Stmt::Debugger(_) => {
                    // TODO: sideEffectOnly segment?
                    continue;
                }
                swc_ecma_ast::Stmt::Labeled(_) => {
                    // TODO: visit the labaled statement's child
                    continue;
                }
                swc_ecma_ast::Stmt::Switch(_) => {
                    // TODO: capture referenced values within the switch
                    continue;
                }
                swc_ecma_ast::Stmt::If(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement
                    continue;
                }
                swc_ecma_ast::Stmt::Throw(_) => {
                    // TODO: side-effect segment
                    continue;
                }
                swc_ecma_ast::Stmt::Try(_) => {
                    // TODO: side-effect statement
                    continue;
                }
                swc_ecma_ast::Stmt::While(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement and the loop condition
                    continue;
                }
                swc_ecma_ast::Stmt::DoWhile(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement and the loop condition
                    continue;
                }
                swc_ecma_ast::Stmt::For(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement and the loop condition
                    continue;
                }
                swc_ecma_ast::Stmt::ForIn(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement and the loop condition
                    continue;
                }
                swc_ecma_ast::Stmt::ForOf(_) => {
                    // TODO: side-effect statement that collects names from
                    // the children of the if statement and the loop condition
                    continue;
                }
                swc_ecma_ast::Stmt::With(_) => {
                    // TODO: record an error here,
                    // with statements are deprecated and unsupported
                    // See: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/with
                    continue;
                }
                swc_ecma_ast::Stmt::Return(_)
                | swc_ecma_ast::Stmt::Break(_)
                | swc_ecma_ast::Stmt::Continue(_) => {
                    // TODO: record an error here,
                    // These statements can't exist at the module scope
                    continue;
                }
            };

            self.segments.push(segment);
        }
    }
}

// unique identifier of a variable declaration within a file
#[derive(Clone, Copy)]
struct VarID(Span);

struct VariableScope {
    /// Variables declared within the current scope, sorted by name
    local_symbols: AHashMap<swc_atoms::Atom, VarID>,

    // Names in this scope that are "hoisted"
    // This is type 1/3 hoisting from here, where a name is considered pre-declared for
    // the entire scope, even if it is declared later in the source.
    //
    // https://developer.mozilla.org/en-US/docs/Glossary/Hoisting
    escaped_symbols: AHashSet<swc_atoms::Atom>,
}

#[derive(thiserror::Error, Debug)]
pub enum VariableScopeError {
    #[error("Variable {0} is already declared in this scope")]
    DuplicateDeclaration(swc_atoms::Atom),
}

impl VariableScope {
    fn declare_local(&mut self, ident: &swc_ecma_ast::Ident) -> Result<(), VariableScopeError> {
        match self.local_symbols.entry(ident.sym.clone()) {
            ahashmap::hash_map::Entry::Occupied(_) => {
                // Should you warn here about a name collision within the scope?
                Err(VariableScopeError::DuplicateDeclaration(ident.sym.clone()))
            }
            ahashmap::hash_map::Entry::Vacant(entry) => {
                entry.insert(VarID(ident.span));
                Ok(())
            }
        }
    }
}

struct VariableScopeTreeNode {
    scope: VariableScope,
    children: Vec<Box<VariableScopeTreeNode>>,
}

impl VariableScopeTreeNode {
    fn new() -> VariableScopeTreeNode {
        VariableScopeTreeNode {
            scope: VariableScope {
                local_symbols: AHashMap::default(),
                escaped_symbols: AHashSet::default(),
            },
            children: Vec::new(),
        }
    }

    fn register_ident(&mut self, ident: &swc_ecma_ast::Ident) -> Result<(), VariableScopeError> {
        self.scope.declare_local(ident)
    }

    fn register_hoisted_ident(
        &mut self,
        ident: &swc_ecma_ast::Ident,
    ) -> Result<(), VariableScopeError> {
        // delete all escaped symbols with the hoisted symbol's name
        self.remove_escaped_symbol(&ident.sym);
        self.scope.declare_local(ident)
    }

    /// Remove a symbol from the list of escaped symbols in this scope and all child scopes.
    ///
    /// For use when we encounter a "hoisted" symbol which is declared in the source after
    /// it first becomes available.
    fn remove_escaped_symbol(&mut self, sym: &swc_atoms::Atom) {
        self.scope.escaped_symbols.remove(sym);
        for child in &mut self.children {
            child.remove_escaped_symbol(sym);
        }
    }

    // Create a new child scope and return a mutable reference to it.
    fn new_child_scope(&mut self) -> &mut VariableScopeTreeNode {
        let child = VariableScopeTreeNode::new();
        self.children.push(Box::new(child));
        self.children.last_mut().unwrap()
    }

    // "uses" a symbol within this scope.
    //
    // If the symbol is not declared in this scope, it is added to the list of escaped symbols.
    fn use_symbol(&mut self, sym: &swc_atoms::Atom) {
        if !self.scope.local_symbols.contains_key(sym) {
            self.scope.escaped_symbols.insert(sym.clone());
        }
    }

    // owned version of use_symbol
    fn use_symbol_owned(&mut self, sym: swc_atoms::Atom) {
        if !self.scope.local_symbols.contains_key(&sym) {
            self.scope.escaped_symbols.insert(sym);
        }
    }
}

/// Visitor that builds a VariableScope from a source file.
struct VariableScopeVisitor<'a, TLogger: SrcFileLogger> {
    logger: &'a TLogger,
    node: &'a mut VariableScopeTreeNode,
}

impl<'a, TLogger> VariableScopeVisitor<'a, TLogger>
where
    TLogger: SrcFileLogger,
{
    fn new(
        logger: &'a TLogger,
        root_scope: &'a mut VariableScopeTreeNode,
    ) -> VariableScopeVisitor<'a, TLogger> {
        Self {
            logger,
            node: root_scope,
        }
    }

    fn visit_binding_pattern(&mut self, pattern: &swc_ecma_ast::Pat) {
        match pattern {
            swc_ecma_ast::Pat::Ident(ident) => {
                self.register_ident(ident);
            }
            swc_ecma_ast::Pat::Array(array_pat) => {
                for elem in &array_pat.elems {
                    if let Some(subpattern) = elem {
                        self.visit_binding_pattern(subpattern);
                    }
                }
            }
            swc_ecma_ast::Pat::Object(object_pat) => {
                for prop in &object_pat.props {
                    match prop {
                        swc_ecma_ast::ObjectPatProp::KeyValue(kv) => {
                            self.visit_binding_pattern(&kv.value);
                        }
                        swc_ecma_ast::ObjectPatProp::Assign(assign_prop) => {
                            // little used "destructure with default" syntax:
                            // let { a = defaultValue } = destructured_object;
                            self.register_ident(&assign_prop.key.id)
                        }
                        swc_ecma_ast::ObjectPatProp::Rest(rest) => {
                            self.visit_binding_pattern(&rest.arg);
                        }
                    }
                }
            }
            swc_ecma_ast::Pat::Rest(rest_pat) => {
                self.visit_binding_pattern(&rest_pat.arg);
            }
            swc_ecma_ast::Pat::Assign(assign_pat) => {
                self.visit_binding_pattern(&assign_pat.left);
                // visit the right side of the binding pattern.
                assign_pat.right.visit_with(self);
            }
            swc_ecma_ast::Pat::Invalid(invalid_pat) => {
                self.logger
                    .src_warn(&invalid_pat.span, "invalid pattern in variable declaration");
            }
            swc_ecma_ast::Pat::Expr(expr_pat) => {
                self.logger.src_warn(
                    &expr_pat.leftmost().span(),
                    "expr pattern in variable declaration was ignored",
                );
            }
        }
    }

    fn register_ident(&mut self, ident: &swc_ecma_ast::Ident) {
        if let Err(e) = self.node.register_ident(ident) {
            self.logger.src_error(&ident.span, &format!("{}", e));
        }
    }

    fn register_hoisted_ident(&mut self, ident: &swc_ecma_ast::Ident) {
        if let Err(e) = self.node.register_hoisted_ident(ident) {
            self.logger.src_error(&ident.span, &format!("{}", e));
        }
    }

    fn new_child_scope(&mut self) -> VariableScopeVisitor<'_, TLogger> {
        let child = self.node.new_child_scope();
        VariableScopeVisitor {
            logger: &self.logger,
            node: child,
        }
    }

    fn mark_all_escaped(&mut self, mut child_scope_escaped_symbols: AHashSet<Atom>) {
        for sym in child_scope_escaped_symbols.drain() {
            self.node.use_symbol_owned(sym);
        }
    }
}

impl<TLogger> Visit for VariableScopeVisitor<'_, TLogger>
where
    TLogger: SrcFileLogger,
{
    fn visit_fn_decl(&mut self, node: &swc_ecma_ast::FnDecl) {
        self.register_hoisted_ident(&node.ident);
        // Create a new child scope, stored inside the current scope.
        let child_scope = &mut self.new_child_scope();
        // pre-declare function parameters in the child scope
        for param in &node.function.params {
            child_scope.visit_binding_pattern(&param.pat);
        }
        // visit the child scope
        node.function.body.visit_with(child_scope);
        // copy the escaped symbols from the child scope into the current scope
        // (TODO: figure our how to do this without cloning?)
        let escaped = child_scope.node.scope.escaped_symbols.clone();
        self.mark_all_escaped(escaped);
    }

    fn visit_fn_expr(&mut self, node: &swc_ecma_ast::FnExpr) {
        // Create a new child scope, stored inside the current scope.
        let child_scope = &mut self.new_child_scope();
        // pre-declare function parameters in the child scope
        for param in &node.function.params {
            child_scope.visit_binding_pattern(&param.pat);
        }
        // visit the child scope
        node.function.body.visit_with(child_scope);
        // copy the escaped symbols from the child scope into the current scope
        // (TODO: figure our how to do this without cloning?)
        let escaped = child_scope.node.scope.escaped_symbols.clone();
        self.mark_all_escaped(escaped);
    }

    fn visit_block_stmt(&mut self, node: &swc_ecma_ast::BlockStmt) {
        // Create a new child scope, stored inside the current scope.
        let child_scope = &mut self.new_child_scope();
        // visit the child scope
        node.visit_children_with(child_scope);
        // (TODO: figure our how to do this without cloning?)
        let escaped = child_scope.node.scope.escaped_symbols.clone();
        self.mark_all_escaped(escaped);
    }

    fn visit_ident_name(&mut self, node: &swc_ecma_ast::IdentName) {
        self.node.use_symbol(&node.sym);
    }
}
