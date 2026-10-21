use super::*;

fn parse(source: &str) -> crate::ParsedModule {
    crate::parse_module(
        &logger::StdioLogger::new(),
        source,
        std::path::Path::new("effects.ts"),
        crate::ModuleFormat::Esm,
    )
    .unwrap()
}

#[test]
fn deterministic_conservative_reasons() {
    let source = r#"
function f(value = defaults()) { unknown(); return value; }
const arrow = () => hidden();
const getter = { get value() { return hidden(); } };
f();
object.value;
proxy[key];
unknown + 1;
`${unknown}`;
[...iterable];
const {...rest} = object;
for (const item of iterable) {}
class C extends Base {
  [key] = instance();
  static [other] = initialize();
  static { register(); }
  method() { deferred(); }
}
export default new C();
declare class Ambient extends Unresolved { [key]: unknown }
"#;
    let module = parse(source);
    let reasons: Vec<_> = module
        .facts()
        .effects()
        .iter()
        .map(|s| {
            (
                &source[s.span().start as usize..s.span().end as usize],
                s.reason(),
                s.deferred().is_some(),
            )
        })
        .collect();
    insta::assert_debug_snapshot!(reasons);
    assert_eq!(module.facts().effects(), parse(source).facts().effects());
    for index in [0, 1, 2] {
        let span = module.segments()[index].span;
        assert!(!module
            .facts()
            .effects()
            .iter()
            .any(|s| s.deferred().is_none()
                && span.start <= s.span().start
                && s.span().end <= span.end));
    }
    for reason in [
        EffectReason::Invocation,
        EffectReason::Construction,
        EffectReason::PropertyRead,
        EffectReason::Coercion,
        EffectReason::SpreadOrIteration,
        EffectReason::Destructuring,
        EffectReason::ClassHeritage,
        EffectReason::ComputedKey,
        EffectReason::StaticBlock,
        EffectReason::UnresolvedRead,
    ] {
        assert!(
            module
                .facts()
                .effects()
                .iter()
                .any(|s| s.reason() == reason && s.deferred().is_none()),
            "{:?}",
            reason
        );
    }
    assert!(module
        .facts()
        .effects()
        .iter()
        .any(|s| s.reason() == EffectReason::Invocation
            && s.deferred().is_some()
            && &source[s.span().start as usize..s.span().end as usize] == "instance()"));
}

#[test]
fn unknown_and_tdz_reads_are_not_silently_pure() {
    let module = parse("later; const later = 1; const f = async () => { await unknown; }; let x = 0; x++; throw x;");
    assert!(module
        .facts()
        .effects()
        .iter()
        .any(|s| s.reason() == EffectReason::BindingRead && s.span().start == 0));
    assert!(module
        .facts()
        .effects()
        .iter()
        .any(|s| s.reason() == EffectReason::UnknownExpression && s.deferred().is_some()));
    assert!(module
        .facts()
        .effects()
        .iter()
        .any(|s| s.reason() == EffectReason::Mutation));
    assert!(module
        .facts()
        .effects()
        .iter()
        .any(|s| s.reason() == EffectReason::UnknownControl && s.deferred().is_none()));
}

#[test]
fn commonjs_stays_opaque_even_when_empty_or_only_declarations() {
    for source in ["", "function f() {}"] {
        let parsed = crate::parse_module(
            &logger::StdioLogger::new(),
            source,
            std::path::Path::new("file.cjs"),
            crate::ModuleFormat::CommonJs,
        )
        .unwrap();
        assert_eq!(parsed.facts().effects().len(), 1);
        assert_eq!(
            parsed.facts().effects()[0].reason(),
            EffectReason::OpaqueCommonJs
        );
    }
}
