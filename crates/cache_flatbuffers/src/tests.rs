use super::{wire as w, *};

mod shapes;

const SOURCE: &str = "export const message = 'hello';";
const OXC: &str = "test-parser-identity";

fn module() -> w::CachedModuleT {
    w::CachedModuleT {
        body: w::ModuleBodyT::EsmBody(Box::default()),
        facts: Box::new(w::ModuleFactsT {
            ecco: Some(Box::default()),
            bindings: vec![w::BindingT {
                id: 0,
                name: "message".into(),
                declarations: vec![w::SpanT { start: 0, end: 7 }],
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn record(module: &w::CachedModuleT) -> w::CacheRecordT {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let root = module.pack(&mut builder);
    builder.finish(root, Some(MODULE_IDENTIFIER));
    let module = builder.finished_data().to_vec();
    let source_hash = w::Sha256T {
        bytes: Sha256::digest(SOURCE.as_bytes()).into(),
    };
    w::CacheRecordT {
        schema: SCHEMA_VERSION,
        extractor: EXTRACTOR_VERSION,
        oxc: OXC.into(),
        source_len: SOURCE.len() as u64,
        source_hash: source_hash.clone(),
        provenance: Box::new(w::SourceProvenanceT {
            source_hash,
            records_hash: w::Sha256T {
                bytes: Sha256::digest(&module).into(),
            },
            ..Default::default()
        }),
        module,
        ..Default::default()
    }
}

fn payload(record: &w::CacheRecordT) -> Vec<u8> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let root = record.pack(&mut builder);
    w::finish_size_prefixed_cache_record_buffer(&mut builder, root);
    builder.finished_data().to_vec()
}

fn checksum(payload: &[u8]) -> Vec<u8> {
    let mut bytes = payload.to_vec();
    bytes.extend_from_slice(&Sha256::digest(payload));
    bytes
}

fn load(bytes: Vec<u8>) -> Result<WireRecord> {
    WireRecord::new(bytes.into(), SOURCE, w::ParserConfig::default(), OXC)
}

#[test]
fn reads_borrow_the_shared_buffer_including_nested_strings() {
    let bytes = frame(&payload(&record(&module()))).unwrap();
    let owner = load(bytes).unwrap();
    let start = owner.bytes().as_ptr() as usize;
    let allocation = start..start + owner.bytes().len();
    let name = owner.module().facts().bindings().get(0).name();
    assert_eq!(name, "message");
    assert!(allocation.contains(&(name.as_ptr() as usize)));
    assert!(allocation.contains(&(owner.root().oxc().as_ptr() as usize)));
    let pointer = name.as_ptr() as usize;
    let clone = owner.clone();
    std::thread::spawn(move || {
        assert_eq!(
            clone.module().facts().bindings().get(0).name().as_ptr() as usize,
            pointer
        );
    })
    .join()
    .unwrap();
    assert_eq!(owner.module().facts().bindings().get(0).id(), 0);
}

#[test]
fn optional_zero_ids_remain_distinct_from_absence() {
    let mut module = module();
    module.facts.references = vec![
        w::ReferenceT {
            name: "present".into(),
            binding: Some(0),
            ..Default::default()
        },
        w::ReferenceT {
            name: "absent".into(),
            binding: None,
            ..Default::default()
        },
    ];
    module.facts.emission.execution.references = vec![
        w::OptionalIndexT { value: Some(0) },
        w::OptionalIndexT { value: None },
    ];
    let loaded = load(frame(&payload(&record(&module))).unwrap()).unwrap();
    let facts = loaded.module().facts();
    assert_eq!(facts.references().get(0).binding(), Some(0));
    assert_eq!(facts.references().get(1).binding(), None);
    assert_eq!(
        facts.emission().execution().references().get(0).value(),
        Some(0)
    );
    assert_eq!(
        facts.emission().execution().references().get(1).value(),
        None
    );
}

#[test]
fn rejects_stale_metadata_and_source_or_seal_repairing() {
    let original = record(&module());
    for mutate in [
        |r: &mut w::CacheRecordT| r.schema -= 1,
        |r: &mut w::CacheRecordT| r.extractor -= 1,
        |r: &mut w::CacheRecordT| r.oxc.push('x'),
        |r: &mut w::CacheRecordT| r.source_len += 1,
        |r: &mut w::CacheRecordT| r.source_hash.bytes[0] ^= 1,
        |r: &mut w::CacheRecordT| r.provenance.source_hash.bytes[0] ^= 1,
        |r: &mut w::CacheRecordT| r.provenance.records_hash.bytes[0] ^= 1,
        |r: &mut w::CacheRecordT| r.config.jsx = true,
        |r: &mut w::CacheRecordT| r.provenance.config.jsx = true,
    ] {
        let mut record = original.clone();
        mutate(&mut record);
        assert!(load(checksum(&payload(&record))).is_err());
    }
    let bytes: Arc<[u8]> = frame(&payload(&original)).unwrap().into();
    assert!(WireRecord::new(bytes, "different source", w::ParserConfig::default(), OXC).is_err());
}

#[test]
fn rejects_truncation_oversize_legacy_data_and_bad_roots() {
    let payload = payload(&record(&module()));
    let bytes = frame(&payload).unwrap();
    for length in 0..bytes.len() {
        assert!(load(bytes[..length].to_vec()).is_err(), "{length}");
    }
    assert!(load(vec![0; MAX_RECORD_BYTES + 1]).is_err());
    assert!(load(checksum(&[0xa1, 0x61, b'x', 0x01])).is_err());
    for (offset, value) in [(0, u32::MAX), (4, u32::MAX), (8, 0)] {
        let mut damaged = payload.clone();
        damaged[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(load(checksum(&damaged)).is_err());
    }
    let mut trailing = payload.clone();
    trailing.push(0);
    assert!(load(checksum(&trailing)).is_err());
    let mut bad_checksum = bytes;
    *bad_checksum.last_mut().unwrap() ^= 1;
    assert!(load(bad_checksum).is_err());
}

#[test]
fn independently_verifies_nested_roots_and_vector_lengths() {
    let original = record(&module());
    for offset in [0, 4] {
        let mut record = original.clone();
        record.module[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        record.provenance.records_hash.bytes = Sha256::digest(&record.module).into();
        assert!(load(checksum(&payload(&record))).is_err());
    }
    let mut bytes = payload(&original);
    let root = verified_root(&bytes).unwrap();
    let vector = root.module().bytes();
    let length_offset = vector.as_ptr() as usize - bytes.as_ptr() as usize - 4;
    bytes[length_offset..length_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(load(checksum(&bytes)).is_err());
}

#[test]
fn rejects_unknown_scalar_enums_in_nested_records() {
    for mutate in [
        |m: &mut w::CachedModuleT| {
            m.facts.effects.push(w::EffectSiteT {
                reason: w::EffectReason(255),
                ..Default::default()
            });
        },
        |m: &mut w::CachedModuleT| {
            m.facts.references.push(w::ReferenceT {
                commonjs: Some(w::CommonJsGlobal(255)),
                ..Default::default()
            });
        },
        |m: &mut w::CachedModuleT| {
            m.facts.dependencies.push(w::DependencyT {
                kind: w::DependencyKind(255),
                ..Default::default()
            });
        },
        |m: &mut w::CachedModuleT| {
            m.facts
                .emission
                .execution
                .contexts
                .push(w::ExecutionContextT {
                    kind: w::ContextKind(255),
                    ..Default::default()
                });
        },
    ] {
        let mut module = module();
        mutate(&mut module);
        let payload = payload(&record(&module));
        assert!(frame(&payload).is_err());
        assert!(load(checksum(&payload)).is_err());
    }
}

#[test]
fn rejects_unknown_and_missing_union_discriminants() {
    let original = record(&module());
    for discriminator in [0, 255] {
        let mut record = original.clone();
        let root = verified_module(&record.module).unwrap();
        // Access to the vtable is bounded by prior generated verification.
        let field =
            root._tab.loc() + root._tab.vtable().get(w::CachedModule::VT_BODY_TYPE) as usize;
        record.module[field] = discriminator;
        record.provenance.records_hash.bytes = Sha256::digest(&record.module).into();
        assert!(load(checksum(&payload(&record))).is_err());
    }
}

#[test]
fn lazy_selector_kind_discriminants_are_checked_before_borrowed_access() {
    for kind in [
        w::LazyWrapperKind::Unknown,
        w::LazyWrapperKind::LazyImport,
        w::LazyWrapperKind::LazyAction,
        w::LazyWrapperKind(255),
    ] {
        let mut module = module();
        module
            .facts
            .emission
            .lazy_selectors
            .push(w::LazySelectorCandidateT {
                kind,
                property: "selected".into(),
                ..Default::default()
            });
        let bytes = checksum(&payload(&record(&module)));
        assert_eq!(load(bytes).is_ok(), kind.variant_name().is_some());
    }
}

fn unknown_expression() -> w::HazardExprT {
    w::HazardExprT {
        value: w::HazardExprValueT::UnknownHazard(Box::default()),
    }
}

fn definition(expression: w::HazardExprT) -> w::DefinitionT {
    w::DefinitionT {
        binding: Box::new(w::EccoBindingT {
            value: w::EccoBindingValueT::GlobalBinding(Box::new(w::GlobalBindingT {
                path: vec!["global".into()],
            })),
        }),
        hazards: Box::new(w::HazardFieldsT {
            default_expression: Some(Box::new(expression)),
            ..Default::default()
        }),
        returns: Box::new(w::ReturnsT {
            value: w::ReturnsValueT::FreshReturn(Box::default()),
        }),
        ..Default::default()
    }
}

#[test]
fn bounds_recursive_contract_verification_before_traversal() {
    let mut module = module();
    let mut expression = unknown_expression();
    for _ in 0..100 {
        expression = w::HazardExprT {
            value: w::HazardExprValueT::OrHazard(Box::new(w::OrHazardT {
                expressions: vec![expression],
            })),
        };
    }
    module.facts.ecco.as_mut().unwrap().facts.definitions = vec![definition(expression)];
    assert!(load(checksum(&payload(&record(&module)))).is_err());
}

#[test]
fn rejects_duplicate_or_unordered_effect_source_map_entries() {
    for keys in [[2, 2], [2, 1]] {
        let mut module = module();
        module.facts.ecco.as_mut().unwrap().setup.sources = keys
            .into_iter()
            .map(|effect| w::EffectSourceT {
                effect,
                value: 0,
                hazard: w::Hazard::Call,
            })
            .collect();
        assert!(load(checksum(&payload(&record(&module)))).is_err());
    }
}
