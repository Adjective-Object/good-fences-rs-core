use super::*;

/// The byte offset of a binding's first declaration, not an OXC symbol ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindingId(pub u32);

/// The byte offset of a dependency expression/declaration. Repeated paths stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DependencyId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    id: BindingId,
    name: String,
    #[serde(with = "span_serde::list")]
    declarations: Vec<Span>,
    top_level: bool,
    function_scoped: bool,
    function: bool,
    import: bool,
    constant: bool,
    type_only: bool,
    ambient: bool,
}

impl Binding {
    pub(crate) fn new(
        id: BindingId,
        name: String,
        declarations: Vec<Span>,
        top_level: bool,
        function_scoped: bool,
        function: bool,
        import: bool,
        constant: bool,
        type_only: bool,
        ambient: bool,
    ) -> Self {
        Self {
            id,
            name,
            declarations,
            top_level,
            function_scoped,
            function,
            import,
            constant,
            type_only,
            ambient,
        }
    }
}

impl Binding {
    pub fn id(&self) -> BindingId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn declarations(&self) -> &[Span] {
        &self.declarations
    }

    pub fn top_level(&self) -> bool {
        self.top_level
    }

    pub fn function_scoped(&self) -> bool {
        self.function_scoped
    }

    pub fn function(&self) -> bool {
        self.function
    }

    pub fn import(&self) -> bool {
        self.import
    }

    pub fn constant(&self) -> bool {
        self.constant
    }

    pub fn type_only(&self) -> bool {
        self.type_only
    }

    pub fn ambient(&self) -> bool {
        self.ambient
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    name: String,
    #[serde(with = "span_serde")]
    span: Span,
    binding: Option<BindingId>,
    read: bool,
    write: bool,
    type_only: bool,
    async_deferred: bool,
    /// Wrapper parameter in CommonJS; never set for a lexical shadow.
    commonjs: Option<CommonJsGlobal>,
}

impl Reference {
    pub(crate) fn new(
        name: String,
        span: Span,
        binding: Option<BindingId>,
        read: bool,
        write: bool,
        type_only: bool,
        async_deferred: bool,
        commonjs: Option<CommonJsGlobal>,
    ) -> Self {
        Self {
            name,
            span,
            binding,
            read,
            write,
            type_only,
            commonjs,
            async_deferred,
        }
    }
}

impl Reference {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn binding(&self) -> Option<BindingId> {
        self.binding
    }

    pub fn read(&self) -> bool {
        self.read
    }

    pub fn write(&self) -> bool {
        self.write
    }

    pub fn type_only(&self) -> bool {
        self.type_only
    }

    pub fn commonjs(&self) -> Option<CommonJsGlobal> {
        self.commonjs
    }
    pub fn async_deferred(&self) -> bool {
        self.async_deferred
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommonJsGlobal {
    Require,
    Module,
    Exports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyKind {
    Import,
    ReExport,
    DynamicImport,
    Require,
    ImportType,
    /// Literal relative resource URL; never a module-evaluation dependency.
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    id: DependencyId,
    kind: DependencyKind,
    #[serde(with = "span_serde")]
    span: Span,
    specifier: String,
    /// Literal pieces separated by runtime gaps, never resolved paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pattern: Option<Vec<String>>,
    #[serde(with = "span_serde")]
    specifier_span: Span,
    type_only: bool,
    /// Lexical containment only, not a claim that a module-level site runs eagerly.
    #[serde(with = "span_serde::optional")]
    enclosing_function: Option<Span>,
}

impl Dependency {
    pub(crate) fn new(
        id: DependencyId,
        kind: DependencyKind,
        span: Span,
        specifier: String,
        pattern: Option<Vec<String>>,
        specifier_span: Span,
        type_only: bool,
        enclosing_function: Option<Span>,
    ) -> Self {
        Self {
            id,
            kind,
            span,
            specifier,
            pattern,
            specifier_span,
            type_only,
            enclosing_function,
        }
    }
}

impl Dependency {
    pub fn id(&self) -> DependencyId {
        self.id
    }

    pub fn kind(&self) -> DependencyKind {
        self.kind
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn specifier(&self) -> &str {
        &self.specifier
    }

    pub fn pattern(&self) -> Option<&[String]> {
        self.pattern.as_deref()
    }

    pub fn specifier_span(&self) -> Span {
        self.specifier_span
    }

    pub fn type_only(&self) -> bool {
        self.type_only
    }

    pub fn enclosing_function(&self) -> Option<Span> {
        self.enclosing_function
    }
}

impl Dependency {
    pub fn matches_specifier(&self, request: &str) -> bool {
        let Some(parts) = &self.pattern else {
            return request == self.specifier;
        };
        let Some(first) = parts.first() else {
            return false;
        };
        let Some(mut rest) = request.strip_prefix(first) else {
            return false;
        };
        if parts.len() == 1 {
            return rest.is_empty();
        }
        let Some(middle) = rest.strip_suffix(parts.last().unwrap()) else {
            return false;
        };
        rest = middle;
        for part in &parts[1..parts.len() - 1] {
            let Some(index) = rest.find(part) else {
                return false;
            };
            rest = &rest[index + part.len()..];
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportedName {
    Name(String),
    Namespace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportBinding {
    dependency: DependencyId,
    binding: BindingId,
    imported: ImportedName,
    #[serde(with = "span_serde")]
    span: Span,
    #[serde(with = "span_serde")]
    imported_span: Span,
    type_only: bool,
}

impl ImportBinding {
    pub(crate) fn new(
        dependency: DependencyId,
        binding: BindingId,
        imported: ImportedName,
        span: Span,
        imported_span: Span,
        type_only: bool,
    ) -> Self {
        Self {
            dependency,
            binding,
            imported,
            span,
            imported_span,
            type_only,
        }
    }
}

impl ImportBinding {
    pub fn dependency(&self) -> DependencyId {
        self.dependency
    }

    pub fn binding(&self) -> BindingId {
        self.binding
    }

    pub fn imported(&self) -> &ImportedName {
        &self.imported
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn imported_span(&self) -> Span {
        self.imported_span
    }

    pub fn type_only(&self) -> bool {
        self.type_only
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportValue {
    Binding(BindingId),
    ReExport {
        dependency: DependencyId,
        imported: ImportedName,
    },
    /// Includes anonymous default functions/classes and default expressions.
    Expression(#[serde(with = "span_serde")] Span),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    /// `None` means export-star, not a default export.
    name: Option<String>,
    value: ExportValue,
    #[serde(with = "span_serde")]
    span: Span,
    /// Absent for the implicit `default` name and export-star.
    #[serde(with = "span_serde::optional")]
    name_span: Option<Span>,
    type_only: bool,
}

impl Export {
    pub(crate) fn new(
        name: Option<String>,
        value: ExportValue,
        span: Span,
        name_span: Option<Span>,
        type_only: bool,
    ) -> Self {
        Self {
            name,
            value,
            span,
            name_span,
            type_only,
        }
    }
}

impl Export {
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn value(&self) -> &ExportValue {
        &self.value
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn name_span(&self) -> Option<Span> {
        self.name_span
    }

    pub fn type_only(&self) -> bool {
        self.type_only
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directive {
    value: String,
    #[serde(with = "span_serde")]
    span: Span,
}

impl Directive {
    pub(crate) fn new(value: String, span: Span) -> Self {
        Self { value, span }
    }
}

impl Directive {
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFacts {
    /// Owned lazy ESM operations, never solved effects. CJS stays opaque.
    ecco: Option<ecco::ParsedFile>,
    /// Effective contracts are identity inputs even for opaque CommonJS.
    ecco_declarations_hash: crate::DeclarationFingerprint,
    /// Effective parser format, including package policy for extensionless/.js input.
    commonjs: bool,
    /// Syntax-local await/for-await sites outside functions, not a schedule.
    #[serde(with = "span_serde::list")]
    top_level_await: Vec<Span>,
    effects: Vec<crate::effects::EffectSite>,
    invocations: Vec<crate::invocation::Invocation>,
    emission: crate::emission::EmissionFacts,
    bindings: Vec<Binding>,
    references: Vec<Reference>,
    dependencies: Vec<Dependency>,
    imports: Vec<ImportBinding>,
    exports: Vec<Export>,
    directives: Vec<Directive>,
    hashbang: Option<Directive>,
}

impl ModuleFacts {
    pub(crate) fn new(
        ecco: Option<ecco::ParsedFile>,
        ecco_declarations_hash: crate::DeclarationFingerprint,
        commonjs: bool,
        top_level_await: Vec<Span>,
        effects: Vec<crate::effects::EffectSite>,
        invocations: Vec<crate::invocation::Invocation>,
        emission: crate::emission::EmissionFacts,
        bindings: Vec<Binding>,
        references: Vec<Reference>,
        dependencies: Vec<Dependency>,
        imports: Vec<ImportBinding>,
        exports: Vec<Export>,
        directives: Vec<Directive>,
        hashbang: Option<Directive>,
    ) -> Self {
        Self {
            ecco,
            ecco_declarations_hash,
            commonjs,
            top_level_await,
            effects,
            invocations,
            emission,
            bindings,
            references,
            dependencies,
            imports,
            exports,
            directives,
            hashbang,
        }
    }

    pub(crate) fn ecco_mut(&mut self) -> &mut Option<ecco::ParsedFile> {
        &mut self.ecco
    }

    pub(crate) fn ecco_declarations_hash_mut(&mut self) -> &mut crate::DeclarationFingerprint {
        &mut self.ecco_declarations_hash
    }
}

impl ModuleFacts {
    pub fn ecco(&self) -> Option<&ecco::ParsedFile> {
        self.ecco.as_ref()
    }

    pub fn ecco_declarations_hash(&self) -> crate::DeclarationFingerprint {
        self.ecco_declarations_hash
    }

    pub fn commonjs(&self) -> bool {
        self.commonjs
    }

    pub fn top_level_await(&self) -> &[Span] {
        &self.top_level_await
    }

    pub fn effects(&self) -> &[crate::effects::EffectSite] {
        &self.effects
    }

    pub fn invocations(&self) -> &[crate::invocation::Invocation] {
        &self.invocations
    }

    pub fn emission(&self) -> &crate::emission::EmissionFacts {
        &self.emission
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    pub fn imports(&self) -> &[ImportBinding] {
        &self.imports
    }

    pub fn exports(&self) -> &[Export] {
        &self.exports
    }

    pub fn directives(&self) -> &[Directive] {
        &self.directives
    }

    pub fn hashbang(&self) -> Option<&Directive> {
        self.hashbang.as_ref()
    }
}

impl Binding {
    pub(super) fn declarations_mut(&mut self) -> &mut Vec<Span> {
        &mut self.declarations
    }
}

impl Export {
    pub(super) fn span_mut(&mut self) -> &mut Span {
        &mut self.span
    }
}

impl ModuleFacts {
    pub(super) fn bindings_mut(&mut self) -> &mut Vec<Binding> {
        &mut self.bindings
    }
    pub(super) fn references_mut(&mut self) -> &mut Vec<Reference> {
        &mut self.references
    }
    pub(super) fn dependencies_mut(&mut self) -> &mut Vec<Dependency> {
        &mut self.dependencies
    }
    pub(super) fn top_level_await_mut(&mut self) -> &mut Vec<Span> {
        &mut self.top_level_await
    }
    pub(super) fn imports_mut(&mut self) -> &mut Vec<ImportBinding> {
        &mut self.imports
    }
    pub(super) fn exports_mut(&mut self) -> &mut Vec<Export> {
        &mut self.exports
    }
    pub(super) fn emission_mut(&mut self) -> &mut crate::emission::EmissionFacts {
        &mut self.emission
    }
    pub(super) fn effects_mut(&mut self) -> &mut Vec<crate::effects::EffectSite> {
        &mut self.effects
    }
    pub(super) fn invocations_mut(&mut self) -> &mut Vec<crate::invocation::Invocation> {
        &mut self.invocations
    }
}
