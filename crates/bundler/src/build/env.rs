use super::*;

pub(super) fn path(root: &Path) -> PathBuf {
    root.join(".bundler-env.mjs")
}

pub(super) struct Module {
    pub path: PathBuf,
    pub text: crate::prepared_text::Text,
    pub record: CacheRecord,
}

pub(super) fn source(fields: &BTreeMap<String, serde_json::Value>) -> eyre::Result<String> {
    fn value(input: &serde_json::Value) -> eyre::Result<String> {
        Ok(match input {
            serde_json::Value::Object(fields) => {
                let fields: BTreeMap<_, _> = fields.iter().collect();
                format!(
                    "{{{}}}",
                    fields
                        .into_iter()
                        .map(|(key, item)| {
                            Ok(format!(
                                "[{}]:{}",
                                serde_json::to_string(key)?,
                                value(item)?
                            ))
                        })
                        .collect::<eyre::Result<Vec<_>>>()?
                        .join(",")
                )
            }
            serde_json::Value::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(value)
                    .collect::<eyre::Result<Vec<_>>>()?
                    .join(",")
            ),
            scalar => serde_json::to_string(scalar)?,
        })
    }
    let fields = fields
        .iter()
        .map(|(key, item)| {
            Ok(format!(
                "[{}]:{}",
                serde_json::to_string(key)?,
                value(item)?
            ))
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    Ok(format!(
        "export const import_meta_env = {{{}}};\n",
        fields.join(",")
    ))
}

impl Module {
    pub fn new(
        root: &Path,
        source: &str,
        declarations: &PreparedDeclarations,
    ) -> eyre::Result<Self> {
        let path = path(root);
        let parsed = ast_segmenter::parse_module_with_prepared_declarations(
            &logger::StdioLogger::new(),
            source,
            &path,
            ModuleFormat::Esm,
            declarations,
        )
        .wrap_err("extracting configured import.meta.env segment")?;
        Ok(Self {
            path,
            text: crate::prepared_text::Text::snapshot(source)
                .wrap_err("snapshotting configured import.meta.env segment")?,
            record: CacheRecord::from_parsed(source, &parsed)
                .wrap_err("sealing configured import.meta.env segment")?,
        })
    }

    pub fn input(&self) -> eyre::Result<OwnedSourceFileInput> {
        Ok(OwnedSourceFileInput::from_parsed(
            self.path.clone(),
            self.record.to_parsed(&self.text)?,
            Default::default(),
        ))
    }
}

pub(super) struct Texts<'a> {
    pub sources: &'a BTreeMap<PathBuf, Source>,
    pub env: Option<&'a Module>,
}

impl link::SourceTexts for Texts<'_> {
    fn text(&self, path: &Path) -> eyre::Result<Option<crate::prepared_text::Text>> {
        if let Some(env) = self.env.filter(|env| env.path == path) {
            return Ok(Some(env.text.clone()));
        }
        link::SourceTexts::text(self.sources, path)
    }
}
