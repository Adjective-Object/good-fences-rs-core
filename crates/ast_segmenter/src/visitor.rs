use logger_srcfile::SrcFileLogger;
use swc_common::Spanned;

use crate::{
    visitors::import_require_expr,
    NormalSegment, Segment, SegmentKind,
};

struct Visitor {
    segments: Vec<SegmentKind>,
}

const RETURN: &str = "return";
const BREAK: &str = "break";
const CONTINUE: &str = "continue";

#[derive(thiserror::Error, Debug)]
pub enum StatementToSegmentError {
    #[error("\"{}\" statements should not occur at the module level", .0)]
    StatementUnexpectedInModuleScope(&'static str),
    #[error("With statements are not supported because they create new non-lexical names")]
    WithStatmentUnsupported,
}

fn module_item_to_segment(
    file_logger: &impl SrcFileLogger,
    module_item: &swc_ecma_ast::ModuleItem,
) -> Option<Segment> {
    match module_item {
        swc_ecma_ast::ModuleItem::Stmt(stmt) => {
            match stmt {
                swc_ecma_ast::Stmt::Decl(_)
                | swc_ecma_ast::Stmt::Expr(_)
                | swc_ecma_ast::Stmt::Block(_)
                | swc_ecma_ast::Stmt::Empty(_)
                | swc_ecma_ast::Stmt::Debugger(_)
                | swc_ecma_ast::Stmt::Labeled(_)
                | swc_ecma_ast::Stmt::Switch(_)
                | swc_ecma_ast::Stmt::If(_)
                | swc_ecma_ast::Stmt::Throw(_)
                | swc_ecma_ast::Stmt::Try(_)
                | swc_ecma_ast::Stmt::While(_)
                | swc_ecma_ast::Stmt::DoWhile(_)
                | swc_ecma_ast::Stmt::For(_)
                | swc_ecma_ast::Stmt::ForIn(_)
                | swc_ecma_ast::Stmt::ForOf(_) => {
                    // Visit to extract the names
                    let names = ast_name_tracker::visitor::find_names(file_logger, stmt);
                    let imports_and_requires = import_require_expr::find_imports_and_requires(stmt);
                    let lazy_imports = imports_and_requires.imported_paths.names()
                        .into_iter()
                        .map(|(specifier, _symbols)| crate::ModuleImport {
                            module_specifier: specifier,
                            extracted_names: None,
                        })
                        .collect();
                    let requires = imports_and_requires.require_paths.names()
                        .into_iter()
                        .map(|(specifier, _symbols)| crate::ModuleImport {
                            module_specifier: specifier,
                            extracted_names: None,
                        })
                        .collect();
                    Some(Segment {
                        variable_scope: names,
                        segment_type: SegmentKind::Normal(NormalSegment {
                            imports: crate::NormalSegmentImportInfo {
                                lazy_imports,
                                requires,
                            },
                        }),
                    })
                }
                swc_ecma_ast::Stmt::With(_) => {
                    // with statements are deprecated and unsupported
                    // See: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Statements/with
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::WithStatmentUnsupported,
                    );
                    None
                }
                swc_ecma_ast::Stmt::Return(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(RETURN),
                    );
                    None
                }
                swc_ecma_ast::Stmt::Break(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(BREAK),
                    );
                    None
                }
                swc_ecma_ast::Stmt::Continue(_) => {
                    file_logger.src_error(
                        &module_item.span(),
                        StatementToSegmentError::StatementUnexpectedInModuleScope(CONTINUE),
                    );
                    None
                }
            }
        }
        swc_ecma_ast::ModuleItem::ModuleDecl(module_decl) => {
            let names = ast_name_tracker::visitor::find_names(file_logger, module_decl);
            // TODO: run ExportsVisitor on the declaration to populate module_deps
            Some(Segment {
                variable_scope: names,
                segment_type: SegmentKind::Normal(NormalSegment {
                    imports: crate::NormalSegmentImportInfo {
                        lazy_imports: vec![],
                        requires: vec![],
                    },
                }),
            })
        }
    }
}

// represents a 2d dependency map between a set of IDs
struct Dependencies2D {
    size: u32,
    backing_bitmap: roaring::RoaringBitmap,
}
impl Dependencies2D {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            backing_bitmap: roaring::RoaringBitmap::new(),
        }
    }

    pub fn add_dependency(&mut self, from: u32, to: u32) {
        self.backing_bitmap.insert(self.idx(from, to));
    }

    pub fn depends_on(&self, from: u32, to: u32) -> bool {
        self.backing_bitmap.contains(self.idx(from, to))
    }

    fn idx(&self, from: u32, to: u32) -> u32 {
        from * self.size + to
    }
}

struct ModuleSegments {
    // The raw segments from this module (source file)
    segments: Vec<ModuleSegment>,
    // The name-based dependencies between this module's segments
    name_dependencies: Dependencies2D,
    // The effect-based dependencies between this module's segments
    effect_dependencies: Dependencies2D,
}

struct ModuleSegment {
    // The raw source code of them segment
    ast_node: swc_ecma_ast::ModuleItem,
    // External names that this segment references, but is not defined within this module
    // (e.g. runtime globals)
    escaped_names: Vec<swc_atoms::Atom>,
}

/// Enum representing how an individual segment imports/exports symbols
/// form another module
struct ModuleImport {
    /// The specifie for the module that is being imported
    /// e.g. './helpers' or 'lodash-es'
    module_specifier: String,
    /// The names that are being imported from the module
    ///
    /// This is nonstandard for module-style imports like
    /// import('foo') or require('foo'), and is only
    /// supported by the import statemnt
    extracted_names: Option<Vec<String>>,
}

fn segment_module(
    file_logger: &impl SrcFileLogger,
    module: &swc_ecma_ast::Module,
) -> ModuleSegments {
    let mut segments = Vec::new();
    for module_item in module.body.iter() {
        if let Some(segment) = module_item_to_segment(file_logger, module_item) {
            segments.push(ModuleSegment {
                ast_node: module_item.clone(),
                escaped_names: vec![],
            });
        }
    }
    let len = segments.len() as u32;
    ModuleSegments {
        segments,
        name_dependencies: Dependencies2D::new(len),
        effect_dependencies: Dependencies2D::new(len),
    }
}
