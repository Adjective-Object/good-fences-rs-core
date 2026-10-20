use std::{fs, path::Path};

use eyre::{ensure, WrapErr};
use ftree_cache::context_data::ContextData;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompilerOptions {
    #[serde(default, alias = "customSourceConditions")]
    custom_conditions: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tsconfig {
    #[serde(default)]
    compiler_options: CompilerOptions,
}

/// Read direct export conditions only; paths/baseUrl do not rewrite runtime requests.
pub fn read_custom_conditions(path: &Path) -> eyre::Result<Vec<String>> {
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path)?)
        .wrap_err_with(|| format!("{}: tsconfig requires strict JSON", path.display()))?;
    ensure!(
        json.get("extends").is_none() && json.get("references").is_none(),
        "{}: inherited/project-reference tsconfig conditions are unsupported",
        path.display()
    );
    let config: Tsconfig = serde_json::from_value(json)
        .wrap_err_with(|| format!("{}: invalid tsconfig export conditions", path.display()))?;
    ensure!(
        config
            .compiler_options
            .custom_conditions
            .iter()
            .all(|condition| !condition.is_empty()),
        "{}: export conditions must not be empty",
        path.display()
    );
    Ok(config.compiler_options.custom_conditions)
}

#[derive(Debug)]
pub(super) struct TsconfigConditions(pub Vec<String>);

impl ContextData for TsconfigConditions {
    fn read_context_data(_: (), path: &Path) -> anyhow::Result<Option<Self>> {
        if !path.try_exists()? {
            return Ok(None);
        }
        read_custom_conditions(path)
            .map(|conditions| Some(Self(conditions)))
            .map_err(|error| anyhow::Error::from_boxed(error.into()))
    }
}
