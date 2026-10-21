//! Effective lexical inputs after certified graph substitutions.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Reference {
    pub symbol: crate::symbols::Symbol,
    pub write: bool,
    pub eager: bool,
}

#[derive(Default)]
pub(super) struct References {
    segments: BTreeMap<SegmentKey, Vec<Reference>>,
}

impl References {
    pub fn new(graph: &SourceGraph, plan: &ChunkPlan) -> eyre::Result<Self> {
        let mut result = Self::default();
        let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
        let consumed_bindings: HashSet<_> = plan
            .lazy_modules
            .modules()
            .iter()
            .map(|module| (module.owner.file_id, module.binding))
            .collect();
        let mut consumed: BTreeSet<_> = plan
            .lazy_modules
            .modules()
            .iter()
            .map(|module| (module.owner.file_id, module.constructor_reference))
            .collect();
        consumed.extend(plan.generated_boundaries.values().map(|boundary| {
            let GraphNode::Segment(consumer) = graph.edges()[boundary.source_edge].from else {
                unreachable!()
            };
            (consumer.file_id, boundary.reference)
        }));
        for &file in &plan.retention.modules {
            let facts = graph
                .file_facts(file)
                .ok_or_eyre("missing reference facts")?;
            let segments = graph.file_segments(file).unwrap();
            for (index, reference) in facts.references().iter().enumerate() {
                if reference.type_only() || consumed.contains(&(file, index as u32)) {
                    continue;
                }
                let Some(binding) = reference.binding() else {
                    continue;
                };
                let segment =
                    segments.partition_point(|segment| segment.span.end <= reference.span().start);
                let key = SegmentKey::new(file, segment as u32);
                if !live.contains(&key) || consumed_bindings.contains(&(file, binding)) {
                    continue;
                }
                result.segments.entry(key).or_default().push(Reference {
                    symbol: (file, binding),
                    write: reference.write(),
                    eager: facts
                        .emission()
                        .segments()
                        .get(segment)
                        .is_some_and(|recipe| {
                            recipe.placement().eager_reads().contains(&(index as u32))
                        }),
                });
            }
        }
        // Synthesized instances reference the original constructor at the selector,
        // not at the now-consumed declaration. Options remain a once-only slot.
        for activation in plan.async_activations.values() {
            let crate::chunks::AsyncOrigin::Lazy {
                declaration,
                module_candidate,
                selector,
            } = activation.origin
            else {
                continue;
            };
            let module = plan
                .lazy_modules
                .modules()
                .iter()
                .find(|module| module.owner == declaration && module.candidate == module_candidate)
                .ok_or_eyre("missing certified module")?;
            let selector = module
                .selectors
                .iter()
                .find(|s| s.candidate == selector)
                .ok_or_eyre("missing certified selector")?;
            let facts = graph.file_facts(declaration.file_id).unwrap();
            let binding = facts
                .references()
                .get(module.constructor_reference as usize)
                .and_then(|reference| reference.binding())
                .ok_or_eyre("missing module constructor binding")?;
            result
                .segments
                .entry(selector.owner)
                .or_default()
                .push(Reference {
                    symbol: (declaration.file_id, binding),
                    write: false,
                    eager: facts
                        .emission()
                        .segments()
                        .get(selector.owner.segment_idx as usize)
                        .is_some_and(|recipe| {
                            recipe
                                .placement()
                                .eager_reads()
                                .contains(&selector.constructor_reference)
                        }),
                });
        }
        Ok(result)
    }

    pub fn get(&self, key: SegmentKey) -> &[Reference] {
        self.segments
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn iter(&self) -> impl Iterator<Item = (SegmentKey, &Reference)> {
        self.segments
            .iter()
            .flat_map(|(&key, references)| references.iter().map(move |r| (key, r)))
    }
}
