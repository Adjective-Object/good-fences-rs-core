use std::convert::TryInto;

use ahashmap::{AHashMap, AHashSet};
use oxc_ast::ast::{Comment, CommentKind, ModuleExportName};
use oxc_span::Span;
use oxc_utils_parse::leading_comments_at;
use std::fmt::Debug;

use crate::{ExportedSymbol, ImportTarget, ReExportedSymbol};

/// Metadata associated with a symbol in a module.
#[derive(Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub struct SymbolTags {
    // If the symbol has been tagged by a comment as "allowed to be unused"
    pub allow_unused_comment: bool,
    // If this symbol is type-only (e.g. declared as a type or interface, or imported as a type)
    pub is_type_only: bool,
}
impl SymbolTags {
    const IGNORE_UNUSED_PREFIX: &str = "@ALLOW-UNUSED-EXPORT";

    /// Build `SymbolTags` from the leading comments before `lo` in the source.
    ///
    /// Only line (`//`) comments are checked for `@ALLOW-UNUSED-EXPORT`.
    pub fn from_comments_parent(
        parent: Self,
        comments: &[Comment],
        source: &str,
        lo: u32,
    ) -> SymbolTags {
        let mut tags = parent;
        let leading = leading_comments_at(comments, source, lo);
        for c in leading.iter() {
            if c.kind != CommentKind::Line {
                continue;
            }
            let cs = c.content_span();
            let text = &source[cs.start as usize..cs.end as usize];
            let line = text.trim();
            if line.len() < Self::IGNORE_UNUSED_PREFIX.len() {
                continue;
            }
            let prefix = &line[..Self::IGNORE_UNUSED_PREFIX.len()];
            if prefix.eq_ignore_ascii_case(Self::IGNORE_UNUSED_PREFIX) {
                tags.allow_unused_comment = true;
            }
        }
        tags
    }

    pub fn from_comments(comments: &[Comment], source: &str, lo: u32) -> SymbolTags {
        SymbolTags::from_comments_parent(SymbolTags::default(), comments, source, lo)
    }
}

/// The interior representation of the name of a symbol.
///
/// for now this is a simple wrapper around a string, but we are encapsulating it,
/// as it should probably be changed to an interned string in the future
#[derive(PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub struct Name {
    inner: String,
}
impl Name {
    pub fn new(s: impl ToString) -> Name {
        Name {
            inner: s.to_string(),
        }
    }
}
impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}
impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.inner)
    }
}
impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}
impl From<String> for Name {
    fn from(s: String) -> Self {
        Name { inner: s }
    }
}
impl From<&str> for Name {
    fn from(s: &str) -> Self {
        Name {
            inner: s.to_string(),
        }
    }
}

/// Represents a symbol either at a module boundary (exported/imported) or within a module.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub enum Symbol {
    // A named export/import
    Named(Name),
    // The default export/import
    Default,
    // A namespace import/export
    Namespace,
}
impl<T: ToString + AsRef<str>> From<T> for Symbol {
    fn from(name: T) -> Self {
        if name.as_ref() == "default" {
            Symbol::Default
        } else {
            Symbol::Named(name.to_string().into())
        }
    }
}
impl Symbol {
    pub fn from_module_export_name(name: &ModuleExportName) -> Self {
        match name {
            ModuleExportName::IdentifierName(ident) => Self::from(ident.name.as_str()),
            ModuleExportName::IdentifierReference(ident) => Self::from(ident.name.as_str()),
            ModuleExportName::StringLiteral(str) => Self::from(str.value.as_str()),
        }
    }
}

impl Symbol {
    pub fn as_str(&self) -> &str {
        match self {
            Symbol::Named(s) => s.as_ref(),
            Symbol::Default => "default",
            Symbol::Namespace => "*",
        }
    }

    pub fn named(s: impl ToString) -> Symbol {
        Symbol::Named(Name::new(s))
    }
}

/// A symbol with associated tags (extracted from comments)
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
pub struct TaggedSymbol {
    pub symbol: Symbol,
    pub tags: SymbolTags,
    pub span: Span,
}
impl TaggedSymbol {
    pub fn new(symbol: Symbol, tags: SymbolTags) -> TaggedSymbol {
        TaggedSymbol {
            symbol,
            tags,
            span: Span::default(),
        }
    }
    pub fn with_span(symbol: Symbol, tags: SymbolTags, span: Span) -> TaggedSymbol {
        TaggedSymbol { symbol, tags, span }
    }
}

/// This represents both:
/// a local symbol that is exported from a module, and
/// a symbol that is re-exported from another module
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub struct ExportBinding {
    /// Either:
    /// - The symbol being re-exported from another module
    /// - The local symbol being exported from this module
    pub original: TaggedSymbol,
    /// If the symbol is renamed, this field contains the new name.
    ///  (e.g. the export { _ as foo } from './foo' generates `renamed_to: Some("foo".to_string())`)
    pub exported_as: Option<ExportedSymbol>,
}

#[derive(thiserror::Error, Debug)]
pub enum ExportBindingCoerceError {
    #[error("")]
    InvalidLocal,
}

impl TryInto<ReExportedSymbol> for ExportBinding {
    type Error = ExportBindingCoerceError;

    fn try_into(self) -> Result<ReExportedSymbol, Self::Error> {
        let tags = self.original.tags;
        let span = self.original.span;
        match self.original.symbol {
            Symbol::Named(name) => Ok(ReExportedSymbol {
                imported_as: ImportTarget::ExportedSymbol(ExportedSymbol::Named(name)),
                exported_as: self.exported_as,
                tags,
                span,
            }),
            Symbol::Default => Ok(ReExportedSymbol {
                imported_as: ImportTarget::ExportedSymbol(ExportedSymbol::Default),
                exported_as: self.exported_as,
                tags,
                span,
            }),
            Symbol::Namespace => Ok(ReExportedSymbol {
                imported_as: ImportTarget::Namespace,
                exported_as: self.exported_as,
                tags,
                span,
            }),
        }
    }
}

/// Type alias for the key in `exports_locals`.
/// An exported local is identified by the symbol it's exported as (Named or Default).
pub type ExportedLocal = ExportedSymbol;

/// Represents the unresolved import/export information from a file, extracted
/// from traversing the AST.
/// where import specifiers are not yet resolved to their final paths.
#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct RawModuleDeps {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imports: AHashMap<String, AHashSet<TaggedSymbol>>,
    // import('./foo') generates ["./foo"]
    //
    // We support extracting a specific set of named imports from
    // dynamic imports, when we see syntax of form:
    //
    // import('foo').then(({ bar, baz }) => { .. })
    pub dynamic_imports: AHashMap<String, AHashSet<Symbol>>,
    // require('foo') generates ['foo']
    //
    // Unlike import(), require() always returns the default export
    // of the module, so we don't track named imports from require().
    pub requires: AHashSet<String>,
    // `export {default as foo, bar} from './foo'`
    // map is 'imported_module' -> 're_exports'
    pub exports_from: AHashMap<String, AHashSet<ReExportedSymbol>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exports_locals: AHashMap<ExportedLocal, TaggedSymbol>,
    // `import './foo'`
    pub executed_paths: AHashSet<String>,
}
