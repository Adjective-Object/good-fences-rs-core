use super::*;

fn name(decoded: &str, start: u64, end: u64) -> Box<wire::NameT> {
    let mut value = wire::NameT::default();
    value.decoded = decoded.to_owned();
    value.span = wire::SpanT { start, end };
    Box::new(value)
}

fn facts() -> wire::FactsT {
    let mut value = wire::FactsT::default();
    let mut selector = wire::SelectorNameT::default();
    selector.name = name("café", 1, 6);
    selector.local = true;
    value.names.push(selector);
    value
}

#[test]
fn typed_style_validation_rejects_discriminants_spans_and_payload_mismatches() {
    let source = ".café {}";
    let config = hash128(b"parser and module policy");
    let record = WireStyle::encode(source.as_bytes(), source, config, facts()).unwrap();
    assert!(record.backing().is_mapped());
    let backing = record.backing().clone();
    for (input, prepared, key) in [
        (b"wrong original".as_slice(), source, config),
        (source.as_bytes(), ".other {}", config),
        (source.as_bytes(), source, [0; 16]),
    ] {
        assert!(WireStyle::from_backing(backing.clone(), input, prepared, key).is_err());
    }
    let mut invalid = facts();
    invalid.names[0].kind = wire::SelectorKind(255);
    assert!(format!(
        "{:#}",
        WireStyle::encode(source.as_bytes(), source, config, invalid).unwrap_err()
    )
    .contains("selector kind"));
    let mut invalid = facts();
    invalid.names[0].name.span.end = 5;
    assert!(format!(
        "{:#}",
        WireStyle::encode(source.as_bytes(), source, config, invalid).unwrap_err()
    )
    .contains("UTF-8 span"));
    let mut invalid = facts();
    let mut animation = wire::AnimationT::default();
    animation.name = name("café", 1, 6);
    animation.span = wire::SpanT { start: 0, end: 7 };
    animation.local = wire::Locality(255);
    invalid.animations.push(animation);
    assert!(WireStyle::encode(source.as_bytes(), source, config, invalid).is_err());
    let mut invalid = facts();
    let mut composition = wire::CompositionT::default();
    composition.span = wire::SpanT { start: 0, end: 9 };
    composition.kind = wire::CompositionKind::File;
    composition.names.push(*name("café", 1, 6));
    invalid.compositions.push(composition);
    assert!(WireStyle::encode(source.as_bytes(), source, config, invalid).is_err());
    for bytes in [Vec::new(), vec![0; 16], {
        let mut bytes = record.backing().to_vec();
        bytes[12] ^= 1;
        bytes
    }] {
        assert!(WireStyle::from_backing(
            SharedBytes::snapshot(bytes.as_slice()).unwrap(),
            source.as_bytes(),
            source,
            config
        )
        .is_err());
    }
}

#[test]
fn style_facts_have_no_independent_resolution_byte_cap() {
    let value = "a".repeat(crate::MAX_RECORD_BYTES + 1);
    let source = format!("a{{background:url({value})}}");
    let mut facts = wire::FactsT::default();
    let mut url = wire::UrlT::default();
    url.span = wire::SpanT {
        start: 13,
        end: source.len() as u64 - 1,
    };
    url.value = value;
    url.function = true;
    facts.urls.push(url);
    let record = WireStyle::encode(source.as_bytes(), &source, [0; 16], facts).unwrap();
    assert!(record.backing().len() > crate::MAX_RECORD_BYTES);
    assert!(record.backing().is_mapped());
    let pointer = record.facts().urls().get(0).value().as_ptr();
    let clone = record.clone();
    drop(record);
    assert_eq!(clone.facts().urls().get(0).value().as_ptr(), pointer);
}
