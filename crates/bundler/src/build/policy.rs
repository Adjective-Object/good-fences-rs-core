use super::*;

pub use ast_segmenter::policy::PolicyName;
use ast_segmenter::policy::{PolicyErrors, PolicyViolation};

/// An exact, portable root-relative source path and its explicitly waived policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyAllowance {
    source: String,
    policies: BTreeSet<PolicyName>,
}

impl PolicyAllowance {
    pub fn new(
        source: impl Into<String>,
        policies: impl IntoIterator<Item = PolicyName>,
    ) -> eyre::Result<Self> {
        let source = source.into();
        ensure!(
            !source.is_empty()
                && !source.chars().any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
                && source.split('/').all(|part| !matches!(part, "" | "." | "..")),
            "policyAllowlist source must be an exact portable root-relative file (no globs, absolute paths or traversal): {source:?}"
        );
        let policies: BTreeSet<_> = policies.into_iter().collect();
        ensure!(
            !policies.is_empty(),
            "policyAllowlist policies must not be empty"
        );
        Ok(Self { source, policies })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn policies(&self) -> &BTreeSet<PolicyName> {
        &self.policies
    }
}

#[derive(Default)]
pub(super) struct Admission {
    pub warnings: Vec<String>,
    pub error: Option<PolicyErrors>,
}

impl Admission {
    pub(super) fn report(
        self,
        path: &Path,
        reported: &mut BTreeSet<PathBuf>,
        warnings: &mut Vec<String>,
        errors: &mut BTreeMap<PathBuf, Vec<eyre::Report>>,
    ) {
        if self.warnings.is_empty() && self.error.is_none() {
            return;
        }
        if !reported.insert(path.to_owned()) {
            return;
        }
        for warning in self.warnings {
            tracing::warn!(target: "bundler::diagnostic", "{warning}");
            warnings.push(warning);
        }
        if let Some(error) = self.error {
            tracing::error!(target: "bundler::diagnostic", "{error}");
            errors
                .entry(path.to_owned())
                .or_default()
                .push(error.into());
        }
    }

    pub(super) fn check(
        root: &Path,
        path: &Path,
        source: &str,
        violations: impl IntoIterator<Item = PolicyViolation>,
        allowlist: &[PolicyAllowance],
    ) -> Self {
        let mut violations = violations.into_iter().peekable();
        if violations.peek().is_none() {
            return Self::default();
        }
        // Source identities are canonical, not caller aliases. Never resolve an
        // allowlist spelling through links or turn an external path into authority.
        let relative = path
            .strip_prefix(root)
            .ok()
            .and_then(|path| {
                path.components()
                    .map(|c| c.as_os_str().to_str())
                    .collect::<Option<Vec<_>>>()
            })
            .map(|parts| parts.join("/"));
        let mut result = Self::default();
        let mut denied = Vec::new();
        for violation in violations {
            let allowed = relative.as_ref().is_some_and(|relative| {
                allowlist.iter().any(|entry| {
                    entry.source == *relative && entry.policies.contains(&violation.policy())
                })
            });
            if allowed {
                result.warnings.push(format!(
                    "POLICY WAIVER: bundled output may be incorrect; source code is preserved without repair.\n{}",
                    violation.diagnostic(path, Some(source))
                ));
            } else {
                denied.push(violation);
            }
        }
        if !denied.is_empty() {
            result.error = Some(PolicyErrors::new(path, Some(source), denied));
        }
        result
    }
}

/// Deterministic stage failures; each report retains its original typed chain.
#[derive(Debug)]
pub struct PreparationErrors {
    errors: BTreeMap<PathBuf, Vec<eyre::Report>>,
}

impl PreparationErrors {
    pub fn errors(&self) -> &BTreeMap<PathBuf, Vec<eyre::Report>> {
        &self.errors
    }

    pub(super) fn new(mut errors: BTreeMap<PathBuf, Vec<eyre::Report>>) -> Self {
        for reports in errors.values_mut() {
            reports.sort_by_cached_key(|error| format!("{error:#}"));
        }
        Self { errors }
    }
}

impl std::fmt::Display for PreparationErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "source preparation failed for {} file(s)",
            self.errors.len()
        )?;
        for (path, errors) in &self.errors {
            for error in errors {
                write!(f, "\n{}: {error:#}", path.display())?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for PreparationErrors {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.errors
            .values()
            .next()?
            .first()
            .map(|error| error.as_ref())
    }
}
