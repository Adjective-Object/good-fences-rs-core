// // use ast_name_tracker::VariableScope;
// // use swc_ecma_ast::Module;

// pub mod visitor;

// // // Global identifier of a segment
// // // (combination of file id and segment index within that file)
// // pub struct SegmentId {
// //     file_id: usize,
// //     segment_id: usize,
// // }

// // // A segment of a file
// // //
// // // At present this maps 1:1 onto a file's statements, but that
// // // assumption may change in the future
// // pub struct Segment {
// //     variable_scope: VariableScope,
// //     segment_type: ParsedSegment,
// // }

// // Parsed information about a segment, depending on how the segment
// // has been classified
// enum SegmentKind {
//     /// This segment is a single import/export statement
//     ///
//     /// Note: to keep this cleanly disjoint from NormalSegment,
//     /// `export const ...` should be expanded into separate
//     /// virtual segments for the declaration and the export.
//     ImportExportStatment(StaticImportType),
//     /// This segment is the declaration of a single lazy module
//     LazyModuleDecl(LazyModule),
//     /// This segment is the declaration of a LazyComponent or LazyFunction,
//     /// extracting a module from a lazy module
//     LazyModuleReference(LazyModuleReference),
//     /// "normal" code is all code which is not a specially recognized segment
//     /// type
//     ///
//     /// e.g. function declarztions, side-effects, the initializing
//     /// expressions of variables, etc.
//     NormalSegment(NormalSegment),
// }

// enum ImportKind {
//     ImportStatement,
//     ImportExpr,
//     RequireExpr,
// }

// // A segment that contains "normal" code
// // e.g. any non-specially recognized code
// #[derive(Clone)]
// struct NormalSegment {
//     has_own_effects: bool,
//     imports: NormalSegmentImportInfo,
// }

// // The target of an export, either a symbol or a namespace
// #[derive(Clone)]
// pub enum ExportTarget {
//     // An individual symbol that is exported
//     Symbol(ExportedSymbol),
//     // A namespace export
//     Namespace,
//     // an effect-only import of another file
//     // in case of `import './foo';` this executes code in file but imports no symbols
//     EffectOnly,
// }

// // A named or default symbol that is exported from or imported into a file
// #[derive(Clone)]
// pub enum ExportedSymbol {
//     // The name of the symbol in the exporting file
//     Named(String),
//     // The default export
//     Default,
// }

// /// Represents a local symbol that is being exported to another file
// pub struct ExportLocal {
//     local_name: String,
//     exported_as: ExportTarget,
// }

// /// Represents a local symbol that is being imported from another file
// pub struct ImportedLocal {
//     local_name: String,
//     imported_as: ExportTarget,
// }

// pub struct ReExportedSymbol {
//     imported_as: ExportedSymbol,
//     exported_as: ExportedSymbol,
// }

// /// This represents the imports and exports of a segment from "normal" code
// /// e.g. function declarztions, side-effects, the initializing
// /// expressions of variables, etc.
// ///
// /// Because all static imports are covered by the other variants of the
// /// SegmentType enum, this should only ever contain dynamic dynamic imports
// /// to other files
// #[derive(Clone)]
// struct NormalSegmentImportInfo {
//     /// Import expressions within the segment
//     lazy_imports: Vec<ModuleImport>,

//     /// Require expressions referenced anywhere within the segment
//     requires: Vec<ModuleImport>,
// }

// struct LazyModule {
//     /// The local name of the lazy module
//     local_name: String,
//     /// The import specifier for the module
//     import: String,
// }

// struct LazyModuleReference {
//     /// The local name of the lazy module
//     local_name: String,
//     /// The member of the lazy module that is being extracted
//     lazy_module_member: String,
// }

// /// Enum representing how an individual segment imports/exports symbols
// /// form another module
// enum StaticImportType {
//     // If this is an exporting segment, this is the list of names
//     // that the segment exports
//     ExportSymbols(Vec<ExportLocal>),

//     // If this is an importing segment, this is the list of names
//     // that the segment imports
//     ImportSymbols(String, Vec<ImportedLocal>),

//     // If this is an export from statement, this is the list of names
//     // that the segment exports from another file
//     ReExportSymbols(String, ReExportedSymbol),

//     // If this is an import statment, this is the path of the file that
//     // is imported
//     EffectOnly(String),
// }

// /// Enum representing how an individual segment imports/exports symbols
// /// form another module
// #[derive(Clone)]
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

// // /// A module, after segmentation.
// // ///
// // /// The inter-module references will need to be resolved, and their
// // /// corresponding segments will need to be linked together during
// // /// repo graph construction.
// // pub struct SegmentedModule {
// //     segments: Vec<Segment>,
// //     // mapping of exported names into the segment that they
// //     // are exported from
// //     exported_symbols: HashMap<String, usize>,
// // }

// // fn segment_file(module: Module) -> SegmentedFile {
// //     // TODO run visitor on file
// //     SegmentedFile { segments: vec![] }
// // }

// // parse files -> SegmentedFiles
// // resolve inter-module imports/exports from the SegmentedFiles
// // expand namespace imports to determine exposed segment names within files
// // resolve names to inter-segment references
// // convert to an IR (intermediate representation_ of dependencies using well-known segment types
// // normalize IR graph using simplification rules
// //

// // build graph that separates name references from effect dependencies
// // (order of symbol declaration within a file matters, as all files within
// // a module will have effect dependencies on all their preceeding statements
// // in the file.
// //
// // Note: function declarations and imports should get hoisted to the front of
// // the segments file. See:
// //
// // https://developer.mozilla.org/en-US/docs/Glossary/Hoisting
// //
// // I don't think implementing specific handling for `var` hoisting is smart, and
// // we should start by assuming all code is written with let/const/class
// // declarations instead. If we need to support var hoisting, we can add that
// // at a later date, but it would require re-working the segment visitor to
// // track the type definition of a variable.

// // e.g. Segments references:
// // Segment A
// //   type: LazyModule{module_name: "some-module" (in its resolved repr, so probably a module ID)}
// //   declares: lazyModule
// //
// // Segment B
// //   type: LazyModuleReference("name")
// //   declares: MyLazyComponent
// //   references: lazyModule (ref: seg A)
// //
// // Segment C
// //   type: Module(exposes: MyComponent(ref: segD), MyOtherComponent(ref: seg ...))
// //   declares: MyComponent, MyOtherComponent
// //
// // Segment D
// //   type: Normal
// //   declares: MyComponent
// //
// // will map onto this kind of dependency graph:
// //      B -> A -async-> C -> D
// //
// // During simplification, we have some rule where if we see this pattern:
// //      X (LazyModuleReference "MyComponent") -> Y (LazyModule) -async-> Z (Module) -"MyComponent"> W (anything)
// //   we should simplify into this kind of name reference graph:
// //      X -lazyModuleImport-> W
// //   and this kind of effect dependency graph:
// //      X -> Z
// //
// //   Where 'lazyModuleImport' is a custom import type that we make our bundler aware of,
// //   that follows the same semantics as an async import.
// //
// // Applying this to the above example, we get
// //     B -lazyModuleImport-> D
// //   with effects
// //     B -async-> C -> D
// //             -> ... other members of module C
// //
// // From this simplification rule, we get automatic splitting of lazy indexes:
// //
// // e.g.
// //     B -> A -async-> C -> D
// //     G -> A -async-> C -> E
// //
// //  which will be reduced during graph simplification to:
// //
// //     B -lazyModule(effects: C)-> D
// //     G -lazyModule(effects: C)-> E
// //
// //  which are disjoint graphs, and will be re-materialoised as separate LazyModules during code generation
// //
// // Side note: "effect" dependencies should not be stored as a separate graph -- instead, it and the symbol graph should
// // be derived from the set of well-known dependencies between segments!
// //
// // Note that this will be sensitive to the effect graph! See how above in the effect graph, we retain the dependency on module C's execution.
// // Therefore, this only remains true so long as C does not have significant side-effects. This approach therefore relies on a robust heuristic
// // for side-effects. This will require defining the target javascript standard library, annotated with if each of the functions are effectful
// // or not during compilation.
// //
// //
// //
// // Looking at bundling, both effect and import dependencies are relevant for determining when a module can be stripped from the graph or not.
// // So, take the following:
// //
// // lazyIndex.ts:
// //  import "./mySideEffectModule";
// //  export { MyComponent } from "./components/MyComponent";
// //  export { MyOtherComponent } from "./components/MyOtherComponent";
// //
// // index.ts:
// //  const lazyModule = new LazyModule(() => import("./lazyIndex"));
// //  const MyLazyComponent = new LazyComponent(lazyModule);
// //  const MyOtherLazyComponent = new LazyComponent(lazyModule);
// //
// //   This generates the following graph:
// //     MyLazyComponent      -> lazyModule -async-> lazyModule_module -> MyComponent
// //     MyOtherLazyComponent -> lazyModule -async-> lazyModule_module -> MyOtherComponent
// //
// //   Which simplifies to:
// //     MyLazyComponent      -lazyModule(effects: lazyModule_module) -> MyComponent
// //     MyOtherLazyComponent -lazyModule(effects: lazyModule_module) -> MyOtherComponent
// //
// //   Our dependency graph looks like this:
// //
// //    MyLazyComponent      MyOtherLazyComponent
// //     |      ⋮              ⋮      |
// //     |     lazyModule_module      |
// //     V           ⋮                V
// //    MyComponent  ⋮   MyOtherComponent
// //         ⋮       ⋮          ⋮
// //         V       V         V
// //          mySideEffectModule
// //
// //      (where ⋮ = effect dependency, and | = symbol dependency)
// //
// //   Before we bundle, we convert our graph of symbol and effect dependencies into a graph of bundling dependencies
// //   by removing all effect dependencies onto symbols that _do not contain a transitive side-effect_
// //
// //   This gives us the following bundling graph:
// //    MyLazyComponent      MyOtherLazyComponent
// //     |      |              |      |
// //     |     lazyModule_module      |
// //     V           |                V
// //    MyComponent  |   MyOtherComponent
// //         |       |         |
// //         V       V         V
// //          mySideEffectModule
// //
// //   (Which is likely to be re-concatenated anyway during bundle hoisting/merging)
// //
// //   However, if we eliminate the side-effect, we get the following clean disjoint graphs:
// //   MyLazyComponent      MyOtherLazyComponent
// //     |                            |
// //     |                            |
// //     V                            V
// //    MyComponent       MyOtherComponent
// //
// // As of now I am unclear if it makes sense to eagerly resolve this during segment side-effect analysis, or if it makes sense to include
// // references to ambient global symbols during segment analysis. My hunch is that eager resolution makes more sense for performance, but
// // may cause issues in a Bazel world.
// //
// // This approach of graph simplification should allow us to also remove trivial re-exports:
// //
// // e.g. Segments references:
// // Segment A
// //   type: ExportsSymbols{ symbols: ["_a" renamed As "A", "_b" renamed As "B"] })
// //
// // Segment B
// //   type: ImportsSymbols{ from: "some-module", symbols: ["A", "B"] }
// //
// // Segment D
// //   type: Normal
// //   declares: MyComponent

// pub fn segment_module(module: Module) {
//     // parse each of the statements in the module
//     for stmt in module.body {}
// }
