use super::*;

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(".bundler-type-wire-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap()
}

fn facts() -> wire::FactsT {
    let name = wire::NameT {
        value: "T".into(),
        span: wire::SpanT { start: 5, end: 6 },
    };
    let local = wire::LocalT {
        name: Box::new(name),
        span: wire::SpanT { start: 0, end: 14 },
        presence: wire::Presence::OnlyType,
        has_type: true,
        ..Default::default()
    };
    let mut facts = wire::FactsT::default();
    facts.locals.push(local);
    facts
}

#[test]
fn checked_original_source_settings_and_corruption() {
    let dir = directory();
    let source = "type T = null;";
    let record = WireTypeOnlyFacts::encode_in(dir.path(), source, [3; 16], facts()).unwrap();
    assert!(record.backing().is_mapped());
    assert_eq!(record.facts().locals().get(0).name().value(), "T");
    for (text, config) in [("type X = null;", [3; 16]), (source, [4; 16])] {
        assert!(WireTypeOnlyFacts::from_backing(record.backing().clone(), text, config).is_err());
    }
    for index in [0, 4, record.backing().len() / 2, record.backing().len() - 1] {
        let mut bytes = record.backing().to_vec();
        bytes[index] ^= 0x40;
        assert!(WireTypeOnlyFacts::from_backing(
            SharedBytes::snapshot_in(dir.path(), bytes.as_slice()).unwrap(),
            source,
            [3; 16]
        )
        .is_err());
    }
    // A valid framing checksum does not excuse a malformed inner table.
    let mut bytes = record.backing().to_vec();
    bytes[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    let end = bytes.len() - CHECKSUM_BYTES;
    let checksum = hash128(&bytes[..end]);
    bytes[end..].copy_from_slice(&checksum);
    assert!(WireTypeOnlyFacts::from_backing(
        SharedBytes::snapshot_in(dir.path(), bytes.as_slice()).unwrap(),
        source,
        [3; 16]
    )
    .is_err());
}

#[test]
fn previous_extractor_cannot_replay_merged_import_use_facts() {
    let dir = directory();
    let source = "type T = null;";
    let record = WireTypeOnlyFacts::encode_in(dir.path(), source, [3; 16], facts()).unwrap();
    let root = flatbuffers::root::<wire::TypeOnlyFacts<'_>>(
        &record.backing()[..record.backing().len() - CHECKSUM_BYTES],
    )
    .unwrap();
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let facts = builder.create_vector(root.facts().bytes());
    let previous = wire::TypeOnlyFacts::create(
        &mut builder,
        &wire::TypeOnlyFactsArgs {
            schema: SCHEMA,
            extractor: 1,
            source_hash: Some(root.source_hash()),
            config_hash: Some(root.config_hash()),
            records_hash: Some(root.records_hash()),
            facts: Some(facts),
        },
    );
    wire::finish_type_only_facts_buffer(&mut builder, previous);
    let mut bytes = builder.finished_data().to_vec();
    bytes.extend_from_slice(&hash128(builder.finished_data()));
    let error = WireTypeOnlyFacts::from_backing(
        SharedBytes::snapshot_in(dir.path(), bytes.as_slice()).unwrap(),
        source,
        [3; 16],
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("TypeOnlyFacts version mismatch"));
}

#[test]
fn semantic_validation_rejects_bad_discriminants_spans_and_aliases() {
    let dir = directory();
    let source = "type T = null;";
    let mut invalid = facts();
    invalid.locals[0].presence = wire::Presence(255);
    assert!(format!(
        "{:#}",
        WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).unwrap_err()
    )
    .contains("unknown TypeOnlyFacts presence"));
    let mut invalid = facts();
    invalid.locals[0].name.span.end = 40;
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).is_err());
    let mut invalid = facts();
    invalid.locals[0].name.span = wire::SpanT { start: 1, end: 2 };
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), "éype T = null;", [0; 16], invalid).is_err());
    let mut invalid = facts();
    let mut export = wire::ExportT::default();
    export.name = invalid.locals[0].name.clone();
    export.local = Some(export.name.clone());
    export.span = invalid.locals[0].span.clone();
    export.presence = wire::Presence::RuntimeValue;
    invalid.exports.push(export);
    assert!(format!(
        "{:#}",
        WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).unwrap_err()
    )
    .contains("disagrees with local"));
    let mut invalid = facts();
    let occurrence = wire::OccurrenceT {
        span: wire::SpanT { start: 0, end: 14 },
        kind: wire::OccurrenceKind::Import,
        ..Default::default()
    };
    invalid.occurrences.push(occurrence);
    assert!(format!(
        "{:#}",
        WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).unwrap_err()
    )
    .contains("missing TypeOnlyFacts request"));
}

#[test]
fn enum_payload_and_diagnostic_links_are_checked_before_borrowing() {
    let dir = directory();
    let source = "const enum E { A = 1 }";
    let name = wire::NameT {
        value: "E".into(),
        span: wire::SpanT { start: 11, end: 12 },
    };
    let member = wire::MemberT {
        name: Box::new(wire::NameT {
            value: "A".into(),
            span: wire::SpanT { start: 15, end: 16 },
        }),
        span: wire::SpanT { start: 15, end: 20 },
        initializer: Some(wire::SpanT { start: 19, end: 20 }),
        kind: wire::ConstantKind::Number,
        number: Some(-0.0),
        ..Default::default()
    };
    let facts = wire::FactsT {
        locals: vec![wire::LocalT {
            name: Box::new(name.clone()),
            span: wire::SpanT { start: 0, end: 22 },
            presence: wire::Presence::ConstEnum,
            has_type: true,
            ..Default::default()
        }],
        const_enums: vec![wire::ConstEnumT {
            name: Box::new(name),
            span: wire::SpanT { start: 0, end: 22 },
            members: vec![member],
            ..Default::default()
        }],
        ..Default::default()
    };
    let valid = WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], facts.clone()).unwrap();
    assert_eq!(
        valid
            .facts()
            .const_enums()
            .get(0)
            .members()
            .get(0)
            .number()
            .unwrap()
            .to_bits(),
        (-0.0f64).to_bits()
    );
    for kind in [
        wire::ConstantKind(255),
        wire::ConstantKind::Unresolved,
        wire::ConstantKind::String,
    ] {
        let mut invalid = facts.clone();
        invalid.const_enums[0].members[0].kind = kind;
        assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).is_err());
    }
    let mut invalid = facts.clone();
    invalid.const_enums[0].members[0].number = Some(f64::INFINITY);
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).is_err());
    let mut invalid = facts.clone();
    invalid.locals.clear();
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], invalid).is_err());
    let mut unresolved = facts.clone();
    unresolved.const_enums[0].members[0].kind = wire::ConstantKind::Unresolved;
    unresolved.const_enums[0].members[0].number = None;
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], unresolved.clone()).is_err());
    unresolved.diagnostics.push(wire::DiagnosticT {
        kind: wire::DiagnosticKind::CyclicConstant,
        span: wire::SpanT { start: 15, end: 20 },
        message: "same-file cycle".into(),
    });
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], unresolved.clone()).is_ok());
    unresolved.diagnostics[0].kind = wire::DiagnosticKind(255);
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), source, [0; 16], unresolved).is_err());
}

#[test]
fn expanded_enum_strings_are_bounded_on_construction_and_checked_cache_hits() {
    let dir = directory();
    let make = |count: usize, length: usize| {
        let mut source = String::from("const enum E {");
        let mut members = Vec::new();
        for i in 0..count {
            let start = source.len() as u32 + 1;
            let text = format!("M{i}");
            source.push_str(&format!(" {text}=0,"));
            let end = source.len() as u32 - 1;
            members.push(wire::MemberT {
                name: Box::new(wire::NameT {
                    value: text.clone(),
                    span: wire::SpanT {
                        start,
                        end: start + text.len() as u32,
                    },
                }),
                span: wire::SpanT { start, end },
                initializer: Some(wire::SpanT {
                    start: end - 1,
                    end,
                }),
                kind: wire::ConstantKind::String,
                string: Some("x".repeat(length)),
                ..Default::default()
            });
        }
        source.push_str(" }");
        let name = Box::new(wire::NameT {
            value: "E".into(),
            span: wire::SpanT { start: 11, end: 12 },
        });
        let owner = wire::SpanT {
            start: 0,
            end: source.len() as u32,
        };
        let facts = wire::FactsT {
            locals: vec![wire::LocalT {
                name: name.clone(),
                span: owner.clone(),
                presence: wire::Presence::ConstEnum,
                has_type: true,
                ..Default::default()
            }],
            const_enums: vec![wire::ConstEnumT {
                name,
                span: owner,
                members,
                ..Default::default()
            }],
            ..Default::default()
        };
        (source, facts)
    };
    for (count, length, reason) in [
        (1, TYPE_ONLY_STRING_VALUE_BYTES + 1, "per-value"),
        (
            TYPE_ONLY_STRING_TOTAL_BYTES / TYPE_ONLY_STRING_VALUE_BYTES + 1,
            TYPE_ONLY_STRING_VALUE_BYTES,
            "aggregate",
        ),
    ] {
        let (source, facts) = make(count, length);
        assert!(format!(
            "{:#}",
            WireTypeOnlyFacts::encode_in(dir.path(), &source, [0; 16], facts.clone()).unwrap_err()
        )
        .contains(reason));

        // Simulate an independently written cache with valid framing/integrity,
        // bypassing only construction's pre-allocation semantic limit checks.
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let inner = facts.pack(&mut builder);
        builder.finish(inner, None);
        let records = builder.finished_data();
        let mut outer = flatbuffers::FlatBufferBuilder::new();
        let data = outer.create_vector(records);
        let root = wire::TypeOnlyFacts::create(
            &mut outer,
            &wire::TypeOnlyFactsArgs {
                schema: SCHEMA,
                extractor: EXTRACTOR,
                source_hash: Some(&wire::Hash128::new(&hash128(source.as_bytes()))),
                config_hash: Some(&wire::Hash128::new(&[0; 16])),
                records_hash: Some(&wire::Hash128::new(&hash128(records))),
                facts: Some(data),
            },
        );
        wire::finish_type_only_facts_buffer(&mut outer, root);
        let mut bytes = outer.finished_data().to_vec();
        bytes.extend_from_slice(&hash128(outer.finished_data()));
        let error = WireTypeOnlyFacts::from_backing(
            SharedBytes::snapshot_in(dir.path(), bytes.as_slice()).unwrap(),
            &source,
            [0; 16],
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains(reason));
    }
    let (source, facts) = make(
        TYPE_ONLY_STRING_TOTAL_BYTES / TYPE_ONLY_STRING_VALUE_BYTES,
        TYPE_ONLY_STRING_VALUE_BYTES,
    );
    assert!(WireTypeOnlyFacts::encode_in(dir.path(), &source, [0; 16], facts).is_ok());
}
