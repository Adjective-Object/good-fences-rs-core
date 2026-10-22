use super::*;

const KEY: Key = Key {
    policy: [1; 32],
    instance: [2; 32],
    dependencies: [3; 32],
};

fn locator(path: &str) -> Box<wire::LocatorT> {
    Box::new(wire::LocatorT {
        path: path.into(),
        ..Default::default()
    })
}

fn data() -> wire::ResolutionT {
    wire::ResolutionT {
        schema: SCHEMA_VERSION,
        policy: wire::DigestT { bytes: KEY.policy },
        instance: wire::DigestT {
            bytes: KEY.instance,
        },
        dependencies: wire::DigestT {
            bytes: KEY.dependencies,
        },
        requests: vec![wire::RequestT {
            mode: wire::Mode::Import,
            importer: "src/main.js".into(),
            specifier: "./target".into(),
            target: Box::new(wire::TargetT {
                kind: wire::TargetKind::File,
                file: Some(locator("src/target.js")),
                external: None,
            }),
            inputs: vec![
                wire::InputT {
                    path: locator("src/target"),
                    ..Default::default()
                },
                wire::InputT {
                    path: locator("src/target.js"),
                    kind: wire::Kind::File,
                    canonical: Some(locator("src/target.js")),
                    identity: Some(wire::IdentityT {
                        volume: 10,
                        file: 20,
                    }),
                    content: Some(wire::DigestT { bytes: [7; 32] }),
                    links: vec![],
                },
            ],
        }],
    }
}

fn rechecksum(bytes: &mut [u8]) {
    let end = bytes.len() - 32;
    let checksum = Sha256::digest(&bytes[..end]);
    bytes[end..].copy_from_slice(&checksum);
}

#[test]
fn borrowed_positive_negative_roundtrip_and_shared_ownership() {
    let record = WireResolution::encode(&data(), KEY).unwrap();
    let shared = record.clone();
    let request = record.root().requests().get(0);
    let pointer = request.specifier().as_ptr() as usize;
    let allocation = record.bytes().as_ptr() as usize;
    assert!((allocation..allocation + record.bytes().len()).contains(&pointer));
    assert_eq!(request.inputs().get(0).kind(), wire::Kind::Missing);
    assert_eq!(request.inputs().get(1).identity().unwrap().file(), 20);
    drop(record);
    std::thread::spawn(move || {
        assert_eq!(
            shared.root().requests().get(0).specifier().as_ptr() as usize,
            pointer
        );
        assert_eq!(shared.root().requests().get(0).specifier(), "./target");
    })
    .join()
    .unwrap();
}

#[test]
fn corruption_truncation_framing_and_structure_are_rejected() {
    let record = WireResolution::encode(&data(), KEY).unwrap();
    for length in [0, 11, 43, record.bytes().len() - 1] {
        assert!(WireResolution::new(record.bytes()[..length].into(), KEY).is_err());
    }
    let mut corrupt = record.bytes().to_vec();
    corrupt[16] ^= 1;
    assert!(WireResolution::new(corrupt.into(), KEY)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    for offset in [0, 4, 8] {
        let mut corrupt = record.bytes().to_vec();
        corrupt[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        rechecksum(&mut corrupt);
        assert!(WireResolution::new(corrupt.into(), KEY).is_err());
    }
    let oversized = vec![0; MAX_RECORD_BYTES + 1];
    assert!(WireResolution::new(oversized.into(), KEY).is_err());
}

#[test]
fn independent_version_and_each_fingerprint_are_checked() {
    let record = WireResolution::encode(&data(), KEY).unwrap();
    for key in [
        Key {
            policy: [4; 32],
            ..KEY
        },
        Key {
            instance: [4; 32],
            ..KEY
        },
        Key {
            dependencies: [4; 32],
            ..KEY
        },
    ] {
        assert!(WireResolution::new(record.bytes().into(), key)
            .unwrap_err()
            .to_string()
            .contains("fingerprint"));
    }
    let mut changed = data();
    changed.schema += 1;
    assert!(WireResolution::encode(&changed, KEY)
        .unwrap_err()
        .to_string()
        .contains("version"));
}

#[test]
fn unknown_discriminants_and_invalid_payloads_are_rejected() {
    for field in 0..5 {
        let mut data = data();
        match field {
            0 => data.requests[0].mode = wire::Mode(255),
            1 => data.requests[0].target.kind = wire::TargetKind(255),
            2 => data.requests[0].inputs[0].kind = wire::Kind(255),
            3 => data.requests[0].inputs[0].path.anchor = wire::Anchor(255),
            _ => data.requests[0].target.external = Some("node:fs".into()),
        }
        assert!(WireResolution::encode(&data, KEY).is_err(), "field {field}");
    }
    let mut data = data();
    data.requests[0].inputs[0].identity = Some(wire::IdentityT { volume: 0, file: 0 });
    assert!(WireResolution::encode(&data, KEY).is_err());
}

#[test]
fn unsafe_paths_duplicate_and_unordered_requests_are_rejected() {
    for path in [
        "../x",
        "a/../x",
        "/x",
        "C:/x",
        "\\\\server\\share",
        "a\\b",
        "a//b",
        "a/./b",
        "NUL.js",
        "COM1",
        "a.",
        "a ",
        "a\u{0}b",
        "",
    ] {
        let mut data = data();
        data.requests[0].importer = path.into();
        assert!(WireResolution::encode(&data, KEY).is_err(), "{path:?}");
    }
    let mut duplicate = data();
    duplicate.requests.push(duplicate.requests[0].clone());
    assert!(WireResolution::encode(&duplicate, KEY).is_err());
    duplicate.requests[0].specifier = "z".into();
    duplicate.requests[1].specifier = "a".into();
    assert!(WireResolution::encode(&duplicate, KEY).is_err());
    let mut external = data();
    external.requests[0].inputs[0].path.parents = 1;
    external.requests[0].inputs[0].path.path = "Users/machine/private".into();
    assert!(WireResolution::encode(&external, KEY).is_err());
    for specifier in [
        "/absolute.js",
        "C:\\absolute.js",
        "\\\\server\\share\\x",
        "file:///x",
    ] {
        let mut data = data();
        data.requests[0].specifier = specifier.into();
        assert!(WireResolution::encode(&data, KEY).is_err(), "{specifier:?}");
    }
    let mut proof = data();
    proof.requests[0].inputs[0].path.path = "../unsafe".into();
    assert!(WireResolution::encode(&proof, KEY).is_err());
}

#[test]
fn verifier_table_budget_is_applied_before_semantic_traversal() {
    let data = data();
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let request = data.requests[0].pack(&mut builder);
    let requests = builder.create_vector(&vec![request; 262_145]);
    let policy = wire::Digest::new(&KEY.policy);
    let instance = wire::Digest::new(&KEY.instance);
    let dependencies = wire::Digest::new(&KEY.dependencies);
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
    assert!(bytes.len() < MAX_RECORD_BYTES);
    let error = WireResolution::new(bytes.into(), KEY).unwrap_err();
    assert!(error.to_string().contains("Too many tables"), "{error:?}");
}
