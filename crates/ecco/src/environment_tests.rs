use std::path::Path;

use super::*;
use crate::{analyze_source, Hazard, HazardExpr};

fn global(path: &[&str]) -> Binding {
    Binding::Global(path.iter().map(|name| (*name).to_owned()).collect())
}

fn module(name: &str, path: &[&str]) -> Binding {
    Binding::Module {
        module: name.to_owned(),
        path: path.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn definition<'a>(facts: &'a LocalFacts, binding: &Binding) -> &'a Definition {
    facts
        .definitions
        .iter()
        .find(|definition| &definition.binding == binding)
        .unwrap_or_else(|| panic!("missing {binding:?}"))
}

#[test]
fn environment_presets_are_independent_and_match_only_exact_paths_or_prefixes() {
    let config = DeclarationConfig::default();
    let all = config.effective().unwrap();
    for binding in [
        global(&["Math", "abs"]),
        global(&["globalThis", "Math", "abs"]),
        global(&["Float16Array", "prototype", "map"]),
        global(&["RegExp", "escape"]),
        global(&["Iterator", "prototype", "toArray"]),
        global(&["console", "log"]),
        module("node:fs/promises", &["readFile"]),
        module("fs/promises", &["default", "readFile"]),
        module("node:sqlite", &["DatabaseSync"]),
        module("node:test/reporters", &["spec"]),
    ] {
        definition(&all, &binding);
        assert!(declaration_matches(&all, &binding));
    }
    for binding in [
        global(&["Math"]),
        module("node:fs", &[]),
        module("node:fs", &["default"]),
    ] {
        assert!(declaration_matches(&all, &binding));
    }
    for binding in [
        global(&["Math", "unknown"]),
        module("node:fs", &["unknown"]),
        module("sqlite", &["DatabaseSync"]),
        global(&["TypedArray", "from"]),
        Binding::Local("Math".into()),
    ] {
        assert!(!declaration_matches(&all, &binding));
    }
    let js = DeclarationConfig {
        node: false,
        ..config.clone()
    }
    .effective()
    .unwrap();
    assert!(declaration_matches(&js, &global(&["Math", "abs"])));
    assert!(!declaration_matches(&js, &global(&["console", "log"])));
    let node = DeclarationConfig {
        javascript: false,
        ..config
    }
    .effective()
    .unwrap();
    assert!(!declaration_matches(&node, &global(&["Math", "abs"])));
    assert!(declaration_matches(&node, &global(&["console", "log"])));
    assert_eq!(
        DeclarationConfig {
            javascript: false,
            node: false,
            user: vec![]
        }
        .effective()
        .unwrap(),
        LocalFacts::default()
    );
}

#[test]
fn environment_user_overrides_are_exact_and_order_is_canonical() {
    let a = parse_declarations(
        "global fn Math.abs(...) { call_effects = true }
         global symbol navigator { read_effects = false }",
    )
    .unwrap();
    let b = parse_declarations(
        r#"module "node:fs" { export fn readFile(...) { call_effects = false } }"#,
    )
    .unwrap();
    let first = DeclarationConfig {
        user: vec![a.clone(), b.clone()],
        ..Default::default()
    };
    let second = DeclarationConfig {
        user: vec![b, a],
        ..Default::default()
    };
    let facts = first.effective().unwrap();
    assert_eq!(facts, second.effective().unwrap());
    assert_eq!(
        serde_cbor::to_vec(&facts).unwrap(),
        serde_cbor::to_vec(&second.effective().unwrap()).unwrap()
    );
    assert_eq!(
        definition(&facts, &global(&["Math", "abs"]))
            .hazards
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert_ne!(
        definition(&facts, &global(&["globalThis", "Math", "abs"]))
            .hazards
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert_eq!(
        definition(&facts, &module("node:fs", &["readFile"]))
            .hazards
            .get(Hazard::Call),
        HazardExpr::Bool(false)
    );
    assert_eq!(
        definition(&facts, &module("fs", &["readFile"]))
            .hazards
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert!(
        !facts
            .global_symbols
            .iter()
            .find(|symbol| symbol.name == "navigator")
            .unwrap()
            .read_effects
    );
}

#[test]
fn environment_rejects_duplicates_invalid_facts_and_build_wide_locals() {
    for source in [
        "global fn Boolean(...) {}",
        "global symbol navigator { read_effects = false }",
        r#"module "node:path" { export fn join(...) {} }"#,
    ] {
        let user = parse_declarations(source).unwrap();
        let config = DeclarationConfig {
            user: vec![user.clone(), user],
            ..Default::default()
        };
        assert!(config
            .effective()
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }
    let local = parse_declarations("fn local(...) {}").unwrap();
    assert!(DeclarationConfig {
        user: vec![local.clone()],
        ..Default::default()
    }
    .effective()
    .unwrap_err()
    .to_string()
    .contains("local"));
    assert!(crate::Context::from_local_facts(local).is_ok());
    let mut invalid = parse_declarations("global fn invalid(value) { returns = value }").unwrap();
    invalid.definitions[0].parameters.clear();
    assert!(DeclarationConfig {
        user: vec![invalid],
        ..Default::default()
    }
    .effective()
    .is_err());
}

#[test]
fn environment_specialization_preserves_coercions_mutations_and_host_effects() {
    let facts = DeclarationConfig::default().effective().unwrap();
    for (source, effectful) in [
        ("Math.abs(2);", false),
        ("Number.isFinite({});", false),
        ("Boolean({});", false),
        ("Object.is({}, {});", false),
        (
            "Math.abs({ valueOf() { console.log('coerced'); return 1; } });",
            true,
        ),
        (
            "Number({ valueOf() { console.log('coerced'); return 1; } });",
            true,
        ),
        ("Object.assign({}, {});", true),
        ("Array.prototype.push.call([], 1);", true),
        ("Atomics.store(unknown, 0, 1);", true),
        ("console.log('visible');", true),
        ("setTimeout(() => {}, 0);", true),
        ("navigator;", true),
        ("localStorage;", true),
        (
            "import { writeFile } from 'node:fs'; writeFile('a', 'b');",
            true,
        ),
    ] {
        let analysis = analyze_source(source, Path::new("environment.js"), facts.clone()).unwrap();
        let evaluation = analysis.context.evaluate();
        assert_eq!(
            analysis
                .roots
                .iter()
                .any(|root| evaluation.effect(root.effects)),
            effectful,
            "{source}"
        );
    }
}
