#![feature(box_patterns)]
// // use ast_name_tracker::VariableScope;
// // use swc_ecma_ast::Module;

use ast_name_tracker::VariableScope;
use raw_module_deps::Name;
use swc_ecma_ast::ModuleExportName;

pub mod name_set;
pub mod raw_module_deps;
pub mod segment_graph;
pub mod segment_info;
pub mod visitor;
pub mod visitors;

pub use segment_info::RawSegment;
pub use visitor::segment_file;

// // Global identifier of a segment
// // (combination of file id and segment index within that file)
// pub struct SegmentId {
//     file_id: usize,
//     segment_id: usize,
// }

// A segment of a file
//
// At present this maps 1:1 onto a file's statements, but that
// assumption may change in the future
pub struct Segment {
    variable_scope: VariableScope,
    segment_type: SegmentKind,
}

// Parsed information about a segment, depending on how the segment
// has been classified
enum SegmentKind {
    /// This segment is a single import/export statement
    ///
    /// Note: to keep this cleanly disjoint from NormalSegment,
    /// `export const ...` should be expanded into separate
    /// virtual segments for the declaration and the export.
    ImportExportStatment(StaticImportType),
    /// This segment is the declaration of a single lazy module
    LazyModuleDecl(LazyModule),
    /// This segment is the declaration of a LazyComponent or LazyFunction,
    /// extracting a module from a lazy module
    LazyModuleReference(LazyModuleReference),
    /// "normal" code is all code which is not a specially recognized segment
    /// type
    ///
    /// e.g. function declarztions, side-effects, the initializing
    /// expressions of variables, etc.
    Normal(NormalSegment),
}

enum ImportKind {
    ImportStatement,
    ImportExpr,
    RequireExpr,
}

// A segment that contains "normal" code
// e.g. any non-specially recognized code
#[derive(Clone)]
struct NormalSegment {
    imports: NormalSegmentImportInfo,
}

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

/// Represents a local symbol that is being exported to another file
pub struct ExportLocal {
    local_name: Name,
    exported_as: Option<ExportedSymbol>,
}

/// Represents a local symbol that is being imported from another file
pub struct ImportedLocal {
    local_name: Name,
    imported_as: ImportTarget,
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

/// This represents the imports and exports of a segment from "normal" code
/// e.g. function declarztions, side-effects, the initializing
/// expressions of variables, etc.
///
/// Because all static imports are covered by the other variants of the
/// SegmentType enum, this should only ever contain dynamic dynamic imports
/// to other files
#[derive(Clone)]
struct NormalSegmentImportInfo {
    /// Import expressions within the segment
    lazy_imports: Vec<ModuleImport>,

    /// Require expressions referenced anywhere within the segment
    requires: Vec<ModuleImport>,
}

struct LazyModule {
    /// The local name of the lazy module
    local_name: Name,
    /// The import specifier for the module
    import: String,
}

struct LazyModuleReference {
    /// The local name of the lazy module
    local_name: Name,
    /// The member of the lazy module that is being extracted
    lazy_module_member: Name,
}

/// Enum representing how an individual segment imports/exports symbols
/// form another module
enum StaticImportType {
    // If this is an exporting segment, this is the list of names
    // that the segment exports
    ExportSymbols(Vec<ExportLocal>),

    // If this is an importing segment, this is the list of names
    // that the segment imports
    ImportSymbols(String, Vec<ImportedLocal>),

    // If this is an export from statement, this is the list of names
    // that the segment exports from another file
    ReExportSymbols(String, ReExportedSymbol),

    // If this is an import statment, this is the path of the file that
    // is imported
    EffectOnly(String),
}

/// Enum representing how an individual segment imports/exports symbols
/// form another module
#[derive(Clone)]
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

// /// A module, after segmentation.
// ///
// /// The inter-module references will need to be resolved, and their
// /// corresponding segments will need to be linked together during
// /// repo graph construction.
// pub struct SegmentedModule {
//     segments: Vec<Segment>,
//     // mapping of exported names into the segment that they
//     // are exported from
//     exported_symbols: HashMap<String, usize>,
// }

// fn segment_file(module: Module) -> SegmentedFile {
//     // TODO run visitor on file
//     SegmentedFile { segments: vec![] }
// }

// parse files -> SegmentedFiles
// resolve inter-module imports/exports from the SegmentedFiles
// expand namespace imports to determine exposed segment names within files
// resolve names to inter-segment references
// convert to an IR (intermediate representation_ of dependencies using well-known segment types
// normalize IR graph using simplification rules
//

// build graph that separates name references from effect dependencies
// (order of symbol declaration within a file matters, as all files within
// a module will have effect dependencies on all their preceeding statements
// in the file.
//
// Note: function declarations and imports should get hoisted to the front of
// the segments file. See:
//
// https://developer.mozilla.org/en-US/docs/Glossary/Hoisting
//
// I don't think implementing specific handling for `var` hoisting is smart, and
// we should start by assuming all code is written with let/const/class
// declarations instead. If we need to support var hoisting, we can add that
// at a later date, but it would require re-working the segment visitor to
// track the type definition of a variable.

// e.g. Segments references:
// Segment A
//   type: LazyModule{module_name: "some-module" (in its resolved repr, so probably a module ID)}
//   declares: lazyModule
//
// Segment B
//   type: LazyModuleReference("name")
//   declares: MyLazyComponent
//   references: lazyModule (ref: seg A)
//
// Segment C
//   type: Module(exposes: MyComponent(ref: segD), MyOtherComponent(ref: seg ...))
//   declares: MyComponent, MyOtherComponent
//
// Segment D
//   type: Normal
//   declares: MyComponent
//
// will map onto this kind of dependency graph:
//      B -> A -async-> C -> D
//
// During simplification, we have some rule where if we see this pattern:
//      X (LazyModuleReference "MyComponent") -> Y (LazyModule) -async-> Z (Module) -"MyComponent"> W (anything)
//   we should simplify into this kind of name reference graph:
//      X -lazyModuleImport-> W
//   and this kind of effect dependency graph:
//      X -> Z
//
//   Where 'lazyModuleImport' is a custom import type that we make our bundler aware of,
//   that follows the same semantics as an async import.
//
// Applying this to the above example, we get
//     B -lazyModuleImport-> D
//   with effects
//     B -async-> C -> D
//             -> ... other members of module C
//
// From this simplification rule, we get automatic splitting of lazy indexes:
//
// e.g.
//     B -> A -async-> C -> D
//     G -> A -async-> C -> E
//
//  which will be reduced during graph simplification to:
//
//     B -lazyModule(effects: C)-> D
//     G -lazyModule(effects: C)-> E
//
//  which are disjoint graphs, and will be re-materialoised as separate LazyModules during code generation
//
// Side note: "effect" dependencies should not be stored as a separate graph -- instead, it and the symbol graph should
// be derived from the set of well-known dependencies between segments!
//
// Note that this will be sensitive to the effect graph! See how above in the effect graph, we retain the dependency on module C's execution.
// Therefore, this only remains true so long as C does not have significant side-effects. This approach therefore relies on a robust heuristic
// for side-effects. This will require defining the target javascript standard library, annotated with if each of the functions are effectful
// or not during compilation.
//
//
//
// Looking at bundling, both effect and import dependencies are relevant for determining when a module can be stripped from the graph or not.
// So, take the following:
//
// lazyIndex.ts:
//  import "./mySideEffectModule";
//  export { MyComponent } from "./components/MyComponent";
//  export { MyOtherComponent } from "./components/MyOtherComponent";
//
// index.ts:
//  const lazyModule = new LazyModule(() => import("./lazyIndex"));
//  const MyLazyComponent = new LazyComponent(lazyModule);
//  const MyOtherLazyComponent = new LazyComponent(lazyModule);
//
//   This generates the following graph:
//     MyLazyComponent      -> lazyModule -async-> lazyModule_module -> MyComponent
//     MyOtherLazyComponent -> lazyModule -async-> lazyModule_module -> MyOtherComponent
//
//   Which simplifies to:
//     MyLazyComponent      -lazyModule(effects: lazyModule_module) -> MyComponent
//     MyOtherLazyComponent -lazyModule(effects: lazyModule_module) -> MyOtherComponent
//
//   Our dependency graph looks like this:
//
//    MyLazyComponent      MyOtherLazyComponent
//     |      ⋮              ⋮      |
//     |     lazyModule_module      |
//     V           ⋮                V
//    MyComponent  ⋮   MyOtherComponent
//         ⋮       ⋮          ⋮
//         V       V         V
//          mySideEffectModule
//
//      (where ⋮ = effect dependency, and | = symbol dependency)
//
//   Before we bundle, we convert our graph of symbol and effect dependencies into a graph of bundling dependencies
//   by removing all effect dependencies onto symbols that _do not contain a transitive side-effect_
//
//   This gives us the following bundling graph:
//    MyLazyComponent      MyOtherLazyComponent
//     |      |              |      |
//     |     lazyModule_module      |
//     V           |                V
//    MyComponent  |   MyOtherComponent
//         |       |         |
//         V       V         V
//          mySideEffectModule
//
//   (Which is likely to be re-concatenated anyway during bundle hoisting/merging)
//
//   However, if we eliminate the side-effect, we get the following clean disjoint graphs:
//   MyLazyComponent      MyOtherLazyComponent
//     |                            |
//     |                            |
//     V                            V
//    MyComponent       MyOtherComponent
//
// As of now I am unclear if it makes sense to eagerly resolve this during segment side-effect analysis, or if it makes sense to include
// references to ambient global symbols during segment analysis. My hunch is that eager resolution makes more sense for performance, but
// may cause issues in a Bazel world.
//
// This approach of graph simplification should allow us to also remove trivial re-exports:
//
// e.g. Segments references:
// Segment A
//   type: ExportsSymbols{ symbols: ["_a" renamed As "A", "_b" renamed As "B"] })
//
// Segment B
//   type: ImportsSymbols{ from: "some-module", symbols: ["A", "B"] }
//
// Segment D
//   type: Normal
//   declares: MyComponent
