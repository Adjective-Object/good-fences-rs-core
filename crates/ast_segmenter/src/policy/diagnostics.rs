use std::{fmt, path::Path};

use oxc_span::Span;

macro_rules! policies {
    ($($variant:ident = $id:literal => ($name:literal, $message:literal)),+ $(,)?) => {
        /// Stable source-local support-policy names. These are not linker policies.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        pub enum PolicyName {
            $(#[serde(rename = $name)] $variant = $id,)+
        }
        impl PolicyName {
            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $name,)+ }
            }
            pub fn message(self) -> &'static str {
                match self { $(Self::$variant => $message,)+ }
            }
            pub fn from_id(id: u16) -> Option<Self> {
                match id { $($id => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

policies! {
    UsesEval = 0 => ("uses-eval", "unsupported unshadowed eval operation"),
    RequireReassignment = 1 => ("require-reassignment", "unsupported require reassignment"),
    RequireIndirectUse = 2 => ("require-indirect-use", "unsupported require alias or indirect use"),
    MixedEsmCommonjs = 3 => ("mixed-esm-commonjs", "mixed ESM/CommonJS wrapper reference or mixed CommonJS/ESM declaration"),
    ModuleAccess = 4 => ("module-access", "unsupported module access (expected module.exports or a guarded module detector)"),
    EsmRequireContext = 5 => ("esm-require-context", "unsupported ESM require context (expected ./ relative template/addition pattern without parent traversal)"),
    WrapperArguments = 6 => ("wrapper-arguments", "unsupported CommonJS wrapper arguments access"),
    WrapperRedeclaration = 7 => ("wrapper-redeclaration", "unsupported CommonJS wrapper redeclaration"),
    ImportAttributes = 8 => ("import-attributes", "unsupported import attributes or import phase"),
    JsonImportBindings = 9 => ("json-import-bindings", "JSON import attributes support only default or side-effect imports"),
    DynamicImportOptions = 10 => ("dynamic-import-options", "unsupported dynamic import options or phase"),
    ImportTypeOptions = 11 => ("import-type-options", "unsupported import type options"),
    WithStatement = 12 => ("with-statement", "unsupported with statement"),
    ImportMeta = 13 => ("import-meta", "unsupported import.meta"),
    CommonjsTopLevelAwait = 14 => ("commonjs-top-level-await", "unsupported top-level await or top-level for-await"),
    ResourceManagementTransform = 15 => ("resource-management-transform", "unsupported resource-management transform"),
    JsxTransform = 16 => ("jsx-transform", "unsupported JSX transform"),
    DecoratorTransform = 17 => ("decorator-transform", "unsupported decorator transform"),
    AutoAccessorTransform = 18 => ("auto-accessor-transform", "unsupported auto-accessor transform"),
    TypescriptEnumTransform = 19 => ("typescript-enum-transform", "unsupported TypeScript enum transform"),
    TypescriptNamespaceTransform = 20 => ("typescript-namespace-transform", "unsupported TypeScript namespace transform"),
    TypescriptParameterPropertyTransform = 21 => ("typescript-parameter-property-transform", "unsupported TypeScript parameter property transform"),
    TypescriptImportAssignment = 22 => ("typescript-import-assignment", "unsupported TypeScript import assignment"),
    TypescriptExportAssignment = 23 => ("typescript-export-assignment", "unsupported TypeScript export assignment"),
    TypescriptNamespaceExport = 24 => ("typescript-namespace-export", "unsupported TypeScript namespace export"),
}

impl fmt::Display for PolicyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Syntax evidence, independent of any caller's admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyViolation {
    policy: PolicyName,
    #[serde(with = "crate::owned::span_serde")]
    span: Span,
}

impl PolicyViolation {
    pub fn new(policy: PolicyName, span: Span) -> Self {
        Self { policy, span }
    }

    pub fn policy(self) -> PolicyName {
        self.policy
    }

    pub fn span(self) -> Span {
        self.span
    }

    /// Bounded context, including following call arguments near short identifiers.
    /// Invalid or missing source coordinates still retain the original byte span.
    pub fn diagnostic(self, path: &Path, source: Option<&str>) -> String {
        let mut result = format!(
            "{}: policy {} at bytes {}..{}: {}",
            path.display(),
            self.policy,
            self.span.start,
            self.span.end,
            self.policy.message()
        );
        let Some(source) = source else { return result };
        let start = self.span.start as usize;
        let end = self.span.end as usize;
        if source.get(start..end).is_none() {
            return result;
        }
        let line = source[..start].bytes().filter(|b| *b == b'\n').count() + 1;
        let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
        let column = source[line_start..start].chars().count() + 1;
        result.push_str(&format!(" (line {line}, column {column})"));
        let mut left = line_start.max(start.saturating_sub(100));
        let mut right = source.len().min(start.saturating_add(220));
        while !source.is_char_boundary(left) {
            left += 1;
        }
        while !source.is_char_boundary(right) {
            right -= 1;
        }
        if let Some((offset, _)) = source[start..right].match_indices('\n').nth(2) {
            right = start + offset;
        }
        result.push_str("\n  ");
        if left > line_start {
            result.push('…');
        }
        // Control characters must not inject terminal escapes into diagnostics.
        result.extend(
            source[left..right]
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c }),
        );
        if right < source.len() {
            result.push('…');
        }
        result
    }
}

/// A typed policy failure retaining every source-local occurrence.
#[derive(Debug)]
pub struct PolicyErrors {
    violations: Vec<PolicyViolation>,
    diagnostics: Vec<String>,
}

impl PolicyErrors {
    pub fn new(path: &Path, source: Option<&str>, violations: Vec<PolicyViolation>) -> Self {
        let diagnostics = violations
            .iter()
            .map(|v| v.diagnostic(path, source))
            .collect();
        Self {
            violations,
            diagnostics,
        }
    }

    pub fn violations(&self) -> &[PolicyViolation] {
        &self.violations
    }
}

impl fmt::Display for PolicyErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostics.join("\n"))
    }
}

impl std::error::Error for PolicyErrors {}
