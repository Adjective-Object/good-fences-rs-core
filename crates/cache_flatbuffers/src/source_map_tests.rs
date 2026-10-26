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
fn upstream_map_accepts_shared_private_mapped_backing() {
    let directory = tempfile::tempdir_in(".").unwrap();
    let encoded = encode(&[wire::MapToken::new(0, 2, 8, 4, 0, 0)]).unwrap();
    let backing = SharedBytes::snapshot_in(directory.path(), encoded.bytes()).unwrap();
    let map = WireSourceMap::from_backing(
        backing,
        "compiled",
        b"json",
        "file:///project/maps/input.map",
    )
    .unwrap();
    assert!(map.backing().is_mapped());
    assert_eq!(map, encoded);
    let pointer = map.root().names().get(0).as_ptr();
    let clone = map.clone();
    drop(map);
    drop(encoded);
    assert_eq!(clone.root().names().get(0).as_ptr(), pointer);
    assert_eq!(clone.lookup(0, 3).unwrap().original_line(), 8);
    drop(clone);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    directory.close().unwrap();
}

#[test]
fn snapshot_keeps_embedded_contents_in_shared_private_backing() {
    let map = WireSourceMap::encode(
        "compiled",
        b"json",
        MapData {
            location: "file:///project/input.map",
            file: None,
            source_root: None,
            sources: &[MapSource {
                original: "input.ts",
                url: "file:///project/input.ts",
                content: Some("export const value: number = 42;"),
            }],
            names: &[],
            tokens: &[wire::MapToken::new(0, 0, 0, 0, 0, -1)],
        },
    )
    .unwrap()
    .snapshot()
    .unwrap();
    assert!(map.backing().is_mapped());
    let content = map.root().sources().get(0).content().unwrap();
    let pointer = content.as_ptr();
    let allocation =
        map.bytes().as_ptr() as usize..map.bytes().as_ptr() as usize + map.bytes().len();
    assert!(allocation.contains(&(pointer as usize)));
    let clone = map.clone().snapshot().unwrap();
    drop(map);
    assert_eq!(
        clone.root().sources().get(0).content().unwrap().as_ptr(),
        pointer
    );
    assert_eq!(
        clone.root().sources().get(0).content(),
        Some("export const value: number = 42;")
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

#[test]
fn upstream_map_rejects_previous_identity_with_valid_new_checksum() {
    let map = encode(&[wire::MapToken::new(0, 0, 0, 0, 0, 0)]).unwrap();
    let end = map.bytes().len() - CHECKSUM_BYTES;
    assert_eq!(&map.bytes()[end..], &hash128(&map.bytes()[..end]));
    assert!(map
        .root()
        .source_hash()
        .bytes()
        .iter()
        .eq(hash128(b"compiled")));
    assert!(map.root().input_hash().bytes().iter().eq(hash128(b"json")));
    let root = map.root();
    for (slot, previous) in [
        (wire::UpstreamMap::VT_SCHEMA, 19_u32),
        (wire::UpstreamMap::VT_EXTRACTOR, 32_u32),
    ] {
        let offset = root._tab.loc() + usize::from(root._tab.vtable().get(slot));
        let mut bytes = map.bytes().to_vec();
        bytes[offset..offset + 4].copy_from_slice(&previous.to_le_bytes());
        let checksum = hash128(&bytes[..end]);
        bytes[end..].copy_from_slice(&checksum);
        let error = WireSourceMap::new(
            bytes.into(),
            "compiled",
            b"json",
            "file:///project/maps/input.map",
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("version"));
    }
    for index in 0..CHECKSUM_BYTES {
        let mut bytes = map.bytes().to_vec();
        bytes[end + index] ^= 1;
        let error = WireSourceMap::new(
            bytes.into(),
            "compiled",
            b"json",
            "file:///project/maps/input.map",
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("checksum"));
    }
}
