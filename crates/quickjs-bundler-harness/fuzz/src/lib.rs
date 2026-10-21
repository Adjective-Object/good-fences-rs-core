use std::{
    collections::BTreeMap,
    fmt::Write,
    path::{Component, Path, PathBuf},
};

use ast_segmenter::{ModuleFormat, parse_module};
use bundler::{
    build::OutputOptions,
    chunks::{ChunkPlan, RootId, RootSeed, partition_with_options},
    link::{Artifacts, Backend, esm_with_options},
    policy::EffectPolicy,
};
use eyre::{Context, Result, ensure};
use quickjs_bundler_harness::{HarnessRunResult, run};
use source_graph::{DependencyTarget, GraphNode, OwnedSourceFileInput, SourceGraph};

pub const MAX_OPERATIONS: usize = 24;
pub const MAX_INPUT_BYTES: usize = 4 + MAX_OPERATIONS;
pub type Modules = BTreeMap<PathBuf, String>;

/// Total byte decoder: prefixes shrink to smaller valid programs, not syntax errors.
pub fn generate(data: &[u8]) -> Modules {
    let header = |index| data.get(index).copied().unwrap_or(0);
    let shared = header(0) & 1 != 0;
    let initial = header(2) % 16;
    let mut operations = String::new();
    for (index, byte) in data.iter().skip(4).take(MAX_OPERATIONS).enumerate() {
        let value = byte >> 4;
        let label = format!("op{index}");
        let operation = match byte & 15 {
            0 => format!("const p=h.property('{label}',{value}); p.value; p.value={value}+1;"),
            1 => format!("+h.coercible('{label}',{value});"),
            2 => format!("'key' in h.membership('{label}');"),
            3 => format!("{value} instanceof h.instance('{label}');"),
            4 => format!("h.callable('{label}',{value})({value},state.value);"),
            5 => format!("new (h.constructible('{label}'))({value});"),
            6 => format!("[...h.iterable('{label}',[{value},state.value])];"),
            7 => format!("h.prototype('{label}') instanceof Object;"),
            8 => format!(
                "const o={{value:{value}}}; h.effect('{label}',h.pure(o)===o,o.value,\
                 +{value},'key' in {{}},{value} instanceof Object,(()=>{value})(),\
                 new Object()!==new Object(),[...[{value}]][0],\
                 Object.getPrototypeOf({{}})===Object.prototype);"
            ),
            9 => format!(
                "const alias=state; alias.value+={value}; value=alias.value;\
                 h.effect('{label}',state.value,value);"
            ),
            10 => format!(
                "const receiver={{value:{value}}};\
                 h.invoke(function(x,...rest){{h.effect('{label}',this.value,x,rest);}},\
                 receiver,state.value,{value});"
            ),
            11 => format!("h.globalRead('fuzzGlobal',{value}); h.effect('{label}',fuzzGlobal);"),
            12 => format!(
                "try{{h.fail('{label}',{value});}}catch(error){{h.effect('caught',error);}}"
            ),
            13 => format!(
                "async function later(x){{h.effect('{label}',x); return x+1;}}\
                 h.effect('awaited',await later({value}));"
            ),
            14 => format!(
                "for(const item of h.iterable('{label}',[{value},state.value]))\
                 {{h.effect('first',item);break;}}"
            ),
            15 => format!(
                "const snapshot={{values:[state.value,{value}]}};\
                 h.effect('{label}',snapshot);snapshot.values[0]+={value};\
                 h.effect('mutated',snapshot);"
            ),
            _ => unreachable!(),
        };
        let wrapped = match byte >> 6 {
            0 => format!("{{{operation}}}"),
            1 => format!("if((state.value&1)==={value}%2){{{operation}}}"),
            2 => format!("try{{{operation}}}catch(error){{h.effect('unexpected',String(error));}}"),
            _ => format!("await (async()=>{{{operation}}})();"),
        };
        writeln!(operations, "{wrapped}").unwrap();
    }
    let shared_source = "export function left(x){return x+1;}\
                         export function right(x){return x+2;}\
                         export function common(x){return x+3;}";
    let a_source = "import {left,common} from '../shared.js';\
                    export function readA(x){return common(left(x));}";
    let b_source = if shared {
        "import {right,common} from '../shared.js';\
         export function readB(x){return common(right(x));}"
    } else {
        "export function readB(x){return x+5;}"
    };
    let (first, second) = if header(1) & 1 == 0 {
        ("a", "b")
    } else {
        ("b", "a")
    };
    let (first_read, second_read) = if first == "a" {
        ("readA", "readB")
    } else {
        ("readB", "readA")
    };
    let repeat = if header(1) & 2 != 0 {
        format!(
            "const again=await import('./islands/{first}.js');\
             h.effect('repeat',again.{first_read}(value));"
        )
    } else {
        String::new()
    };
    let guard = header(3) & 1 != 0;
    let offset = header(3) >> 1;
    BTreeMap::from([
        (
            "entry.js".into(),
            format!(
                "import {{execute,value,update}} from './ops.js';\
                 import {{compute}} from './async.js';\
                 export async function main(){{\
                 h.effect('start',value);\
                 await execute(); update({initial});h.effect('live',value);\
                 h.effect('computed',await compute(value-{offset},{guard}));\
                 const first=await import('./islands/{first}.js');\
                 h.effect('first-island',first.{first_read}(value));\
                 {repeat}\
                 const second=await import('./islands/{second}.js');\
                 h.effect('second-island',second.{second_read}(value));\
                 }}"
            ),
        ),
        (
            "ops.js".into(),
            format!(
                "const state=h.cell({initial});export let value={initial};\
                 export function update(x){{state.value+=x;value=state.value;}}\
                 export async function execute(){{{operations}}}"
            ),
        ),
        (
            "async.js".into(),
            "function add(x){return x+7;}\
             function checked(x){if(x<0)throw 2;return add(x);}\
             export async function compute(x,guard){\
             if(guard){try{return checked(x);}catch(error){return error;}}\
             return add(x);}"
                .into(),
        ),
        ("islands/a.js".into(), a_source.into()),
        ("islands/b.js".into(), b_source.into()),
        ("shared.js".into(), shared_source.into()),
    ])
}

fn resolve(modules: &Modules, importer: &Path, specifier: &str) -> Result<DependencyTarget> {
    if specifier == "harness" {
        return Ok(DependencyTarget::External("harness".into()));
    }
    ensure!(
        specifier.starts_with("./") || specifier.starts_with("../"),
        "unsupported bare dependency {specifier:?}"
    );
    let joined = importer.parent().unwrap_or(Path::new("")).join(specifier);
    let mut target = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Normal(part) => target.push(part),
            Component::CurDir => {}
            Component::ParentDir => ensure!(target.pop(), "dependency escapes map"),
            _ => eyre::bail!("nonrelative dependency"),
        }
    }
    ensure!(
        modules.contains_key(&target),
        "missing dependency {target:?}"
    );
    Ok(DependencyTarget::File(target))
}

fn graph(modules: &Modules) -> Result<SourceGraph> {
    let inputs = modules
        .iter()
        .map(|(path, source)| {
            let parsed = parse_module(&logger::StdioLogger::new(), source, path, ModuleFormat::Esm)
                .wrap_err_with(|| format!("parse {path:?}"))?;
            let resolutions = parsed
                .facts
                .dependencies
                .iter()
                .map(|dependency| {
                    Ok((
                        dependency.id,
                        resolve(modules, path, &dependency.specifier)?,
                    ))
                })
                .collect::<Result<_>>()?;
            Ok(OwnedSourceFileInput::from_parsed(
                path.clone(),
                parsed,
                resolutions,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SourceGraph::from_owned(inputs))
}

fn build(
    graph: &SourceGraph,
    modules: &Modules,
    options: &OutputOptions,
) -> Result<(ChunkPlan, Artifacts)> {
    let entry = graph
        .file_id(Path::new("entry.js"))
        .ok_or_else(|| eyre::eyre!("missing entry"))?;
    let policy = EffectPolicy::default();
    // Partition discovers reachable dynamic occurrence roots to its fixed point.
    let plan = partition_with_options(
        graph,
        &[RootSeed {
            id: RootId::Entry("main".into()),
            evaluation_roots: vec![entry],
            binding_roots: vec![GraphNode::Module(entry)],
        }],
        &policy,
        options,
    )
    .wrap_err("partition")?;
    let artifacts = esm_with_options(graph, &plan, &policy, modules, options).wrap_err("emit")?;
    ensure!(artifacts.backend == Backend::Native, "not native output");
    Ok((plan, artifacts))
}

fn compare(source: &HarnessRunResult, artifacts: &Artifacts) -> Result<()> {
    let entry = artifacts
        .entries
        .get("main")
        .ok_or_else(|| eyre::eyre!("output: missing entry"))?;
    let output = execute(&artifacts.files, entry).wrap_err("output execution")?;
    ensure!(
        source == &output,
        "effects mismatch\nsource log: {source:#?}\noutput log: {output:#?}"
    );
    Ok(())
}

fn execute(modules: &Modules, entry: &Path) -> Result<HarnessRunResult> {
    let mut modules = modules.clone();
    let bootstrap = PathBuf::from("__fuzz_runner__.js");
    ensure!(!modules.contains_key(&bootstrap), "reserved runner path");
    modules.insert(
        bootstrap.clone(),
        format!(
            "import * as library from 'harness';\
             globalThis.h=library;const entry=await import('./{}');await entry.main();",
            entry.display()
        ),
    );
    run(&modules, bootstrap)
}

/// Runs the real source and every output configuration; no error is an acceptance.
pub fn check(data: &[u8]) -> Result<()> {
    let modules = generate(data);
    let context = || format!("input: {data:?}\nsource modules: {modules:#?}");
    let source = execute(&modules, Path::new("entry.js"))
        .wrap_err_with(|| format!("generator/source execution\n{}", context()))?;
    let graph = graph(&modules).wrap_err_with(|| format!("parse/resolution\n{}", context()))?;
    for synthesize_async_boundaries in [false, true] {
        for min_chunk_bytes in [0, 4096] {
            let options = OutputOptions {
                synthesize_async_boundaries,
                min_chunk_bytes,
                ..OutputOptions::default()
            };
            let config = format!(
                "synthesize_async_boundaries={synthesize_async_boundaries},\
                 min_chunk_bytes={min_chunk_bytes}"
            );
            let (_, artifacts) = build(&graph, &modules, &options).wrap_err_with(|| {
                format!("build: {config}\n{}\nsource log: {source:#?}", context())
            })?;
            compare(&source, &artifacts).wrap_err_with(|| {
                format!(
                    "{config}\n{}\nsource log: {source:#?}\nemitted modules: {:#?}",
                    context(),
                    artifacts.files
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bundler::chunks::NodeId;

    const SEEDS: &[&[u8]] = &[
        include_bytes!("../seeds/hazards"),
        include_bytes!("../seeds/shared"),
        include_bytes!("../seeds/guards"),
    ];

    #[test]
    fn seeds_and_diverse_prefixes_match_real_output() {
        check(&[]).unwrap();
        for seed in SEEDS {
            for end in 0..=seed.len() {
                check(&seed[..end]).unwrap();
            }
        }
        for seed in 0_u32..24 {
            let bytes: Vec<_> = (0_u32..28)
                .map(|index| seed.wrapping_mul(37).wrapping_add(index * 19) as u8)
                .collect();
            check(&bytes).unwrap();
        }
    }

    #[test]
    fn generation_is_deterministic_and_bounded() {
        let bytes: Vec<_> = (0..=255).collect();
        assert_eq!(generate(&bytes), generate(&bytes));
        assert_eq!(generate(&bytes), generate(&bytes[..MAX_INPUT_BYTES]));
        let modules = generate(&bytes);
        assert_eq!(modules.len(), 6);
        assert!(modules.values().all(|source| source.len() < 16_384));
        assert!(modules[Path::new("ops.js")].contains("'op23'"));
        assert!(!modules[Path::new("ops.js")].contains("'op24'"));
    }

    #[test]
    fn guards_skip_operations_and_generated_functions_can_throw() {
        for (operation, executes) in [(0x40, true), (0x50, false)] {
            let modules = generate(&[0, 0, 0, 0, operation]);
            let result = execute(&modules, Path::new("entry.js")).unwrap();
            assert_eq!(
                result
                    .effects
                    .iter()
                    .any(|effect| effect.name == "property.get"),
                executes
            );
            check(&[0, 0, 0, 0, operation]).unwrap();
        }
        for (guard, expected) in [(2, 6), (3, 2)] {
            let modules = generate(&[0, 0, 0, guard]);
            let result = execute(&modules, Path::new("entry.js")).unwrap();
            let computed = result
                .effects
                .iter()
                .find(|effect| effect.name == "computed")
                .unwrap();
            assert_eq!(computed.arguments[0].as_i64(), Some(expected));
            check(&[0, 0, 0, guard]).unwrap();
        }
    }

    #[test]
    fn emitted_effect_mutation_is_rejected() {
        let modules = generate(&[]);
        let source = execute(&modules, Path::new("entry.js")).unwrap();
        let (_, mut artifacts) = build(
            &graph(&modules).unwrap(),
            &modules,
            &OutputOptions::default(),
        )
        .unwrap();
        compare(&source, &artifacts).unwrap();
        let entry = artifacts.entries["main"].clone();
        artifacts.files.get_mut(&entry).unwrap().push_str(
            "\nimport {effect as fuzzInjectedEffect} from 'harness';\
             fuzzInjectedEffect('deliberate-mutation');",
        );
        let error = compare(&source, &artifacts).unwrap_err();
        assert!(format!("{error:#}").contains("effects mismatch"));
    }

    #[test]
    fn emitted_stall_after_all_effects_is_rejected() {
        let modules = generate(&[]);
        let source = execute(&modules, Path::new("entry.js")).unwrap();
        let (_, mut artifacts) = build(
            &graph(&modules).unwrap(),
            &modules,
            &OutputOptions::default(),
        )
        .unwrap();
        let original = artifacts.entries["main"].clone();
        artifacts.files.insert(
            "stall.js".into(),
            format!(
                "import {{main as original}} from './{}';\
                 export async function main(){{await original();await new Promise(()=>{{}});}}",
                original.display()
            ),
        );
        artifacts.entries.insert("main".into(), "stall.js".into());
        let error = compare(&source, &artifacts).unwrap_err();
        assert!(format!("{error:#}").contains("entry evaluation is unsettled"));
    }

    #[test]
    fn generated_boundaries_and_parent_placement_really_happen() {
        let modules = generate(&[]);
        let graph = graph(&modules).unwrap();
        let (disabled, _) = build(&graph, &modules, &OutputOptions::default()).unwrap();
        assert!(disabled.generated_boundaries.is_empty());
        let (enabled, artifacts) = build(
            &graph,
            &modules,
            &OutputOptions {
                synthesize_async_boundaries: true,
                ..OutputOptions::default()
            },
        )
        .unwrap();
        assert!(!enabled.generated_boundaries.is_empty());
        assert!(
            enabled
                .generated_boundaries
                .keys()
                .all(|site| site.module == Path::new("async.js"))
        );
        let entry_owner = &artifacts.members[&NodeId::Module("entry.js".into())];
        let a_owner = &artifacts.members[&NodeId::Module("islands/a.js".into())];
        assert_eq!(entry_owner, a_owner, "exclusive island must parent-hoist");
        compare(
            &execute(&modules, Path::new("entry.js")).unwrap(),
            &artifacts,
        )
        .unwrap();
    }

    #[test]
    fn shared_library_clustering_changes_actual_owners() {
        let modules = generate(&[1]);
        let graph = graph(&modules).unwrap();
        let (_, separate) = build(&graph, &modules, &OutputOptions::default()).unwrap();
        let (_, clustered) = build(
            &graph,
            &modules,
            &OutputOptions {
                min_chunk_bytes: 4096,
                ..OutputOptions::default()
            },
        )
        .unwrap();
        assert_ne!(separate.members, clustered.members);
        assert!(
            clustered.files.len() < separate.files.len(),
            "size policy must reduce physical chunks"
        );
        compare(
            &execute(&modules, Path::new("entry.js")).unwrap(),
            &clustered,
        )
        .unwrap();
    }
}
