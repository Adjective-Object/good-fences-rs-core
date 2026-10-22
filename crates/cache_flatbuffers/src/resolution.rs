//! Independent resolution wire format. No resolver implementation or filesystem dependency.
use std::sync::Arc;

use eyre::{ensure, Result};
use sha2::{Digest, Sha256};

use crate::{resolution_wire as wire, MAX_RECORD_BYTES};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub policy: [u8; 32],
    pub instance: [u8; 32],
    pub dependencies: [u8; 32],
}

/// Portable slash-only path, independent of the reader's host platform.
pub fn validate_path(path: &str, empty: bool) -> Result<()> {
    ensure!(
        (empty && path.is_empty())
            || (!path.is_empty()
                && path.split('/').all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && !part.ends_with(['.', ' '])
                        && !part
                            .chars()
                            .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
                        && !reserved_component(part)
                })),
        "unsafe resolution path"
    );
    Ok(())
}

fn reserved_component(part: &str) -> bool {
    let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || base
            .strip_prefix("COM")
            .or_else(|| base.strip_prefix("LPT"))
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
}

fn locator(path: wire::Locator<'_>) -> Result<()> {
    ensure!(
        matches!(
            path.anchor(),
            wire::Anchor::Project
                | wire::Anchor::Package
                | wire::Anchor::CanonicalProject
                | wire::Anchor::CanonicalPackage
                | wire::Anchor::NativeProject
                | wire::Anchor::NativePackage
                | wire::Anchor::WorkingDirectory
                | wire::Anchor::CanonicalWorkingDirectory
                | wire::Anchor::NativeWorkingDirectory
        ),
        "unknown resolution anchor"
    );
    ensure!(
        path.parents() <= 64
            && (path.parents() == 0
                || matches!(
                    path.anchor(),
                    wire::Anchor::Project
                        | wire::Anchor::CanonicalProject
                        | wire::Anchor::NativeProject
                )),
        "invalid resolution ancestor anchor"
    );
    validate_path(path.path(), true)?;
    ensure!(
        path.parents() == 0
            || path.path().is_empty()
            || path.path() == "package.json"
            || path.path() == "node_modules"
            || path.path().starts_with("node_modules/"),
        "unportable external resolution anchor"
    );
    Ok(())
}

fn text(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 16 * 1024 && !value.chars().any(char::is_control),
        "invalid resolution request text"
    );
    ensure!(
        !value.starts_with(['/', '\\'])
            && !value.starts_with("file:")
            && !(value.as_bytes().get(1) == Some(&b':')
                && value.as_bytes()[0].is_ascii_alphabetic()),
        "unportable absolute resolution request"
    );
    Ok(())
}

fn validate(root: wire::Resolution<'_>, key: Key) -> Result<()> {
    ensure!(
        root.schema() == SCHEMA_VERSION,
        "resolution cache version mismatch"
    );
    ensure!(
        root.policy().bytes().iter().eq(key.policy)
            && root.instance().bytes().iter().eq(key.instance)
            && root.dependencies().bytes().iter().eq(key.dependencies),
        "resolution cache fingerprint mismatch"
    );
    let mut previous = None;
    for request in root.requests() {
        ensure!(
            matches!(request.mode(), wire::Mode::Import | wire::Mode::Require),
            "unknown resolution request mode"
        );
        validate_path(request.importer(), false)?;
        text(request.specifier())?;
        let order = (request.mode().0, request.importer(), request.specifier());
        ensure!(
            previous.is_none_or(|p| p < order),
            "unordered or duplicate resolution requests"
        );
        previous = Some(order);
        let target = request.target();
        match target.kind() {
            wire::TargetKind::File => {
                ensure!(target.external().is_none(), "ambiguous resolution target");
                let file = target
                    .file()
                    .ok_or_else(|| eyre::eyre!("missing file target"))?;
                locator(file)?;
                ensure!(!file.path().is_empty(), "empty file target");
            }
            wire::TargetKind::External => {
                ensure!(target.file().is_none(), "ambiguous resolution target");
                text(
                    target
                        .external()
                        .ok_or_else(|| eyre::eyre!("missing external target"))?,
                )?;
            }
            _ => eyre::bail!("unknown resolution target kind"),
        }
        ensure!(
            !request.inputs().is_empty() || target.kind() == wire::TargetKind::External,
            "unproven resolution request"
        );
        for input in request.inputs() {
            locator(input.path())?;
            ensure!(
                matches!(
                    input.kind(),
                    wire::Kind::Missing
                        | wire::Kind::File
                        | wire::Kind::Directory
                        | wire::Kind::Other
                ),
                "unknown resolution input kind"
            );
            let present = input.kind() != wire::Kind::Missing;
            ensure!(
                input.identity().is_some() == present && input.canonical().is_some() == present,
                "invalid resolution input identity"
            );
            ensure!(
                input.content().is_none() || input.kind() == wire::Kind::File,
                "non-file resolution content seal"
            );
            if let Some(path) = input.canonical() {
                locator(path)?;
            }
            let mut links = std::collections::BTreeSet::new();
            for link in input.links() {
                locator(link.path())?;
                locator(link.target())?;
                // Links are observed in prefix order. Host-dependent ordering is
                // checked by the filesystem adapter, not lexical table order.
                let order = (
                    link.path().anchor().0,
                    link.path().parents(),
                    link.path().path(),
                );
                ensure!(links.insert(order), "duplicate resolution link");
                if let Some(relative) = link.relative() {
                    ensure!(
                        !relative.is_empty()
                            && relative.len() <= 16 * 1024
                            && relative
                                .split('/')
                                .all(|p| p == ".." || p == "." || validate_path(p, false).is_ok()),
                        "unsafe relative resolution link"
                    );
                }
            }
        }
    }
    Ok(())
}

/// Structurally and semantically verified immutable wire bytes. Filesystem
/// proofs still require fresh validation at the consumer boundary.
#[derive(Clone, Debug)]
pub struct WireResolution {
    bytes: Arc<[u8]>,
}

impl WireResolution {
    /// Rebuild an aggregate directly from verified borrowed tables. No owned
    /// proof tree is reconstructed when preserving requests from an older file.
    pub fn aggregate(key: Key, requests: &[wire::Request<'_>]) -> Result<Self> {
        use flatbuffers::{FlatBufferBuilder, WIPOffset};
        fn locator<'a>(
            builder: &mut FlatBufferBuilder<'a>,
            value: wire::Locator<'_>,
        ) -> WIPOffset<wire::Locator<'a>> {
            let path = builder.create_string(value.path());
            wire::Locator::create(
                builder,
                &wire::LocatorArgs {
                    anchor: value.anchor(),
                    parents: value.parents(),
                    path: Some(path),
                },
            )
        }
        let mut builder = FlatBufferBuilder::new();
        let mut entries = Vec::new();
        for request in requests {
            let mut inputs = Vec::new();
            for input in request.inputs() {
                let path = locator(&mut builder, input.path());
                let canonical = input.canonical().map(|p| locator(&mut builder, p));
                let mut links = Vec::new();
                for link in input.links() {
                    let path = locator(&mut builder, link.path());
                    let target = locator(&mut builder, link.target());
                    let relative = link.relative().map(|s| builder.create_string(s));
                    links.push(wire::Link::create(
                        &mut builder,
                        &wire::LinkArgs {
                            path: Some(path),
                            target: Some(target),
                            relative,
                        },
                    ));
                }
                let links = builder.create_vector(&links);
                inputs.push(wire::Input::create(
                    &mut builder,
                    &wire::InputArgs {
                        path: Some(path),
                        kind: input.kind(),
                        canonical,
                        identity: input.identity(),
                        content: input.content(),
                        links: Some(links),
                    },
                ));
            }
            let inputs = builder.create_vector(&inputs);
            let file = request.target().file().map(|p| locator(&mut builder, p));
            let external = request
                .target()
                .external()
                .map(|s| builder.create_string(s));
            let target = wire::Target::create(
                &mut builder,
                &wire::TargetArgs {
                    kind: request.target().kind(),
                    file,
                    external,
                },
            );
            let importer = builder.create_string(request.importer());
            let specifier = builder.create_string(request.specifier());
            entries.push(wire::Request::create(
                &mut builder,
                &wire::RequestArgs {
                    mode: request.mode(),
                    importer: Some(importer),
                    specifier: Some(specifier),
                    target: Some(target),
                    inputs: Some(inputs),
                },
            ));
        }
        let requests = builder.create_vector(&entries);
        let policy = wire::Digest::new(&key.policy);
        let instance = wire::Digest::new(&key.instance);
        let dependencies = wire::Digest::new(&key.dependencies);
        let root = wire::Resolution::create(
            &mut builder,
            &wire::ResolutionArgs {
                schema: SCHEMA_VERSION,
                policy: Some(&policy),
                instance: Some(&instance),
                dependencies: Some(&dependencies),
                requests: Some(requests),
            },
        );
        builder.finish_size_prefixed(root, Some("GFRS"));
        let mut bytes = builder.finished_data().to_vec();
        bytes.extend_from_slice(&Sha256::digest(builder.finished_data()));
        Self::new(bytes.into(), key)
    }

    /// Construction-only object API. Warm readers never unpack.
    pub fn encode(data: &wire::ResolutionT, key: Key) -> Result<Self> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let root = data.pack(&mut builder);
        builder.finish_size_prefixed(root, Some("GFRS"));
        let mut bytes = builder.finished_data().to_vec();
        bytes.extend_from_slice(&Sha256::digest(builder.finished_data()));
        Self::new(bytes.into(), key)
    }

    pub fn new(bytes: Arc<[u8]>, key: Key) -> Result<Self> {
        ensure!(
            bytes.len() >= 44 && bytes.len() <= MAX_RECORD_BYTES,
            "invalid resolution cache size"
        );
        let (payload, checksum) = bytes.split_at(bytes.len() - 32);
        ensure!(
            Sha256::digest(payload).as_slice() == checksum,
            "resolution cache checksum mismatch"
        );
        ensure!(
            u32::from_le_bytes(payload[..4].try_into()?) as usize == payload.len() - 4,
            "resolution cache size prefix mismatch"
        );
        ensure!(
            wire::resolution_size_prefixed_buffer_has_identifier(payload),
            "invalid resolution cache identifier"
        );
        let options = flatbuffers::VerifierOptions {
            max_depth: 16,
            max_tables: 262_144,
            max_apparent_size: MAX_RECORD_BYTES * 4,
            ..Default::default()
        };
        let root = wire::size_prefixed_root_as_resolution_with_opts(&options, payload)?;
        validate(root, key)?;
        Ok(Self { bytes })
    }

    pub fn root(&self) -> wire::Resolution<'_> {
        // SAFETY: construction verifies these exact immutable bytes, including
        // all discriminants and borrowed semantic invariants.
        unsafe {
            wire::size_prefixed_root_as_resolution_unchecked(&self.bytes[..self.bytes.len() - 32])
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests;
