use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use ast_segmenter::ModuleFormat;
use eyre::{ensure, WrapErr};

use crate::{CacheRecord, ParserConfig, MAX_RECORD_BYTES};

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

pub fn load_prepared(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ecco::LocalFacts,
) -> eyre::Result<CacheOutcome> {
    load_prepared_inner(logger, path, prepared, mode, declarations, || Ok(()))
}

pub fn sidecar_path(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_owned();
    name.push(".seg.cbor");
    PathBuf::from(name)
}

pub fn is_cache_artifact(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        let name = n.to_string_lossy();
        name.ends_with(".seg.cbor") || name.contains(".seg.cbor.stage-")
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
    load(logger, path, package, mode, declarations, || Ok(()))
}

fn read_cache(path: &Path) -> Result<Vec<u8>, MissReason> {
    let file = File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            MissReason::Missing
        } else {
            MissReason::ReadFailure(e.to_string())
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| MissReason::ReadFailure(e.to_string()))?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(MissReason::InvalidRecord("cache exceeds byte limit".into()));
    }
    Ok(bytes)
}

fn unchanged(path: &Path, original: &[u8]) -> eyre::Result<()> {
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
    declarations: &ecco::LocalFacts,
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
    )
}

fn load_prepared_inner(
    logger: &impl logger::Logger,
    path: &Path,
    prepared: PreparedSource<'_>,
    mode: CacheMode,
    declarations: &ecco::LocalFacts,
    before_publish: impl FnOnce() -> eyre::Result<()>,
) -> eyre::Result<CacheOutcome> {
    let source = prepared.source;
    let config = ParserConfig::for_path(prepared.parser_path, prepared.package)?;
    let cached = if mode == CacheMode::Disabled {
        Err(MissReason::Disabled)
    } else {
        read_cache(&sidecar_path(path)).and_then(|bytes| {
            CacheRecord::decode_with_declarations(&bytes, source, config, declarations)
                .map_err(|e| MissReason::InvalidRecord(format!("{e:#}")))
        })
    };
    let (record, status, write) = match cached {
        Ok(record) => (record, CacheStatus::Hit, CacheWrite::NotRequested),
        Err(reason) => {
            let parsed = ast_segmenter::parse_module_with_declarations(
                logger,
                source,
                prepared.parser_path,
                prepared.package,
                declarations.clone(),
            )?;
            let record = CacheRecord::from_parsed(source, &parsed)?;
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
    unchanged(path, prepared.original)?;
    Ok(CacheOutcome {
        source: source.to_owned(),
        record,
        status,
        write,
    })
}

fn publish(
    path: &Path,
    source: &str,
    original: &[u8],
    record: &CacheRecord,
    before_publish: impl FnOnce() -> eyre::Result<()>,
) -> eyre::Result<()> {
    let bytes = record.encode(source)?;
    let destination = sidecar_path(path);
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
        .tempfile_in(parent)?;
    staging.write_all(&bytes)?;
    staging.flush()?;
    staging.as_file().sync_all()?;
    before_publish()?;
    unchanged(path, original)?;
    staging
        .persist(&destination)
        .map_err(|e| eyre::eyre!("cache replacement failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
