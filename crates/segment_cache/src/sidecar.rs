use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use ast_segmenter::ModuleFormat;
use eyre::{ensure, WrapErr};

use crate::{CacheRecord, ParserConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    ReadWrite,
    ReadOnly,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissReason {
    Disabled,
    Missing,
    ReadFailure(String),
    InvalidRecord(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    Miss(MissReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheWrite {
    NotRequested,
    Stored,
    Failed(String),
}

#[derive(Debug)]
pub struct CacheOutcome {
    pub source: String,
    pub record: CacheRecord,
    pub status: CacheStatus,
    pub write: CacheWrite,
}

/// Generated JavaScript and the exact on-disk input from which it was prepared.
pub struct PreparedSource<'a> {
    pub source: &'a str,
    pub original: &'a [u8],
    pub parser_path: &'a Path,
    pub package: ModuleFormat,
}

enum PolicyHandling<F> {
    Strict(F),
    Report(F),
}

pub fn load_prepared(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ecco::LocalFacts,
) -> eyre::Result<CacheOutcome> {
    let declarations = ast_segmenter::PreparedDeclarations::new(declarations.clone())?;
    load_prepared_with_declarations(logger, path, prepared, mode, &declarations)
}

/// Resource loading with a shared, validated declaration configuration.
pub fn load_prepared_with_declarations(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
) -> eyre::Result<CacheOutcome> {
    load_prepared_with_dependencies(logger, path, prepared, mode, declarations, |_| {})
}

/// Report fresh checked dependency syntax only; cache hits retain borrowed facts.
/// Observations must not be published unless the complete load succeeds.
pub fn load_prepared_with_dependencies(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
    dependencies: impl FnOnce(&[ast_segmenter::owned::Dependency]),
) -> eyre::Result<CacheOutcome> {
    load_prepared_inner(
        logger,
        path,
        prepared,
        mode,
        declarations,
        || Ok(()),
        PolicyHandling::Strict(
            |deps: &[ast_segmenter::owned::Dependency],
             _: &[ast_segmenter::policy::PolicyViolation]| dependencies(deps),
        ),
    )
    .wrap_err_with(|| format!("loading prepared source cache for {}", path.display()))
}

/// Extract/cache source-local policy evidence without admitting it. Unlike the
/// strict loaders, every caller must enforce its current policy on returned hits.
pub fn load_prepared_reporting_policies(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
    observations: impl FnOnce(
        &[ast_segmenter::owned::Dependency],
        &[ast_segmenter::policy::PolicyViolation],
    ),
) -> eyre::Result<CacheOutcome> {
    load_prepared_inner(
        logger,
        path,
        prepared,
        mode,
        declarations,
        || Ok(()),
        PolicyHandling::Report(observations),
    )
    .wrap_err_with(|| {
        format!(
            "loading policy-reporting source cache for {}",
            path.display()
        )
    })
}

pub fn sidecar_path(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_owned();
    name.push(".seg.fb");
    PathBuf::from(name)
}

pub fn is_cache_artifact(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        let name = n.to_string_lossy();
        name.ends_with(".seg.fb")
            || name.ends_with(".seg.cbor")
            || name.contains(".seg.fb.stage-")
            || name.contains(".seg.cbor.stage-")
    })
}

/// Reads a checked source snapshot; cache failures never hide source diagnostics.
pub fn load_or_parse(
    logger: &impl logger::Logger,
    path: &Path,
    package: ModuleFormat,
    mode: CacheMode,
) -> eyre::Result<CacheOutcome> {
    load_or_parse_with_declarations(
        logger,
        path,
        package,
        mode,
        &ecco::DeclarationConfig::default().effective()?,
    )
}

pub fn load_or_parse_with_declarations(
    logger: &impl logger::Logger,
    path: &Path,
    package: ModuleFormat,
    mode: CacheMode,
    declarations: &ecco::LocalFacts,
) -> eyre::Result<CacheOutcome> {
    let declarations = ast_segmenter::PreparedDeclarations::new(declarations.clone())?;
    load_or_parse_with_prepared_declarations(logger, path, package, mode, &declarations)
}

pub fn load_or_parse_with_prepared_declarations(
    logger: &impl logger::Logger,
    path: &Path,
    package: ModuleFormat,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
) -> eyre::Result<CacheOutcome> {
    load(logger, path, package, mode, declarations, || Ok(()))
}

fn read_cache(path: &Path) -> Result<cache_flatbuffers::SharedBytes, MissReason> {
    let _span = tracing::info_span!("cache.read").entered();
    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MissReason::Missing
        } else {
            MissReason::ReadFailure(format!("opening cache {}: {e}", path.display()))
        }
    })?;
    cache_flatbuffers::SharedBytes::snapshot(file).map_err(|e| {
        MissReason::ReadFailure(format!("snapshotting cache {}: {e:#}", path.display()))
    })
}

fn unchanged(path: &Path, original: &[u8]) -> eyre::Result<()> {
    let _span = tracing::info_span!("cache.recheck").entered();
    ensure!(
        fs::read(path).wrap_err_with(|| format!("{}: source recheck failed", path.display()))?
            == original,
        "{}: source changed during cache operation",
        path.display()
    );
    Ok(())
}

fn load(
    logger: &impl logger::Logger,
    path: &Path,
    package: ModuleFormat,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
    before_publish: impl FnOnce() -> eyre::Result<()>,
) -> eyre::Result<CacheOutcome> {
    let source = fs::read_to_string(path)
        .wrap_err_with(|| format!("{}: source read failed", path.display()))?;
    load_prepared_inner(
        logger,
        path,
        PreparedSource {
            source: &source,
            original: source.as_bytes(),
            parser_path: path,
            package,
        },
        mode,
        declarations,
        before_publish,
        PolicyHandling::Strict(
            |_: &[ast_segmenter::owned::Dependency],
             _: &[ast_segmenter::policy::PolicyViolation]| {},
        ),
    )
}

fn load_prepared_inner(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ast_segmenter::PreparedDeclarations,
    before_publish: impl FnOnce() -> eyre::Result<()>,
    policies: PolicyHandling<
        impl FnOnce(&[ast_segmenter::owned::Dependency], &[ast_segmenter::policy::PolicyViolation]),
    >,
) -> eyre::Result<CacheOutcome> {
    let (strict, observations) = match policies {
        PolicyHandling::Strict(observations) => (true, observations),
        PolicyHandling::Report(observations) => (false, observations),
    };
    let source = prepared.source;
    let _span = tracing::info_span!("cache.load", bytes = source.len()).entered();
    let config =
        ParserConfig::for_path(prepared.parser_path, prepared.package).wrap_err_with(|| {
            format!(
                "selecting cache parser mode for {}",
                prepared.parser_path.display()
            )
        })?;
    let cached = if mode == CacheMode::Disabled {
        Err(MissReason::Disabled)
    } else {
        read_cache(&sidecar_path(path)).and_then(|bytes| {
            CacheRecord::decode_backing(bytes, source, config)
                .and_then(|record| {
                    record.check_declarations(declarations)?;
                    Ok(record)
                })
                .map_err(|e| MissReason::InvalidRecord(format!("{e:#}")))
        })
    };
    let (record, status, write) = match cached {
        Ok(record) => (record, CacheStatus::Hit, CacheWrite::NotRequested),
        Err(reason) => {
            let extraction_span = tracing::info_span!("cache.extract").entered();
            let parsed = if strict {
                ast_segmenter::parse_module_with_dependencies(
                    logger,
                    source,
                    prepared.parser_path,
                    prepared.package,
                    declarations,
                    |deps| observations(deps, &[]),
                )
            } else {
                ast_segmenter::parse_module_reporting_policies(
                    logger,
                    source,
                    prepared.parser_path,
                    prepared.package,
                    declarations,
                    observations,
                )
            }
            .wrap_err_with(|| format!("extracting source facts for {}", path.display()))?;
            let record = CacheRecord::from_parsed_reporting_policies(source, &parsed)
                .wrap_err_with(|| {
                    format!("constructing source cache record for {}", path.display())
                })?;
            drop(parsed);
            drop(extraction_span);
            let write = if mode == CacheMode::ReadWrite {
                match publish(path, source, prepared.original, &record, before_publish) {
                    Ok(()) => CacheWrite::Stored,
                    Err(e) => CacheWrite::Failed(format!("{e:#}")),
                }
            } else {
                CacheWrite::NotRequested
            };
            (record, CacheStatus::Miss(reason), write)
        }
    };
    if strict {
        enforce_policies(path, source, record.module().facts())?;
    }
    unchanged(path, prepared.original)?;
    Ok(CacheOutcome {
        source: source.to_owned(),
        record,
        status,
        write,
    })
}

pub(crate) fn enforce_policies(
    path: &Path,
    source: &str,
    facts: ast_segmenter::views::ModuleFacts<'_>,
) -> eyre::Result<()> {
    let violations: Vec<_> = facts.policy_violations().iter().collect();
    if !violations.is_empty() {
        return Err(
            ast_segmenter::policy::PolicyErrors::new(path, Some(source), violations).into(),
        );
    }
    Ok(())
}

fn publish(
    path: &Path,
    source: &str,
    original: &[u8],
    record: &CacheRecord,
    before_publish: impl FnOnce() -> eyre::Result<()>,
) -> eyre::Result<()> {
    let _span = tracing::info_span!("cache.publish").entered();
    let destination = sidecar_path(path);
    let bytes = record
        .wire_bytes(source)
        .wrap_err_with(|| format!("checking cache source for {}", destination.display()))?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut prefix = destination
        .file_name()
        .ok_or_else(|| eyre::eyre!("missing sidecar name"))?
        .to_owned();
    prefix.push(".stage-");
    let mut staging = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .wrap_err_with(|| format!("creating cache staging file for {}", destination.display()))?;
    staging
        .write_all(bytes)
        .wrap_err_with(|| format!("writing cache staging file for {}", destination.display()))?;
    staging
        .flush()
        .wrap_err_with(|| format!("flushing cache staging file for {}", destination.display()))?;
    staging
        .as_file()
        .sync_all()
        .wrap_err_with(|| format!("syncing cache staging file for {}", destination.display()))?;
    before_publish()?;
    unchanged(path, original)?;
    let staging = staging.into_temp_path();
    // std also handles replacement of open destinations on Windows.
    fs::rename(&staging, &destination)
        .wrap_err_with(|| format!("replacing cache {}", destination.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
