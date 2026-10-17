use super::*;

fn record(source: &str, path: &str, format: ModuleFormat) -> CacheRecord {
    let parsed =
        ast_segmenter::parse_module(&logger::StdioLogger::new(), source, Path::new(path), format)
            .unwrap();
    CacheRecord::from_parsed(source, &parsed).unwrap()
}

fn envelope(record: &CacheRecord) -> Vec<u8> {
    let payload = serde_cbor::to_vec(record).unwrap();
    serde_cbor::to_vec(&Envelope {
        checksum: digest(&payload),
        payload,
    })
    .unwrap()
}

#[test]
fn deterministic_roundtrip_after_parser_arena_drop() {
    for (path, source, format) in [
        ("file.ts", "#!/bin/node\n'use strict'; import {x as y} from './dep'; export const café = y; export {café as 'é'}; import('./lazy');", ModuleFormat::Esm),
        ("file.cjs", "#!/bin/node\n'use strict'; module.exports = require('./dep'); function later(){ return require('./dep'); }", ModuleFormat::CommonJs),
        ("file.d.ts", "export interface A { value: string }", ModuleFormat::Esm),
        ("empty.cjs", "", ModuleFormat::CommonJs),
        ("file.js", "export * from './a'; export * as ns from './b'; export default 1;", ModuleFormat::Esm),
    ] {
        let first = record(source, path, format);
        let bytes = first.encode(source).unwrap();
        assert_eq!(bytes, record(source, path, format).encode(source).unwrap());
        let decoded = CacheRecord::decode(&bytes, source, first.config).unwrap();
        assert_eq!(first, decoded);
        assert_eq!(bytes, decoded.encode(source).unwrap());
    }
}

#[test]
fn rejects_versions_config_source_and_corruption() {
    let source = "export const café = 1;";
    let original = record(source, "file.js", ModuleFormat::Esm);
    for mutate in [
        |r: &mut CacheRecord| r.schema += 1,
        |r: &mut CacheRecord| r.extractor += 1,
        |r: &mut CacheRecord| r.oxc.push('x'),
        |r: &mut CacheRecord| r.magic.clear(),
        |r: &mut CacheRecord| r.source_len += 1,
        |r: &mut CacheRecord| r.source_hash[0] ^= 1,
        |r: &mut CacheRecord| r.config.jsx = true,
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
    let bytes = original.encode(source).unwrap();
    assert!(CacheRecord::decode(&bytes, "export const café = 2;", original.config).is_err());
    for i in 0..bytes.len() {
        assert!(CacheRecord::decode(&bytes[..i], source, original.config).is_err());
        let mut corrupt = bytes.clone();
        corrupt[i] ^= 1;
        assert!(
            CacheRecord::decode(&corrupt, source, original.config).is_err(),
            "byte {i}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(CacheRecord::decode(&trailing, source, original.config).is_err());
}

#[test]
fn rejects_impossible_spans_ids_and_order() {
    let source = "export const café = 1; console.log(café);";
    let original = record(source, "file.js", ModuleFormat::Esm);
    for mutate in [
        |r: &mut CacheRecord| r.module.segments[0] = Span::new(2, 1),
        |r: &mut CacheRecord| r.module.segments[0].end = u32::MAX,
        |r: &mut CacheRecord| r.module.facts.bindings[0].declarations[0].end = 17,
        |r: &mut CacheRecord| r.module.facts.bindings[0].id.0 += 1,
        |r: &mut CacheRecord| {
            r.module.facts.references[0].binding = Some(ast_segmenter::owned::BindingId(999))
        },
        |r: &mut CacheRecord| r.module.segments.reverse(),
    ] {
        let mut modified = original.clone();
        mutate(&mut modified);
        assert!(CacheRecord::decode(&envelope(&modified), source, original.config).is_err());
    }
}

#[test]
fn bounded_decoder_rejects_allocation_and_depth_bombs() {
    for bytes in [
        vec![0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x9f, 0xff],
        vec![0x81; 40],
        vec![0; MAX_RECORD_BYTES + 1],
        [vec![0x9a, 0, 4, 0, 0], vec![0; 262_144]].concat(),
    ] {
        assert!(bounded::check(&bytes).is_err());
    }
}
