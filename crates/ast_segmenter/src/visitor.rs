// use ahashmap::{AHashMap, AHashSet};
// use ast_name_tracker::VariableScope;
// use logger_srcfile::SrcFileLogger;
// use swc_common::Spanned;

// use crate::{NormalSegment, Segment, SegmentKind};

// struct Visitor {
//     segments: Vec<SegmentKind>,
// }

// const RETURN: &str = "return";
// const BREAK: &str = "break";
// const CONTINUE: &str = "continue";

// #[derive(thiserror::Error, Debug)]
// pub enum StatementToSegmentError {
//     #[error("\"{}\" statements should not occur at the module level", .0)]
//     StatementUnexpectedInModuleScope(&'static str),
//     #[error("With statements are not supported because they create new non-lexical names")]
//     WithStatmentUnsupported,
// }

// fn module_item_to_segment(
//     file_logger: &impl SrcFileLogger,
//     module_item: &swc_ecma_ast::ModuleItem,
// ) -> Option<Segment> {
//     match module_item {
//         swc_ecma_ast::ModuleItem::Stmt(stmt) => {
//             match stmt {
//                 swc_ecma_ast::Stmt::Decl(_)
//                 | swc_ecma_ast::Stmt::Expr(_)
//                 | swc_ecma_ast::Stmt::Block(_)
//                 | swc_ecma_ast::Stmt::Empty(_)
//                 | swc_ecma_ast::Stmt::Debugger(_)
//                 | swc_ecma_ast::Stmt::Labeled(_)
//                 | swc_ecma_ast::Stmt::Switch(_)
//                 | swc_ecma_ast::Stmt::If(_)
//                 | swc_ecma_ast::Stmt::Throw(_)
//                 | swc_ecma_ast::Stmt::Try(_)
//                 | swc_ecma_ast::Stmt::While(_)
//                 | swc_ecma_ast::Stmt::DoWhile(_)
//                 | swc_ecma_ast::Stmt::For(_)
//                 | swc_ecma_ast::Stmt::ForIn(_)
//                 | swc_ecma_ast::Stmt::ForOf(_) => {
//                     // Visit to extract the names
//                     let names = ast_name_tracker::visitor::find_names(file_logger, stmt);
//                     let imports_exports =
//                     Some(Segment {
//                         variable_scope: names,
//                         segment_type: SegmentKind::Normal(NormalSegment {
//                             imports: crate::NormalSegmentImportInfo {
//                                 lazy_imports: (),
//                                 requires: (),
//                             },
//                         }),
//                     })
//                 }
//                 swc_ecma_ast::Stmt::With(_) => {
//                     // with statements are deprecated and unsupported
//                     // See: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/with
//                     file_logger.src_error(
//                         &module_item.span(),
//                         StatementToSegmentError::WithStatmentUnsupported,
//                     );
//                     None
//                 }
//                 swc_ecma_ast::Stmt::Return(_) => {
//                     file_logger.src_error(
//                         &module_item.span(),
//                         StatementToSegmentError::StatementUnexpectedInModuleScope(RETURN),
//                     );
//                     None
//                 }
//                 swc_ecma_ast::Stmt::Break(_) => {
//                     file_logger.src_error(
//                         &module_item.span(),
//                         StatementToSegmentError::StatementUnexpectedInModuleScope(BREAK),
//                     );
//                     None
//                 }
//                 swc_ecma_ast::Stmt::Continue(_) => {
//                     file_logger.src_error(
//                         &module_item.span(),
//                         StatementToSegmentError::StatementUnexpectedInModuleScope(CONTINUE),
//                     );
//                     None
//                 }
//             }
//         }
//         swc_ecma_ast::ModuleItem::ModuleDecl(module_decl) => {
//             let names = ast_name_tracker::visitor::find_names(file_logger, stmt);
//             Ok(names)
//         }
//     }
// }

// // represents a 2d dependency map between a set of IDs
// struct Dependencies2D {
//     size: u32,
//     backing_bitmap: roaring::RoaringBitmap,
// }
// impl Dependencies2D {
//     pub fn new(size: u32) -> Self {
//         Self {
//             size,
//             backing_bitmap: roaring::RoaringBitmap::new(),
//         }
//     }

//     pub fn add_dependency(&mut self, from: u32, to: u32) {
//         self.backing_bitmap.contains(self.idx(from, to));
//     }

//     pub fn depends_on(&self, from: u32, to: u32) -> bool {
//         self.backing_bitmap.contains(self.idx(from, to))
//     }

//     fn idx(&self, from: u32, to: u32) -> u32 {
//         from * self.size + to
//     }
// }

// struct ModuleSegments {
//     // The raw segments from this module (source file)
//     segments: Vec<ModuleSegment>,
//     // The name-based dependencies between this module's segments
//     name_dependencies: Dependencies2D,
//     // The effect-based dependencies between this module's segments
//     effect_dependencies: Dependencies2D,
// }

// struct ModuleSegment {
//     // The raw source code of them segment
//     ast_node: swc_ecma_ast::ModuleItem,
//     // External names that this segment references, but is not defined within this module
//     // (e.g. runtime globals)
//     escaped_names: Vec<swc_atoms::Atom>,
// }

// #[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
// pub enum ExportedSymbol {
//     // A named export
//     Named(String),
//     // The default export
//     Default,
//     // A namespace export
//     Namespace,
//     ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
// }

// #[derive(Debug, Eq, PartialEq, Clone, Hash)]
// pub struct ReExportedSymbol {
//     /// The symbol being re-exported from another module
//     pub imported: ExportedSymbol,
//     /// If the symbol is renamed, this field contains the new name.
//     ///  (e.g. the export { _ as foo } from './foo' generates `renamed_to: Some("foo".to_string())`)
//     pub renamed_to: Option<ExportedSymbol>,
// }

// /// Represents the raw import/export information from a file, where import
// /// specifiers are not yet resolved to their final paths.
// #[derive(Debug, PartialEq, Eq, Clone)]
// pub struct RawImportExportInfo {
//     // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
//     pub imported_path_ids: AHashMap<String, AHashSet<ExportedSymbol>>,
//     // require('foo') generates ['foo']
//     pub require_paths: AHashSet<String>,
//     // import('./foo') generates ["./foo"]
//     pub imported_paths: AHashSet<String>,
//     // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
//     pub export_from_ids: AHashMap<String, AHashMap<ReExportedSymbol, ExportedSymbolMetadata>>,
//     // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
//     pub exported_ids: AHashMap<ExportedSymbol, ExportedSymbolMetadata>,
//     // `import './foo'`
//     pub executed_paths: AHashSet<String>,
// }

// /// Enum representing how an individual segment imports/exports symbols
// /// form another module
// struct ModuleImport {
//     /// The specifie for the module that is being imported
//     /// e.g. './helpers' or 'lodash-es'
//     module_specifier: String,
//     /// The names that are being imported from the module
//     ///
//     /// This is nonstandard for module-style imports like
//     /// import('foo') or require('foo'), and is only
//     /// supported by the import statemnt
//     extracted_names: Option<Vec<String>>,
// }

// fn segment_module(
//     file_logger: &impl SrcFileLogger,
//     module: &swc_ecma_ast::Module,
// ) -> ModuleSegments {
//     for module_item in module.body.iter() {
//         match module_item {
//             swc_ecma_ast::ModuleItem::Stmt(stmt) => {
//                 let segment = statement_to_segment(file_logger, stmt);
//             }
//             swc_ecma_ast::ModuleItem::ModuleDecl(decl) => {}
//         }
//     }
// }
