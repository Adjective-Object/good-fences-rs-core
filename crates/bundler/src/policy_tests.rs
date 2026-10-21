use super::*;
use ast_segmenter::{effects::EffectReason, invocation::Invocation};
use policy::EffectPolicy;
use std::{fs, path::Path};

fn calls(source: &str) -> Vec<Invocation> {
    crate::tests::input("file.ts", source)
        .facts
        .owned()
        .unwrap()
        .invocations()
        .to_vec()
}

#[test]
fn parses_bounded_grammar_canonically_and_fingerprints_effective_configuration() {
    let first = EffectPolicy::parse([
        " *.appendChild( ... ) ",
        "danger(...)",
        "*.appendChild(...)",
    ])
    .unwrap();
    let second = EffectPolicy::parse(["danger(...)", "*.appendChild(...)"]).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.canonical_patterns().collect::<Vec<_>>(),
        ["*.appendChild(...)", "danger(...)"]
    );
    assert_ne!(first.fingerprint(), EffectPolicy::default().fingerprint());
    assert_ne!(
        first.fingerprint(),
        first.clone().with_pure_annotations(false).fingerprint()
    );
    insta::assert_debug_snapshot!(first.fingerprint());
}

#[test]
fn rejects_malformed_unsupported_and_oversized_patterns_with_diagnostics() {
    let mut diagnostics = Vec::new();
    for pattern in [
        "",
        "*",
        "f()",
        "f(value)",
        "f(...args)",
        "f(...);",
        "a.b.c(...)",
        "*.a?.(...)",
        "*['appendChild'](...)",
        "f(...).g(...)",
        "new F(...)",
        "*(...)",
        "a..b(...)",
        "a-b(...)",
        "a b(...)",
        "名(...)",
        "f(...",
        "f(...))",
    ] {
        let error = EffectPolicy::parse([pattern]).unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("effect pattern 1"), "{diagnostic}");
        diagnostics.push((pattern, diagnostic));
    }
    insta::assert_debug_snapshot!(diagnostics);
    let too_long = format!("{}(...)", "a".repeat(policy::MAX_PATTERN_BYTES));
    assert!(EffectPolicy::parse([too_long]).is_err());
    assert!(EffectPolicy::parse(std::iter::repeat_n("f(...)", policy::MAX_PATTERNS + 1)).is_err());
    assert!(EffectPolicy::parse(std::iter::repeat_n("f(...)", policy::MAX_PATTERNS)).is_ok());
}

#[test]
fn wildcard_members_match_computed_strings_optional_and_shadowed_receivers_without_alias_inference()
{
    let policy = EffectPolicy::parse(["*.appendChild(...)"]).unwrap();
    let mut results = Vec::new();
    for (source, expected) in [
        ("node.appendChild(child);", true),
        ("node['appendChild'](child);", true),
        ("node?.appendChild(child);", true),
        ("node.appendChild?.(child);", true),
        ("node?.['appendChild']?.(child);", true),
        ("(node?.appendChild)(child);", true),
        ("getNode().appendChild(child);", true),
        ("function local(node) { node.appendChild(child); }", true),
        ("const alias = node; alias.appendChild(child);", true),
        (
            "const appendChild = node.appendChild; appendChild(child);",
            false,
        ),
        ("node[key](child);", false),
        ("node[`appendChild`](child);", false),
        ("node.other('appendChild');", false),
        ("new node.appendChild(child);", true),
    ] {
        let matches = calls(source)
            .iter()
            .any(|c| policy.matching_patterns(c.into()).next().is_some());
        assert_eq!(matches, expected, "{source}");
        results.push((source, matches));
    }
    insta::assert_debug_snapshot!(results);
}

#[test]
fn exact_global_patterns_exclude_lexical_shadows_and_do_not_follow_aliases() {
    let policy = EffectPolicy::parse(["danger(...)", "document.appendChild(...)"]).unwrap();
    for (source, expected) in [
        ("danger();", true),
        ("danger?.();", true),
        ("new danger();", true),
        ("document['appendChild'](child);", true),
        ("document?.appendChild?.(child);", true),
        ("function danger() {} danger();", false),
        ("const danger = () => {}; danger();", false),
        ("import {danger} from 'external'; danger();", false),
        (
            "function local(document) { document.appendChild(child); }",
            false,
        ),
        (
            "const document = object; document.appendChild(child);",
            false,
        ),
        ("const alias = document; alias.appendChild(child);", false),
        ("const alias = danger; alias();", false),
        ("document.body.appendChild(child);", false),
    ] {
        assert_eq!(
            calls(source)
                .iter()
                .any(|c| policy.matching_patterns(c.into()).next().is_some()),
            expected,
            "{source}"
        );
    }
}

#[test]
fn matching_rules_are_add_only_and_override_pure_without_making_deferred_creation_effectful() {
    let source = "/* @__PURE__ */ node.appendChild(sideEffect(), ...iterable);";
    let parsed = crate::tests::input("entry.ts", source);
    let span = parsed.segments[0].span;
    let base = effects::summarize(parsed.facts.view(), span, false);
    let policy = EffectPolicy::parse(["*.appendChild(...)", "node.appendChild(...)"]).unwrap();
    let known = effects::summarize_with_policy(parsed.facts.view(), span, false, &policy);
    assert_eq!(base.rewrites.len(), 1);
    assert!(known.rewrites.is_empty());
    assert_eq!(known.known_effects.len(), 2);
    assert!(base.sites.iter().all(|s| known.sites.contains(s)));
    assert_eq!(
        known
            .sites
            .iter()
            .filter(|s| s.reason() == EffectReason::Invocation)
            .count(),
        2
    );
    insta::assert_debug_snapshot!(known);
    let graph = SourceGraph::from_owned([crate::tests::input(
        "entry.ts",
        "function f() { /*@__PURE__*/ node.appendChild(child); }",
    )]);
    let plan = retain_with_policy(&graph, &[0], &[], &policy).unwrap();
    assert!(plan.segments.is_empty());
    assert!(plan.effects[0].summary.known_effects.is_empty());
    let plain = calls("unknown();");
    let unknown = crate::tests::input("unknown.ts", "unknown();");
    let result = effects::summarize_with_policy(
        unknown.facts.view(),
        unknown.segments[0].span,
        false,
        &policy,
    );
    assert!(result
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::Invocation));
    assert!(policy
        .matching_patterns((&plain[0]).into())
        .next()
        .is_none());
}

#[test]
fn disabling_annotations_is_fingerprinted_and_restores_invocation() {
    let graph = SourceGraph::from_owned([crate::tests::input(
        "file.ts",
        "function f() {} /*@__PURE__*/ f();",
    )]);
    let policy = EffectPolicy::default().with_pure_annotations(false);
    let default = retain(&graph, &[0], &[]).unwrap();
    let disabled = retain_with_policy(&graph, &[0], &[], &policy).unwrap();
    assert!(default.segments.is_empty());
    assert_eq!(disabled.segments.len(), 2);
    assert!(!default.is_current(&graph, &policy));
    assert!(disabled.is_current(&graph, &policy));
    assert!(!disabled.is_current(&graph, &EffectPolicy::default()));
}

#[test]
fn source_value_context_preserves_concise_arrow_returns_and_binding_roots_reach_evaluation() {
    let graph = SourceGraph::from_owned([crate::tests::input(
        "entry.ts",
        "export const arrow = () => /*@__PURE__*/ node.appendChild(child); effect();",
    )]);
    let call = graph.file_facts(0).unwrap().invocations().get(0).unwrap();
    assert_eq!(
        call.value_use(),
        ast_segmenter::invocation::ValueUse::Required
    );
    assert!(call.deferred().is_some());
    let root = GraphNode::Segment(SegmentKey::new(0, 0));
    let policy = EffectPolicy::parse(["*.appendChild(...)"]).unwrap();
    let plan = retain_with_policy(&graph, &[], &[root], &policy).unwrap();
    assert!(plan
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::BindingEvaluation
            && e.from == root
            && e.to == GraphNode::Module(0)));
    assert!(plan
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::ModuleEffect
            && e.from == GraphNode::Module(0)
            && e.to == GraphNode::Segment(SegmentKey::new(0, 1))));
    assert!(plan.effects[0].summary.known_effects.is_empty());
}

#[test]
fn dsl_only_reanalysis_uses_owned_cache_facts_without_reparse_or_cache_contamination() {
    use ast_segmenter::ModuleFormat;
    use segment_cache::{load_or_parse, sidecar_path, CacheMode, CacheStatus, CacheWrite};
    let dir = test_tmpdir::TmpDir::new();
    let path = dir.root_join("entry.ts");
    let source = "/*@__PURE__*/ node.appendChild(sideEffect());";
    fs::write(&path, source).unwrap();
    let cold = load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite,
    )
    .unwrap();
    assert_eq!(cold.write, CacheWrite::Stored);
    let sidecar = sidecar_path(&path);
    let bytes = fs::read(&sidecar).unwrap();
    let mtime = fs::metadata(&sidecar).unwrap().modified().unwrap();
    let warm = load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite,
    )
    .unwrap();
    assert_eq!(warm.status, CacheStatus::Hit);
    let graph = SourceGraph::from_owned([source_graph::OwnedSourceFileInput::from_parsed(
        path.clone(),
        warm.record.to_parsed(&warm.source).unwrap(),
        Default::default(),
    )]);
    let default = EffectPolicy::default();
    let changed = EffectPolicy::parse(["*.appendChild(...)"]).unwrap();
    // Analysis accepts no source and succeeds while it is absent: policy edits cannot reparse.
    fs::remove_file(&path).unwrap();
    let base = retain_with_policy(&graph, &[0], &[], &default).unwrap();
    let added = retain_with_policy(&graph, &[0], &[], &changed).unwrap();
    assert_eq!(base.revision, added.revision);
    assert_ne!(base.policy_fingerprint, added.policy_fingerprint);
    assert!(!base.is_current(&graph, &changed));
    assert!(added.is_current(&graph, &changed));
    assert_eq!(base.effects[0].summary.rewrites.len(), 1);
    assert!(added.effects[0].summary.rewrites.is_empty());
    assert_eq!(added.effects[0].summary.known_effects.len(), 1);
    fs::write(&path, source).unwrap();
    let again = load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite,
    )
    .unwrap();
    assert_eq!(again.status, CacheStatus::Hit);
    assert_eq!(again.write, CacheWrite::NotRequested);
    assert_eq!(again.record, cold.record);
    assert_eq!(fs::read(&sidecar).unwrap(), bytes);
    assert_eq!(fs::metadata(&sidecar).unwrap().modified().unwrap(), mtime);
    assert_eq!(graph.file_path(0), Some(Path::new(&path)));
}
