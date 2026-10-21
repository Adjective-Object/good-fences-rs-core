use super::*;
use ast_segmenter::{effects::EffectReason, invocation::ValueUse};
use effects::{ArgumentGuard, StatementEffects};

fn summary(source: &str, demanded: bool) -> StatementEffects {
    let parsed = crate::tests::input("file.ts", source);
    effects::summarize(parsed.facts.view(), parsed.segments[0].span, demanded)
}

#[test]
fn precise_block_comment_attachment_not_substrings_or_declaration_comments() {
    for (source, expected) in [
        ("/* @__PURE__ */ f();", true),
        ("/*#__PURE__*/ new C();", true),
        ("/* @__PURE__ */ (f());", true),
        ("/* @__PURE__ */ (f)(1);", true),
        ("/* @__PURE__ */ f?.();", true),
        ("/* @__PURE__ */ /* ordinary trivia */ f();", true),
        ("/* @__PURE__ */ const x = f();", false),
        ("/* @__PURE__ */ 0; f();", false),
        ("/* @__PURE__ */ unknown + f();", false),
        ("/* @__PURE__ */ 0, f();", false),
        ("/* prose @__PURE__ prose */ f();", false),
        ("// @__PURE__\nf();", false),
        ("const text = '/* @__PURE__ */'; f();", false),
        ("f(); /* @__PURE__ */", false),
        ("/* @__PURE__ */ object.value; f();", false),
    ] {
        let parsed = crate::tests::input("file.ts", source);
        assert_eq!(
            parsed
                .facts
                .invocations()
                .iter()
                .any(|c| c.pure().is_some()),
            expected,
            "{source}"
        );
    }
}

#[test]
fn recipes_preserve_callee_receiver_arguments_and_spread_iteration() {
    let source = "/* @__PURE__ */ object.method(sideEffect(), ...iterable);";
    let result = summary(source, false);
    assert_eq!(result.rewrites.len(), 1);
    assert!(result
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::PropertyRead));
    assert!(result
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::SpreadOrIteration));
    let calls: Vec<_> = result
        .sites
        .iter()
        .filter(|s| s.reason() == EffectReason::Invocation)
        .map(|s| &source[s.span().start as usize..s.span().end as usize])
        .collect();
    assert_eq!(calls, ["sideEffect()"]);
    let rewrite = &result.rewrites[0];
    assert_eq!(
        &source[rewrite.callee.start as usize..rewrite.callee.end as usize],
        "object.method"
    );
    assert_eq!(rewrite.arguments.len(), 2);
    assert!(rewrite.arguments[1].spread());
    insta::assert_debug_snapshot!(result);
}

#[test]
fn pure_invocation_is_not_whole_statement_purity() {
    let graph = SourceGraph::from_owned([crate::tests::input(
        "file.ts",
        "function f(x) {} /* @__PURE__ */ f(sideEffect());",
    )]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.segments.len(), 2);
    let result = &plan.effects[1].summary;
    assert_eq!(result.rewrites.len(), 1);
    assert!(result
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::Invocation));
    let pure = SourceGraph::from_owned([crate::tests::input(
        "file.ts",
        "function f() {} /* @__PURE__ */ f();",
    )]);
    let plan = retain(&pure, &[0], &[]).unwrap();
    assert!(plan.segments.is_empty());
    assert_eq!(plan.effects[1].summary.rewrites.len(), 1);
}

#[test]
fn used_initializer_export_and_argument_results_keep_invocation() {
    let graph = SourceGraph::from_owned([crate::tests::input(
        "file.ts",
        "function f() {} export const value = /* #__PURE__ */ f();",
    )]);
    let dead = retain(&graph, &[0], &[]).unwrap();
    assert!(dead.segments.is_empty());
    assert_eq!(dead.effects[1].summary.rewrites.len(), 1);
    let export = graph
        .resolve_export(0, "value", source_graph::Namespace::Value)
        .unwrap();
    let used = retain(&graph, &[0], &export.route).unwrap();
    assert!(used.effects[1].summary.rewrites.is_empty());
    assert!(used.effects[1]
        .summary
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::Invocation));
    assert!(summary("consume(/* @__PURE__ */ f());", false)
        .rewrites
        .is_empty());
    assert!(summary("const {x} = /* @__PURE__ */ f();", false)
        .rewrites
        .is_empty());
    let default = summary("export default /* @__PURE__ */ f();", false);
    assert_eq!(default.rewrites.len(), 1);
    assert!(summary("export default /* @__PURE__ */ f();", true)
        .rewrites
        .is_empty());
}

#[test]
fn optional_chains_keep_original_argument_guards_not_comma_hoisting() {
    let mut snapshots = Vec::new();
    for (source, guard) in [
        (
            "/* @__PURE__ */ f?.(side());",
            ArgumentGuard::OriginalChain {
                optional_call: true,
            },
        ),
        (
            "/* @__PURE__ */ object?.method(side());",
            ArgumentGuard::OriginalChain {
                optional_call: false,
            },
        ),
        (
            "/* @__PURE__ */ object.method?.(...iterable);",
            ArgumentGuard::OriginalChain {
                optional_call: true,
            },
        ),
        (
            "/* @__PURE__ */ (object?.method)(side());",
            ArgumentGuard::Always,
        ),
    ] {
        let result = summary(source, false);
        assert_eq!(result.rewrites.len(), 1, "{source}");
        assert_eq!(result.rewrites[0].guard, guard, "{source}");
        snapshots.push((source, result.rewrites));
    }
    insta::assert_debug_snapshot!(snapshots);
}

#[test]
fn computed_receivers_and_construction_keep_all_input_evaluation() {
    let result = summary("/*#__PURE__*/ receiver()[key()](arg());", false);
    assert_eq!(result.rewrites.len(), 1);
    assert_eq!(
        result
            .sites
            .iter()
            .filter(|s| s.reason() == EffectReason::Invocation)
            .count(),
        3
    );
    let new = summary("/*#__PURE__*/ new Constructor(side(), ...iterable);", false);
    assert_eq!(new.rewrites.len(), 1);
    assert!(new.rewrites[0].construct);
    assert!(!new
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::Construction));
    assert!(new
        .sites
        .iter()
        .any(|s| s.reason() == EffectReason::SpreadOrIteration));
}

#[test]
fn value_contexts_are_source_facts_not_policy_and_deferred_bodies_stay_intact() {
    let parsed = crate::tests::input("file.ts",
        "function f(x = /*PURE?*/ defaults()) { return /*@__PURE__*/ g(); } const v = (/*@__PURE__*/ a(), /*@__PURE__*/ b());");
    let calls = &parsed.facts.invocations();
    assert_eq!(calls.get(1).unwrap().value_use(), ValueUse::Required);
    assert!(calls.get(1).unwrap().deferred().is_some());
    assert_eq!(calls.get(2).unwrap().value_use(), ValueUse::Discarded);
    assert_eq!(calls.get(3).unwrap().value_use(), ValueUse::StatementValue);
    let graph = SourceGraph::from_owned([parsed]);
    let plan = retain(&graph, &[], &[GraphNode::Segment(SegmentKey::new(0, 0))]).unwrap();
    assert!(plan.effects[0].summary.rewrites.is_empty());
}

#[test]
fn late_binding_demand_restores_effect_order_and_opaque_cjs_ignores_hints() {
    let graph = SourceGraph::from_owned([crate::tests::input(
        "file.ts",
        "function f() {} first(); export const value = /*@__PURE__*/ f(); second();",
    )]);
    let used = retain(&graph, &[0], &[GraphNode::Segment(SegmentKey::new(0, 2))]).unwrap();
    assert!(used
        .edges
        .iter()
        .any(|e| e.kind == RetentionEdgeKind::EffectOrder
            && e.from == GraphNode::Segment(SegmentKey::new(0, 3))
            && e.to == GraphNode::Segment(SegmentKey::new(0, 2))));
    let graph = SourceGraph::from_owned([crate::tests::input("file.cjs", "/*@__PURE__*/ f();")]);
    let plan = retain(&graph, &[0], &[]).unwrap();
    assert_eq!(plan.segments.len(), 1);
    assert!(plan.effects[0].summary.rewrites.is_empty());
    assert_eq!(
        plan.effects[0].summary.sites[0].reason(),
        EffectReason::OpaqueCommonJs
    );
}

#[test]
fn nested_recipes_compose_inside_the_original_optional_argument_guard() {
    let source = "/*@__PURE__*/ f?.((/*@__PURE__*/ g(), sideEffect()));";
    let result = summary(source, false);
    assert_eq!(result.rewrites.len(), 2);
    let outer = &result.rewrites[0];
    let inner = &result.rewrites[1];
    assert!(effects::contains(
        outer.arguments[0].span(),
        inner.invocation
    ));
    assert_eq!(
        outer.guard,
        ArgumentGuard::OriginalChain {
            optional_call: true
        }
    );
    assert_eq!(inner.guard, ArgumentGuard::Always);
    let invocations: Vec<_> = result
        .sites
        .iter()
        .filter(|s| s.reason() == EffectReason::Invocation)
        .map(|s| &source[s.span().start as usize..s.span().end as usize])
        .collect();
    assert_eq!(invocations, ["sideEffect()"]);
    insta::assert_debug_snapshot!(result.rewrites);
}
