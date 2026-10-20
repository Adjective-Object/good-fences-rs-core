use crate::{
    raw_module_deps::{ExportBinding, RawModuleDeps, Symbol, SymbolTags, TaggedSymbol},
    ExportedSymbol, ReExportedSymbol,
};

use logger_srcfile::SrcFileLogger;
use oxc_ast::ast::{
    Argument, CallExpression, Declaration, ExportAllDeclaration, ExportDefaultDeclaration,
    ExportDefaultDeclarationKind, ExportNamedDeclaration, ExportSpecifier as OxcExportSpecifier,
    Expression, ImportDeclaration, ImportDeclarationSpecifier, ImportOrExportKind,
    ModuleExportName, TSImportEqualsDeclaration, TSModuleDeclarationName, TSModuleReference,
};
use oxc_ast::{Comment, ast::IdentifierReference};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Semantic;
use std::convert::TryInto;

/// AST visitor that gathers information on file imports and exports from an oxc AST.
pub struct ExportsVisitor<'v, 'a, TLogger: SrcFileLogger> {
    pub logger: &'v TLogger,
    pub comments: &'v [Comment],
    pub source: &'v str,
    pub semantic: &'v Semantic<'a>,
    pub module_deps: RawModuleDeps,
}

/// Extracts information from each specifier in an `export { ... } from '...'` declaration.
fn get_export_bindings<'a>(
    comments: &[Comment],
    source_text: &str,
    parent_tags: SymbolTags,
    export: &ExportNamedDeclaration<'a>,
) -> Vec<ExportBinding> {
    let mut parent_tags = parent_tags;
    parent_tags.is_type_only |= export.export_kind == ImportOrExportKind::Type;

    export
        .specifiers
        .iter()
        .map(|spec: &OxcExportSpecifier<'a>| -> ExportBinding {
            let specifier_tags = SymbolTags::from_comments_parent(
                parent_tags.clone(),
                comments,
                source_text,
                spec.span.start,
            );
            let imported_name = module_export_name_to_str(&spec.local);
            let exported_name = module_export_name_to_str(&spec.exported);
            let mut specifier_tags = specifier_tags;
            specifier_tags.is_type_only |= spec.export_kind == ImportOrExportKind::Type;
            let exported_as = if imported_name == exported_name {
                None
            } else {
                Some(ExportedSymbol::from_module_export_name(&spec.exported))
            };
            ExportBinding {
                original: TaggedSymbol::with_span(
                    Symbol::from(imported_name),
                    specifier_tags,
                    export.span,
                ),
                exported_as,
            }
        })
        .collect()
}

fn module_export_name_to_str<'a>(name: &ModuleExportName<'a>) -> &'a str {
    match name {
        ModuleExportName::IdentifierName(ident) => ident.name.as_str(),
        ModuleExportName::IdentifierReference(ident) => ident.name.as_str(),
        ModuleExportName::StringLiteral(s) => s.value.as_str(),
    }
}

fn is_global_require<'a>(semantic: &Semantic<'a>, ident: &IdentifierReference<'a>) -> bool {
    if ident.name != "require" {
        return false;
    }
    let Some(ref_id) = ident.reference_id.get() else {
        return false;
    };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}

impl<'v, 'a, TLogger: SrcFileLogger> ExportsVisitor<'v, 'a, TLogger> {
    pub fn new(
        logger: &'v TLogger,
        comments: &'v [Comment],
        source: &'v str,
        semantic: &'v Semantic<'a>,
    ) -> Self {
        Self {
            logger,
            comments,
            source,
            semantic,
            module_deps: Default::default(),
        }
    }

    pub fn has_disable_export_comment(&self, lo: u32) -> bool {
        has_disable_export_comment(self.comments, self.source, lo)
    }
}

pub fn has_disable_export_comment(comments: &[Comment], source: &str, lo: u32) -> bool {
    let leading = oxc_utils_parse::leading_comments_at(comments, source, lo);
    leading.iter().any(|c| {
        if c.kind != oxc_ast::CommentKind::Line {
            return false;
        }
        let text = &source[(c.span.start as usize + 2)..c.span.end as usize];
        text.trim().starts_with("@ALLOW-UNUSED-EXPORT")
    })
}

impl<'v, 'a, T: SrcFileLogger> From<ExportsVisitor<'v, 'a, T>> for RawModuleDeps {
    fn from(x: ExportsVisitor<'v, 'a, T>) -> Self {
        x.module_deps
    }
}

impl<'v, 'a, T: SrcFileLogger> Visit<'a> for ExportsVisitor<'v, 'a, T> {
    // Handles `export default foo`
    // Handles `export default class Foo {}` or `export default function foo() {}`
    fn visit_export_default_declaration(&mut self, decl: &ExportDefaultDeclaration<'a>) {
        walk::walk_export_default_declaration(self, decl);
        let mut tags = SymbolTags::from_comments(self.comments, self.source, decl.span.start);
        tags.is_type_only = matches!(
            &decl.declaration,
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
        );
        self.module_deps.exports_locals.insert(
            ExportedSymbol::Default,
            TaggedSymbol::with_span(Symbol::Default, tags, decl.span),
        );
    }

    // Handles `export const foo = 1`, `export class Foo {}`, `export function bar() {}`
    // Also handles `export { foo as bar } from 'source'` and `export { foo }`
    fn visit_export_named_declaration(&mut self, export: &ExportNamedDeclaration<'a>) {
        walk::walk_export_named_declaration(self, export);

        if let Some(source) = &export.source {
            // Re-export: `export { foo } from './foo'`
            let tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
            let source_str = source.value.to_string();
            get_export_bindings(self.comments, self.source, tags, export)
                .into_iter()
                .for_each(|binding| {
                    if let Ok(re_export) = TryInto::<ReExportedSymbol>::try_into(binding) {
                        self.module_deps
                            .exports_from
                            .entry(source_str.clone())
                            .or_default()
                            .insert(re_export);
                    }
                });
        } else if let Some(declaration) = &export.declaration {
            // Export declaration: `export const foo = 1`
            let mut tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
            let is_type = matches!(
                declaration,
                Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_)
            );
            tags.is_type_only = is_type;

            let idents = extract_declaration_names(self.logger, declaration);

            for ident in idents {
                self.module_deps.exports_locals.insert(
                    ExportedSymbol::from(ident.as_str()),
                    TaggedSymbol::with_span(
                        Symbol::from(ident.as_str()),
                        tags.clone(),
                        export.span,
                    ),
                );
            }
        } else {
            // Local re-export: `export { foo }` or `export { foo as bar }`
            let allow_unused = self.has_disable_export_comment(export.span.start);
            let is_type_only = export.export_kind == ImportOrExportKind::Type;
            for spec in &export.specifiers {
                let local_name = module_export_name_to_str(&spec.local).to_string();
                let exported_as = ExportedSymbol::from_module_export_name(&spec.exported);
                let exported_key = exported_as.clone();
                let mut tags = SymbolTags::from_comments(
                    self.comments,
                    self.source,
                    spec.span.start,
                );
                tags.allow_unused_comment |= allow_unused;
                tags.is_type_only |= is_type_only || spec.export_kind == ImportOrExportKind::Type;
                self.module_deps.exports_locals.insert(
                    exported_key,
                    TaggedSymbol::with_span(
                        Symbol::from(local_name.as_str()),
                        tags,
                        export.span,
                    ),
                );
            }
        }
    }

    // `export * from './foo'`
    fn visit_export_all_declaration(&mut self, export: &ExportAllDeclaration<'a>) {
        walk::walk_export_all_declaration(self, export);
        let source = export.source.value.to_string();
        let mut tags = SymbolTags::from_comments(self.comments, self.source, export.span.start);
        tags.is_type_only |= export.export_kind == ImportOrExportKind::Type;

        if let Some(exported) = &export.exported {
            // `export * as ns from 'mod'`
            let binding = ReExportedSymbol {
                imported_as: crate::ImportTarget::Namespace,
                exported_as: Some(ExportedSymbol::from_module_export_name(exported)),
                tags,
                span: export.span,
            };
            self.module_deps
                .exports_from
                .entry(source)
                .or_default()
                .insert(binding);
        } else {
            // `export * from 'mod'`
            let binding = ReExportedSymbol {
                imported_as: crate::ImportTarget::Namespace,
                exported_as: None,
                tags,
                span: export.span,
            };
            self.module_deps
                .exports_from
                .entry(source)
                .or_default()
                .insert(binding);
        }
    }

    // import foo = require('./foo')
    fn visit_ts_import_equals_declaration(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        walk::walk_ts_import_equals_declaration(self, decl);
        if let TSModuleReference::ExternalModuleReference(module_ref) = &decl.module_reference {
            self.module_deps
                .dynamic_imports
                .entry(module_ref.expression.value.to_string())
                .or_default()
                .insert(Symbol::Namespace);
        }
    }

    // import('foo')
    fn visit_import_expression(&mut self, import_expr: &oxc_ast::ast::ImportExpression<'a>) {
        walk::walk_import_expression(self, import_expr);
        if let Some(import_path) = extract_string_from_expression(&import_expr.source) {
            self.module_deps
                .dynamic_imports
                .entry(import_path)
                .or_default()
                .insert(Symbol::Namespace);
        }
    }

    // require('foo')
    fn visit_call_expression(&mut self, expr: &CallExpression<'a>) {
        walk::walk_call_expression(self, expr);
        if let Expression::Identifier(ident) = &expr.callee {
            if is_global_require(self.semantic, ident) {
                if let Some(import_path) = extract_first_string_argument(&expr.arguments) {
                    self.module_deps.requires.insert(import_path);
                }
            }
        }
    }

    // import foo from './foo';
    fn visit_import_declaration(&mut self, import: &ImportDeclaration<'a>) {
        walk::walk_import_declaration(self, import);

        let src = import.source.value.to_string();

        // import './foo';
        let specifiers = match &import.specifiers {
            Some(specs) if !specs.is_empty() => specs,
            _ => {
                self.module_deps.executed_paths.insert(src);
                return;
            }
        };

        // import .. from ..
        let mut parent_tags =
            SymbolTags::from_comments(self.comments, self.source, import.span.start);
        parent_tags.is_type_only |= import.import_kind == ImportOrExportKind::Type;

        let tagged_symbols: Vec<TaggedSymbol> = specifiers
            .iter()
            .map(|spec| -> TaggedSymbol {
                let mut tags = parent_tags.clone();
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(named) => {
                        tags.is_type_only |= named.import_kind == ImportOrExportKind::Type;
                        TaggedSymbol::new(
                            Symbol::from_module_export_name(&named.imported),
                            tags,
                        )
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                        TaggedSymbol::new(Symbol::Default, tags)
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
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

/// Extract a string value from an expression (e.g. `'foo'` in `import('foo')`).
fn extract_string_from_expression(expr: &Expression<'_>) -> Option<String> {
    if let Expression::StringLiteral(s) = expr {
        Some(s.value.to_string())
    } else {
        None
    }
}

/// Extract the first string argument from a call's argument list.
fn extract_first_string_argument(args: &oxc_allocator::Vec<'_, Argument<'_>>) -> Option<String> {
    let first = args.first()?;
    match first {
        Argument::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// Extract declared names from a `Declaration` node.
fn extract_declaration_names(_logger: &impl SrcFileLogger, decl: &Declaration<'_>) -> Vec<String> {
    match decl {
        Declaration::ClassDeclaration(class) => {
            class.id.as_ref().map_or(vec![], |id| vec![id.name.to_string()])
        }
        Declaration::FunctionDeclaration(func) => {
            func.id.as_ref().map_or(vec![], |id| vec![id.name.to_string()])
        }
        Declaration::VariableDeclaration(var_decl) => var_decl
            .declarations
            .iter()
            .flat_map(|d| collect_binding_names(&d.id))
            .collect(),
        Declaration::TSInterfaceDeclaration(decl) => {
            vec![decl.id.name.to_string()]
        }
        Declaration::TSTypeAliasDeclaration(decl) => {
            vec![decl.id.name.to_string()]
        }
        Declaration::TSEnumDeclaration(decl) => {
            vec![decl.id.name.to_string()]
        }
        Declaration::TSModuleDeclaration(decl) => match &decl.id {
            TSModuleDeclarationName::Identifier(ident) => vec![ident.name.to_string()],
            TSModuleDeclarationName::StringLiteral(s) => vec![s.value.to_string()],
        },
        _ => vec![],
    }
}

/// Recursively collect all binding identifier names from a `BindingPattern`.
pub(crate) fn collect_binding_names(pattern: &oxc_ast::ast::BindingPattern<'_>) -> Vec<String> {
    use oxc_ast::ast::BindingPattern;
    match pattern {
        BindingPattern::BindingIdentifier(ident) => vec![ident.name.to_string()],
        BindingPattern::ObjectPattern(obj) => obj
            .properties
            .iter()
            .flat_map(|prop| collect_binding_names(&prop.value))
            .chain(
                obj.rest
                    .as_ref()
                    .into_iter()
                    .flat_map(|rest| collect_binding_names(&rest.argument)),
            )
            .collect(),
        BindingPattern::ArrayPattern(arr) => arr
            .elements
            .iter()
            .flatten()
            .flat_map(|elem| collect_binding_names(elem))
            .chain(
                arr.rest
                    .as_ref()
                    .into_iter()
                    .flat_map(|rest| collect_binding_names(&rest.argument)),
            )
            .collect(),
        BindingPattern::AssignmentPattern(assign) => collect_binding_names(&assign.left),
    }
}
