use std::{collections::BTreeMap, sync::OnceLock};

use eyre::{ensure, eyre, Result};

use crate::{parse_declarations, Binding, Definition, LocalFacts};

/// Host preset selection, independent of JavaScript language contracts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Environment {
    #[default]
    Node,
    Browser,
}

/// Build-wide contracts. Per-file local contracts belong in `LocalFacts`, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationConfig {
    pub javascript: bool,
    /// Enable Node contracts when selecting `Environment::Node`.
    pub node: bool,
    /// Enable browser contracts when selecting `Environment::Browser`.
    pub browser: bool,
    pub user: Vec<LocalFacts>,
}

impl Default for DeclarationConfig {
    fn default() -> Self {
        Self {
            javascript: true,
            node: true,
            browser: true,
            user: Vec::new(),
        }
    }
}

impl DeclarationConfig {
    /// Expand presets, then replace exact preset bindings with validated user contracts.
    ///
    /// Output ordering depends only on binding names, not user file ordering.
    pub fn effective(&self) -> Result<LocalFacts> {
        self.effective_for_environment(Environment::Node)
    }

    /// Select only the requested host preset, retaining switches and exact user overrides.
    pub fn effective_for_environment(&self, environment: Environment) -> Result<LocalFacts> {
        static PRESETS: [OnceLock<Result<LocalFacts, String>>; 6] = [const { OnceLock::new() }; 6];
        let host = match environment {
            Environment::Node if self.node => 1,
            Environment::Browser if self.browser => 2,
            _ => 0,
        };
        let index = usize::from(self.javascript) * 3 + host;
        let presets = PRESETS[index]
            .get_or_init(|| {
                self.preset_facts(environment)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| eyre!("{error}"))?;
        let mut user = LocalFacts::default();
        for facts in &self.user {
            user.definitions_mut()
                .extend(facts.definitions().iter().cloned());
            user.global_symbols_mut()
                .extend(facts.global_symbols().iter().cloned());
        }
        ensure!(
            user.definitions()
                .iter()
                .all(|definition| !matches!(definition.binding(), Binding::Local(_))),
            "build-wide declarations cannot contain local contracts"
        );
        crate::parser::validate_facts(&user)?;
        let mut definitions = BTreeMap::new();
        let mut symbols = BTreeMap::new();
        for facts in [presets, &user] {
            for definition in facts.definitions() {
                definitions.insert(binding_key(definition), definition.clone());
            }
            for symbol in facts.global_symbols() {
                symbols.insert(symbol.name().to_owned(), symbol.clone());
            }
        }
        Ok(LocalFacts::new(
            definitions.into_values().collect(),
            symbols.into_values().collect(),
        ))
    }

    fn preset_facts(&self, environment: Environment) -> Result<LocalFacts> {
        let mut presets = LocalFacts::default();
        if self.javascript {
            let mut facts = parse_declarations(include_str!("../presets/ecmascript2025.ecco"))?;
            let mut expanded = Vec::new();
            for definition in std::mem::take(facts.definitions_mut()) {
                let Binding::Global(path) = definition.binding() else {
                    unreachable!("JavaScript preset contains only globals")
                };
                if path[0] == "TypedArray" {
                    for name in [
                        "Int8Array",
                        "Uint8Array",
                        "Uint8ClampedArray",
                        "Int16Array",
                        "Uint16Array",
                        "Int32Array",
                        "Uint32Array",
                        "Float16Array",
                        "Float32Array",
                        "Float64Array",
                        "BigInt64Array",
                        "BigUint64Array",
                    ] {
                        let mut alias = definition.clone();
                        let mut path = path.clone();
                        path[0] = name.to_owned();
                        *alias.binding_mut() = Binding::Global(path);
                        expanded.push(alias);
                    }
                } else {
                    expanded.push(definition);
                }
            }
            *facts.definitions_mut() = expanded;
            for definition in facts.definitions() {
                let Binding::Global(path) = definition.binding() else {
                    unreachable!("JavaScript preset contains only globals")
                };
                let mut alias = definition.clone();
                *alias.binding_mut() = Binding::Global(
                    std::iter::once("globalThis".to_owned())
                        .chain(path.iter().cloned())
                        .collect(),
                );
                presets.definitions_mut().push(alias);
            }
            let (definitions, symbols) = facts.into_parts();
            presets.definitions_mut().extend(definitions);
            presets.global_symbols_mut().extend(symbols);
        }
        if environment == Environment::Node && self.node {
            let facts = parse_declarations(include_str!("../presets/node24.ecco"))?;
            let (definitions, symbols) = facts.into_parts();
            for definition in definitions {
                if let Binding::Module { module, path } = definition.binding() {
                    let mut modules = vec![module.clone()];
                    if !matches!(
                        module.as_str(),
                        "node:sea" | "node:sqlite" | "node:test" | "node:test/reporters"
                    ) {
                        modules.push(module.trim_start_matches("node:").to_owned());
                    }
                    for module in modules {
                        let mut alias = definition.clone();
                        *alias.binding_mut() = Binding::Module {
                            module: module.clone(),
                            path: path.clone(),
                        };
                        presets.definitions_mut().push(alias);
                        if path.first().is_some_and(|name| name != "default") {
                            let mut alias = definition.clone();
                            *alias.binding_mut() = Binding::Module {
                                module,
                                path: std::iter::once("default".to_owned())
                                    .chain(path.iter().cloned())
                                    .collect(),
                            };
                            presets.definitions_mut().push(alias);
                        }
                    }
                } else {
                    let mut alias = definition.clone();
                    let Binding::Global(path) = definition.binding() else {
                        unreachable!("Node preset contains no locals")
                    };
                    *alias.binding_mut() = Binding::Global(
                        std::iter::once("globalThis".to_owned())
                            .chain(path.iter().cloned())
                            .collect(),
                    );
                    presets.definitions_mut().push(alias);
                    presets.definitions_mut().push(definition);
                }
            }
            presets.global_symbols_mut().extend(symbols);
        }
        if environment == Environment::Browser && self.browser {
            let facts = parse_declarations(include_str!("../presets/browser.ecco"))?;
            let (definitions, symbols) = facts.into_parts();
            presets.definitions_mut().extend(definitions);
            presets.global_symbols_mut().extend(symbols);
        }
        crate::parser::validate_facts(&presets)?;
        Ok(presets)
    }
}

fn binding_key(definition: &Definition) -> (u8, String, Vec<String>) {
    match definition.binding() {
        Binding::Global(path) => (0, String::new(), path.clone()),
        Binding::Module { module, path } => (1, module.clone(), path.clone()),
        Binding::Local(name) => (2, name.clone(), Vec::new()),
    }
}

/// Whether a binding has an exact contract or is a trusted namespace prefix.
///
/// This does not match arbitrary children of a declared namespace.
pub fn declaration_matches(facts: &LocalFacts, binding: &Binding) -> bool {
    facts.definitions().iter().any(|definition| {
        *definition.binding() == *binding
            || match (binding, definition.binding()) {
                (Binding::Global(prefix), Binding::Global(path)) => {
                    prefix.len() < path.len() && path.starts_with(prefix)
                }
                (
                    Binding::Module {
                        module,
                        path: prefix,
                    },
                    Binding::Module {
                        module: other,
                        path,
                    },
                ) => *module == *other && prefix.len() < path.len() && path.starts_with(prefix),
                _ => false,
            }
    })
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
