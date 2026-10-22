use super::*;

fn encode(tokens: &[wire::MapToken]) -> Result<WireSourceMap> {
    WireSourceMap::encode(
        "compiled",
        b"json",
        MapData {
            location: "file:///project/maps/input.map",
            file: Some("compiled.mjs"),
            source_root: Some("../source"),
            sources: &[MapSource {
                original: "input.ts",
                url: "file:///project/source/input.ts",
                content: None,
            }],
            names: &["originalName"],
            tokens,
        },
    )
}

#[test]
fn upstream_map_borrows_metadata_and_preserves_sparse_boundaries() {
    let map = encode(&[
        wire::MapToken::new(0, 2, 8, 4, 0, 0),
        wire::MapToken::new(0, 7, 0, 0, -1, -1),
        wire::MapToken::new(2, 1, 9, 0, 0, -1),
    ])
    .unwrap();
    let root = map.root();
    let allocation =
        map.bytes().as_ptr() as usize..map.bytes().as_ptr() as usize + map.bytes().len();
    for text in [
        root.location(),
        root.source_root().unwrap(),
        root.sources().get(0).url(),
        root.names().get(0),
    ] {
        assert!(allocation.contains(&(text.as_ptr() as usize)));
    }
    assert!(root.sources().get(0).content().is_none());
    assert!(map.lookup(0, 0).is_none());
    assert_eq!(map.lookup(0, 3).unwrap().original_line(), 8);
    assert_eq!(map.lookup(0, 8).unwrap().source(), -1);
    assert!(map.lookup(1, 10).is_none());
    let clone = map.clone();
    assert_eq!(
        root.names().get(0).as_ptr(),
        clone.root().names().get(0).as_ptr()
    );
}

#[test]
fn upstream_map_rejects_corruption_stale_inputs_and_invalid_semantic_links() {
    let map = encode(&[wire::MapToken::new(0, 0, 0, 0, 0, 0)]).unwrap();
    for (source, input, location) in [
        ("changed", &b"json"[..], "file:///project/maps/input.map"),
        (
            "compiled",
            &b"changed"[..],
            "file:///project/maps/input.map",
        ),
        ("compiled", &b"json"[..], "file:///moved/input.map"),
    ] {
        assert!(WireSourceMap::new(Arc::from(map.bytes()), source, input, location).is_err());
    }
    let mut bytes = map.bytes().to_vec();
    bytes[10] ^= 0xff;
    assert!(WireSourceMap::new(
        bytes.into(),
        "compiled",
        b"json",
        "file:///project/maps/input.map"
    )
    .is_err());
    assert!(encode(&[wire::MapToken::new(0, 0, 0, 0, 1, 0)]).is_err());
    assert!(encode(&[wire::MapToken::new(0, 0, 0, 0, -1, 0)]).is_err());
    assert!(encode(&[
        wire::MapToken::new(1, 0, 0, 0, 0, -1),
        wire::MapToken::new(0, 0, 0, 0, 0, -1),
    ])
    .is_err());
}
