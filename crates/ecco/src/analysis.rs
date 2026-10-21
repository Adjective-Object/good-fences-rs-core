use std::{collections::HashMap, hash::Hash, sync::Arc};

use eyre::{ensure, eyre, Result};
use oxc_span::Span;
use serde::{Deserialize, Serialize};

use crate::*;

mod records;
pub use records::*;

impl ParsedFile {
    /// Check deserialized IDs and references before they enter an interpreter.
    pub fn validate(&self) -> Result<()> {
        crate::parser::validate_facts(&self.declarations())?;
        ensure!(self.version() == 1, "unsupported Ecco parsed-file version");
        ensure!(self.values().len() >= 2, "missing Ecco constant values");
        ensure!(
            self.values()[0].hazards() == Hazards::NONE
                && self.values()[1].hazards() == Hazards::UNKNOWN
                && self.values()[..2]
                    .iter()
                    .all(|v| v.alias().is_none() && v.definition().is_none()),
            "invalid Ecco constant values"
        );
        let mut definitions = vec![false; self.declarations().definitions().len()];
        for value in self.values() {
            if let Some(index) = value.definition() {
                let slot = definitions
                    .get_mut(index)
                    .ok_or_else(|| eyre!("invalid Ecco declaration index"))?;
                ensure!(!*slot, "duplicate Ecco declaration slot");
                *slot = true;
            }
            if let Some(alias) = value.alias() {
                ensure!(self.valid_value(alias), "invalid Ecco initial alias");
            }
        }
        ensure!(
            definitions.into_iter().all(|present| present),
            "missing Ecco declaration slot"
        );
        ensure!(
            self.declarations()
                .definitions()
                .iter()
                .enumerate()
                .all(|(index, _)| {
                    self.values()
                        .get(index + 2)
                        .is_some_and(|v| v.definition() == Some(index))
                }),
            "invalid Ecco declaration slot order"
        );
        let mut symbols = std::collections::HashSet::new();
        for symbol in self.symbols() {
            ensure!(
                symbol.id().0 == symbol.span().start
                    && symbol.span().start <= symbol.span().end
                    && symbols.insert(symbol.id())
                    && self.valid_value(symbol.value()),
                "invalid or duplicate Ecco source symbol"
            );
        }
        for reference in self.external_references() {
            ensure!(
                reference.span().start <= reference.span().end,
                "invalid Ecco reference span"
            );
        }
        for root in self.roots() {
            ensure!(
                root.span().start <= root.span().end,
                "invalid Ecco root span"
            );
            ensure!(
                root.function().returns().len() == 1,
                "invalid Ecco root result"
            );
            self.validate_function(&root.function())?;
        }
        self.validate_function(&self.setup())?;
        for function in self.functions() {
            self.validate_function(function)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol<K> {
    pub file: K,
    pub local: SymbolId,
}

#[derive(Debug, Clone)]
pub enum Resolution<K> {
    Symbol(Symbol<K>),
    Root {
        file: K,
        root: RootId,
    },
    /// Module namespace identity; static members are separate external references.
    Namespace(K),
    /// A contract in the referring file's declaration set.
    Declaration(Binding),
    Unknown,
}

/// The caller owns import/export resolution. Errors (including ambiguous or
/// missing exports) must be returned, not converted to `Unknown`.
pub trait Resolver<K> {
    fn resolve(&self, from: &K, reference: &ExternalReference) -> Result<Resolution<K>>;
}

impl<K, F> Resolver<K> for F
where
    F: Fn(&K, &ExternalReference) -> Result<Resolution<K>>,
{
    fn resolve(&self, from: &K, reference: &ExternalReference) -> Result<Resolution<K>> {
        self(from, reference)
    }
}

#[derive(Debug, Clone)]
pub struct FileAnalysis {
    pub roots: Vec<SourceRoot>,
    pub matches: Vec<DefinitionMatch>,
    /// Final bound values, not interpreter-local lexical binding slots.
    pub symbols: HashMap<SymbolId, ValueId>,
}

#[derive(Debug, Clone)]
pub struct AnalysisResult<K> {
    pub context: Context,
    pub evaluation: Evaluation,
    pub files: HashMap<K, FileAnalysis>,
}

/// Register every file before solving, including files participating in cycles.
#[derive(Debug, Clone)]
pub struct Analysis<K> {
    files: Vec<(K, Arc<ParsedFile>)>,
}

impl<K> Default for Analysis<K> {
    fn default() -> Self {
        Self { files: Vec::new() }
    }
}

impl<K: Clone + Eq + Hash> Analysis<K> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, key: K, parsed: Arc<ParsedFile>) -> Result<()> {
        ensure!(
            !self.files.iter().any(|(existing, _)| existing == &key),
            "duplicate Ecco file key"
        );
        parsed.validate()?;
        self.files.push((key, parsed));
        Ok(())
    }

    /// Resolve against one immutable caller graph, then instantiate lazy roots
    /// and calls and propagate hazards to convergence.
    pub fn solve(self, resolver: &impl Resolver<K>) -> Result<AnalysisResult<K>> {
        let mut context = Context::from_local_facts(LocalFacts::default())?;
        let mut values = HashMap::new();
        let mut symbols = HashMap::new();
        let mut root_values = HashMap::new();
        let mut namespaces = HashMap::new();
        let mut declaration_namespaces = HashMap::new();
        for (key, parsed) in &self.files {
            let environment = context.install_values(parsed);
            for symbol in parsed.symbols() {
                symbols.insert(
                    Symbol {
                        file: key.clone(),
                        local: symbol.id(),
                    },
                    environment[&symbol.value()],
                );
            }
            for index in 0..parsed.roots().len() {
                root_values.insert((key.clone(), RootId(index)), context.value(Hazards::NONE));
            }
            namespaces.insert(key.clone(), namespace(&mut context));
            values.insert(key.clone(), environment);
        }

        let mut installed = Vec::new();
        for (key, parsed) in &self.files {
            let environment = &values[key];
            let mut externals = Vec::new();
            for reference in parsed.external_references() {
                let value = match resolver.resolve(key, reference)? {
                    Resolution::Symbol(symbol) => *symbols
                        .get(&symbol)
                        .ok_or_else(|| eyre!("resolver returned an unregistered Ecco symbol"))?,
                    Resolution::Root { file, root } => *root_values
                        .get(&(file, root))
                        .ok_or_else(|| eyre!("resolver returned an unregistered Ecco root"))?,
                    Resolution::Namespace(file) => *namespaces
                        .get(&file)
                        .ok_or_else(|| eyre!("resolver returned an unregistered Ecco namespace"))?,
                    Resolution::Declaration(binding) => {
                        if let Some(index) = parsed
                            .declarations()
                            .definitions()
                            .iter()
                            .position(|definition| definition.binding() == &binding)
                        {
                            context.declaration_value(parsed, environment, index)
                        } else {
                            ensure!(
                                parsed.is_declared_namespace(&binding),
                                "resolver returned a missing Ecco declaration"
                            );
                            *declaration_namespaces
                                .entry((key.clone(), binding))
                                .or_insert_with(|| namespace(&mut context))
                        }
                    }
                    Resolution::Unknown => context.unknown(),
                };
                externals.push(value);
            }
            let (setup, roots) = context.install_file(parsed, environment.clone(), externals);
            for (index, root) in roots.iter().enumerate() {
                context.root_output(*root, root_values[&(key.clone(), RootId(index))]);
            }
            installed.push((key, parsed, setup, roots));
        }
        // Install every hoisted function before executing any module roots.
        for (_, _, setup, _) in &installed {
            context.run_root(*setup);
        }
        let mut files = HashMap::new();
        for (key, parsed, _, roots) in installed {
            let roots = roots
                .into_iter()
                .zip(parsed.roots())
                .map(|(root, parsed)| {
                    let result = context.run_root(root);
                    SourceRoot {
                        span: parsed.span(),
                        value: result.value(),
                        effects: result.effects(),
                    }
                })
                .collect();
            files.insert(
                key.clone(),
                FileAnalysis {
                    roots,
                    matches: parsed.definition_matches().to_vec(),
                    symbols: parsed
                        .symbols()
                        .iter()
                        .map(|symbol| (symbol.id(), values[key][&symbol.value()]))
                        .collect(),
                },
            );
        }
        for file in files.values_mut() {
            for value in file.symbols.values_mut() {
                *value = context.read_binding(*value);
            }
        }
        let evaluation = context.evaluate();
        Ok(AnalysisResult {
            context,
            evaluation,
            files,
        })
    }
}

fn namespace(context: &mut Context) -> ValueId {
    let mut hazards = Hazards::UNKNOWN;
    hazards.0[Hazard::PropertyRead as usize] = false;
    context.value(hazards)
}

pub(crate) fn analyze_parsed(parsed: ParsedFile) -> Result<SourceAnalysis> {
    let mut analysis = Analysis::new();
    let parsed = Arc::new(parsed);
    analysis.add_file((), parsed.clone())?;
    let result = analysis.solve(&|_: &(), reference: &ExternalReference| {
        if parsed
            .declarations()
            .definitions()
            .iter()
            .any(|d| d.binding() == reference.binding())
            || parsed.is_declared_namespace(reference.binding())
        {
            Ok(Resolution::Declaration(reference.binding().clone()))
        } else {
            Ok(Resolution::Unknown)
        }
    })?;
    let file = result.files.into_values().next().expect("registered file");
    Ok(SourceAnalysis {
        context: result.context,
        roots: file.roots,
        matches: file.matches,
    })
}

impl ParsedFile {
    fn is_declared_namespace(&self, binding: &Binding) -> bool {
        self.declarations().definitions().iter().any(|definition| {
            match (binding, definition.binding()) {
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
}

pub(crate) mod span {
    use super::*;

    pub fn serialize<S: serde::Serializer>(span: &Span, serializer: S) -> Result<S::Ok, S::Error> {
        (span.start, span.end).serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Span, D::Error> {
        let (start, end) = <(u32, u32)>::deserialize(deserializer)?;
        if start > end {
            return Err(serde::de::Error::custom("reversed source span"));
        }
        Ok(Span::new(start, end))
    }
}
