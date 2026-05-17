#![feature(box_patterns)]

use raw_module_deps::Name;
use swc_ecma_ast::ModuleExportName;

pub mod name_set;
pub mod raw_module_deps;
pub mod segment_graph;
pub mod segment_info;
pub mod visitor;
pub mod visitors;

pub use segment_info::Segment;
pub use visitor::segment_file;

// The target of an import, either a symbol or a namespace
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ImportTarget {
    // An individual symbol that is exported
    ExportedSymbol(ExportedSymbol),
    // A namespace export
    Namespace,
}

// A named or default symbol that is exported from or imported into a file
// This enclosed in <> in the below examples::
// export foo as <bar>
// export * as <Foo> from './foo'
// export type { foo as <default> } from './foo'
//
// Note: this is currently a different type than `Symbol`, because `Symbol`
// is the union of
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub enum ExportedSymbol {
    // The name of the symbol in the exporting file
    Named(Name),
    // The default export
    Default,
}
impl<T: ToString + AsRef<str>> From<T> for ExportedSymbol {
    fn from(name: T) -> Self {
        if name.as_ref() == "default" {
            ExportedSymbol::Default
        } else {
            ExportedSymbol::Named(name.to_string().into())
        }
    }
}
impl ExportedSymbol {
    pub fn from_module_export_name(name: &ModuleExportName) -> Self {
        match name {
            ModuleExportName::Ident(ident) => Self::from(ident.sym.as_ref()),
            ModuleExportName::Str(str) => Self::from(str.value.as_ref()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReExportedSymbol {
    pub imported_as: ImportTarget,
    pub exported_as: Option<ExportedSymbol>,
    pub tags: raw_module_deps::SymbolTags,
    pub span: swc_common::Span,
}

/// Identity is determined by imported_as + exported_as only;
/// tags and span are metadata.
impl PartialEq for ReExportedSymbol {
    fn eq(&self, other: &Self) -> bool {
        self.imported_as == other.imported_as && self.exported_as == other.exported_as
    }
}
impl Eq for ReExportedSymbol {}
impl std::hash::Hash for ReExportedSymbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.imported_as.hash(state);
        self.exported_as.hash(state);
    }
}

