use bundler::{build::BuildResult, publish::Published};
use std::io::Write;

pub(crate) fn write(
    stdout: &mut impl Write,
    build: &BuildResult,
    published: &Published,
) -> eyre::Result<()> {
    writeln!(
        stdout,
        "{}",
        serde_json::json!({
            "entries": published.entries,
            "backend": build.product.artifacts.backend,
            "layout": build.product.artifacts.backend.layout(),
            "directory": published.directory,
            "stylesheets": build.product.artifacts.stylesheets,
            "resources": build.product.artifacts.resources.keys().collect::<Vec<_>>(),
            "generation": build.product.generation,
            "sourceParses": build.counters.source_parses,
            "speculativeParses": build.counters.speculative_parses,
            "declarationFingerprints": build.counters.declaration_fingerprints,
            "diskHits": build.counters.disk_hits,
            "resolutions": build.counters.resolutions,
            "resolutionCacheHits": build.counters.resolution_cache_hits,
            "resolutionCacheMisses": build.counters.resolution_cache_misses,
            "inventoryDirectoryReads": build.counters.inventory_directory_reads,
            "emissions": build.counters.emissions,
            "outputParses": build.counters.output_parses,
            "sourceReparses": build.counters.source_reparses,
            "fragmentReparses": build.counters.fragment_reparses,
            "finalValidations": build.counters.final_validations,
            "artifactWrites": published.files_written,
            "warnings": build.warnings,
            "planWarnings": build.product.plan.retention.warnings,
            "publicationWarnings": published.warnings,
            "generatedBoundaries": build.product.plan.generated_boundaries.len(),
            "moduleMembers": build.product.artifacts.members.iter().filter_map(|(node, artifact)| {
                let bundler::chunks::NodeId::Module(source) = node else { return None; };
                Some(serde_json::json!({"source": source, "artifact": artifact}))
            }).collect::<Vec<_>>(),
            "segmentMembers": build.product.artifacts.members.iter().filter_map(|(node, artifact)| {
                let bundler::chunks::NodeId::Segment {module, index} = node else { return None; };
                let roots = build.product.plan.node_roots.get(node).into_iter().flatten().map(|root| {
                    use bundler::chunks::RootId;
                    match root {
                        RootId::Entry(entry) => serde_json::json!({"entry": entry}),
                        RootId::Async(site) => serde_json::json!({"module": site.module, "offset": site.offset}),
                        RootId::AsyncExport {site, name} => serde_json::json!({
                            "module": site.module, "offset": site.offset, "name": name
                        }),
                    }
                }).collect::<Vec<_>>();
                Some(serde_json::json!({"source": module, "segment": index, "artifact": artifact, "roots": roots}))
            }).collect::<Vec<_>>(),
        })
    )?;
    Ok(())
}
