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
        .definitions()
        .iter()
        .find(|definition| definition.binding() == binding)
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
        browser: false,
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
            browser: false,
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
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert_ne!(
        definition(&facts, &global(&["globalThis", "Math", "abs"]))
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert_eq!(
        definition(&facts, &module("node:fs", &["readFile"]))
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Bool(false)
    );
    assert_eq!(
        definition(&facts, &module("fs", &["readFile"]))
            .hazards()
            .get(Hazard::Call),
        HazardExpr::Bool(true)
    );
    assert!(!facts
        .global_symbols()
        .iter()
        .find(|symbol| symbol.name() == "navigator")
        .unwrap()
        .read_effects());
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
    invalid.definitions_mut()[0].parameters_mut().clear();
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

#[test]
fn browser_environment_is_separate_and_preserves_host_effects() {
    let config = DeclarationConfig::default();
    let facts = config
        .effective_for_environment(Environment::Browser)
        .unwrap();
    definition(&facts, &global(&["fetch"]));
    definition(&facts, &global(&["Math", "abs"]));
    assert!(!declaration_matches(
        &facts,
        &module("node:fs", &["readFile"])
    ));
    assert!(!declaration_matches(&facts, &global(&["process"])));
    assert!(!declaration_matches(
        &facts,
        &global(&["window", "arbitraryAPI"])
    ));
    for (source, effectful) in [
        ("Math.abs(2);", false),
        ("Number.isFinite({});", false),
        ("const document = 1; document;", false),
        ("window;", true),
        ("self;", true),
        ("document;", true),
        ("typeof document;", true),
        ("function read() { return document; } read();", true),
        ("globalThis.document;", true),
        ("window.document;", true),
        ("self.location.href;", true),
        ("navigator.userAgent;", true),
        ("location;", true),
        ("document.cookie;", true),
        ("document.cookie = 'x=y';", true),
        ("document.createElement('custom-element');", true),
        ("document.querySelector('main');", true),
        ("document.body.appendChild(unknown);", true),
        ("window.arbitraryAPI();", true),
        ("fetch('/data');", true),
        ("setTimeout(() => {}, 0);", true),
        ("clearTimeout(1);", true),
        ("queueMicrotask(() => {});", true),
        ("requestAnimationFrame(() => {});", true),
        ("addEventListener('click', () => {});", true),
        ("dispatchEvent(unknown);", true),
        ("Event;", true),
        ("new CustomEvent('ready');", true),
        ("new Image();", true),
        ("new MutationObserver(() => {});", true),
        ("location.href = '/next';", true),
        ("localStorage.getItem('key');", true),
    ] {
        let analysis = analyze_source(source, Path::new("browser.js"), facts.clone()).unwrap();
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
    let disabled = DeclarationConfig {
        javascript: false,
        browser: false,
        ..Default::default()
    };
    assert_eq!(
        disabled
            .effective_for_environment(Environment::Browser)
            .unwrap(),
        LocalFacts::default()
    );
    assert!(declaration_matches(
        &disabled.effective().unwrap(),
        &module("node:fs", &["readFile"])
    ));
}

#[test]
fn browser_user_contracts_override_reads_and_calls_independently() {
    for (extra, effectful) in [
        ("", true),
        ("global symbol fetch { read_effects = false }", false),
    ] {
        let config = DeclarationConfig {
            user: vec![parse_declarations(&format!(
                "global fn fetch(...) {{ call_effects = false }} {extra}"
            ))
            .unwrap()],
            ..Default::default()
        };
        let facts = config
            .effective_for_environment(Environment::Browser)
            .unwrap();
        let analysis = analyze_source("fetch('/data');", Path::new("browser.js"), facts).unwrap();
        let evaluation = analysis.context.evaluate();
        assert_eq!(evaluation.effect(analysis.roots[0].effects), effectful);
    }
}
