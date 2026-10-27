//! Ori-compatible source strings, translation catalogs and pseudo-localization.
use super::*;
use ftree_cache::inputs::{InputKind, InputSnapshot, PathInput};

/// Localization applies only to `*.locstring.json`, never ordinary JSON.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocaleOptions {
    /// Omitted/empty/`none` keeps source values; pseudo aliases select pseudo-localization.
    pub default: String,
    pub ploc: PseudoLocaleOptions,
}

/// Omitted fields use Ori defaults. Empty prefixes/suffixes are intentional.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PseudoLocaleOptions {
    pub prepend: Option<String>,
    pub append: Option<String>,
    pub start_delimiter: Option<String>,
    pub end_delimiter: Option<String>,
    pub extend: Option<f64>,
    #[serde(rename = "override")]
    pub override_: Option<String>,
    pub omit_keys: BTreeSet<String>,
}

impl LocaleOptions {
    pub fn validate(&self) -> eyre::Result<()> {
        ensure!(
            self.default.is_empty()
                || (self.default.len() <= 128
                    && self
                        .default
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))),
            "locale must be a single nonempty ASCII locale name (letters, digits, '-' or '_')"
        );
        ensure!(
            self.ploc.extend.is_none_or(f64::is_finite),
            "locale.ploc.extend must be finite"
        );
        Ok(())
    }

    fn pseudo(&self) -> bool {
        matches!(
            self.default.to_ascii_lowercase().as_str(),
            "qps-ploc"
                | "ploc"
                | "pseudo"
                | "pseudo-loc"
                | "pseudo-locale"
                | "pseudo-locstring"
                | "pseudo-locstrings"
                | "pseudoloc"
                | "pseudolocale"
                | "pseudolocstring"
                | "pseudolocstrings"
        )
    }
}

pub(super) fn matches(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(".locstring.json"))
}

pub(super) struct Localization {
    options: LocaleOptions,
    fingerprint: Fingerprint,
    translations: BTreeMap<String, String>,
    inputs: Vec<PathInput>,
    catalog: Option<(PathBuf, BTreeSet<PathBuf>)>,
    pub warnings: Vec<String>,
}

impl Localization {
    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn new(inventory: &Inventory, options: &LocaleOptions) -> eyre::Result<Self> {
        options.validate()?;
        let mut result = Self {
            options: options.clone(),
            fingerprint: Fingerprint::of(&serde_json::to_vec(options)?),
            translations: BTreeMap::new(),
            inputs: Vec::new(),
            catalog: None,
            warnings: Vec::new(),
        };
        if options.default.is_empty() || options.default == "none" || options.pseudo() {
            return Ok(result);
        }
        let directory = inventory.root().join("locale").join(&options.default);
        let paths = catalog_paths(&directory, inventory.snapshot(), &mut result.inputs)?;
        let mut origins = BTreeMap::new();
        let mut identities = Vec::new();
        for path in &paths {
            let canonical = inventory.admit(path)?;
            let (input, bytes) = inventory
                .snapshot()
                .read(&canonical)
                .wrap_err_with(|| format!("reading localization catalog {}", path.display()))?;
            identities.push((
                path.strip_prefix(inventory.root())?.to_owned(),
                input.content_seal().copied(),
            ));
            result.inputs.push(input);
            let values = parse(
                &canonical,
                bytes
                    .as_deref()
                    .ok_or_else(|| eyre::eyre!("missing catalog {}", path.display()))?,
            )?;
            for (key, value) in values {
                if let Some(previous) = origins.insert(key.clone(), path.clone()) {
                    eyre::bail!(
                        "duplicate localization catalog key {key:?} in {} and {}",
                        previous.display(),
                        path.display()
                    );
                }
                result.translations.insert(key, value);
            }
        }
        result.fingerprint = Fingerprint::of(&serde_json::to_vec(&(options, identities))?);
        if paths.is_empty() {
            result.warnings.push(format!(
                "no localization catalogs matched {}\\**.locstrings.json",
                directory.display()
            ));
        } else if result.translations.is_empty() {
            result.warnings.push(format!(
                "localization catalogs contained no translations in {}",
                directory.display()
            ));
        }
        result.catalog = Some((directory, paths));
        Ok(result)
    }

    pub fn recheck(&self) -> eyre::Result<()> {
        let snapshot = InputSnapshot::default();
        for input in &self.inputs {
            snapshot.validate(input)?;
        }
        if let Some((directory, paths)) = &self.catalog {
            ensure!(
                &catalog_paths(directory, &snapshot, &mut Vec::new())? == paths,
                "localization catalog candidates changed during build: {}",
                directory.display()
            );
        }
        Ok(())
    }

    pub fn source(
        &self,
        path: &Path,
        input: &[u8],
        warnings: &mut Vec<String>,
    ) -> eyre::Result<String> {
        let values = parse(path, input)?;
        let mut localized = BTreeMap::new();
        for (key, value) in &values {
            if key.starts_with('_') {
                continue;
            }
            if !values.contains_key(&format!("_{key}.comment")) {
                warnings.push(format!(
                    "locstring file '{}' was missing the expected translation comment key {:?}",
                    path.display(),
                    format!("_{key}.comment")
                ));
            }
            let value = if self.options.pseudo() && !self.options.ploc.omit_keys.contains(key) {
                pseudo(value, &self.options.ploc).wrap_err_with(|| {
                    format!("pseudo-localizing string {key:?} in {}", path.display())
                })?
            } else {
                self.translations
                    .get(key)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(value)
                    .clone()
            };
            localized.insert(key, value);
        }
        // Literal-only bindings and the default object are independently prunable.
        // Computed property keys preserve own __proto__ data, and string export
        // aliases avoid reserved/nonidentifier source names.
        let mut source = format!(
            "// localization {:?} input {:?}\n",
            self.fingerprint(),
            Fingerprint::of(input)
        );
        for (index, (key, value)) in localized.iter().enumerate() {
            if key.as_str() != "default" {
                source.push_str(&format!(
                    "const loc_{index} = {};\nexport {{loc_{index} as {}}};\n",
                    serde_json::to_string(value)?,
                    serde_json::to_string(key)?
                ));
            }
        }
        source.push_str("const loc_default = /*#__PURE__*/ (() => ({");
        for (key, value) in localized {
            source.push_str(&format!(
                "[{}]:{},",
                serde_json::to_string(key)?,
                serde_json::to_string(&value)?
            ));
        }
        source.push_str("}))();\nexport {loc_default as default};\n");
        Ok(source)
    }
}

fn parse(path: &Path, bytes: &[u8]) -> eyre::Result<BTreeMap<String, String>> {
    serde_json::from_slice(bytes)
        .wrap_err_with(|| format!("parsing localization string object {}", path.display()))
}

fn catalog_paths(
    directory: &Path,
    snapshot: &InputSnapshot,
    inputs: &mut Vec<PathInput>,
) -> eyre::Result<BTreeSet<PathBuf>> {
    fn visit(
        directory: &Path,
        snapshot: &InputSnapshot,
        inputs: &mut Vec<PathInput>,
        active: &mut BTreeSet<PathBuf>,
        paths: &mut BTreeSet<PathBuf>,
    ) -> eyre::Result<()> {
        let input = snapshot.capture(directory)?;
        ensure!(
            input.kind() == InputKind::Directory,
            "localization locale directory is missing or not a directory: {}",
            directory.display()
        );
        let canonical = input.canonical().expect("directory identity").to_owned();
        ensure!(
            active.insert(canonical.clone()),
            "cyclic localization catalog directory: {}",
            directory.display()
        );
        inputs.push(input);
        let mut entries = fs::read_dir(directory)
            .wrap_err_with(|| {
                format!(
                    "enumerating localization catalogs in {}",
                    directory.display()
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .wrap_err_with(|| {
                format!(
                    "reading localization catalog entries in {}",
                    directory.display()
                )
            })?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let kind = entry
                .file_type()
                .wrap_err_with(|| format!("inspecting localization catalog {}", path.display()))?;
            if kind.is_dir() {
                visit(&path, snapshot, inputs, active, paths)?;
            } else if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".locstrings.json"))
            {
                ensure!(
                    kind.is_file(),
                    "localization catalog must be a regular file: {}",
                    path.display()
                );
                paths.insert(path);
            } else if kind.is_symlink() {
                eyre::bail!(
                    "localization catalog directories cannot contain symbolic links: {}",
                    path.display()
                );
            }
        }
        active.remove(&canonical);
        Ok(())
    }
    let mut paths = BTreeSet::new();
    visit(
        directory,
        snapshot,
        inputs,
        &mut BTreeSet::new(),
        &mut paths,
    )?;
    Ok(paths)
}

// Ported from Microsoft Ori pseudoloc/pseudoloc.go (MIT); see
// localization.LICENSE.txt. The accent table originates in bunkat/pseudoloc.
fn pseudo(input: &str, options: &PseudoLocaleOptions) -> eyre::Result<String> {
    let padding = options.extend.unwrap_or(0.5).max(0.0) * input.len() as f64;
    ensure!(
        padding.is_finite() && padding < usize::MAX as f64,
        "pseudo-localization padding length is not representable"
    );
    let padding = padding as usize;
    let prepend = options.prepend.as_deref().unwrap_or("[[");
    let append = options.append.as_deref().unwrap_or("]]");
    let mut output = pseudo_buffer([
        prepend.chars().count(),
        input.chars().count(),
        padding,
        append.chars().count(),
    ])?;
    output.extend(prepend.chars());
    let start: Vec<_> = options
        .start_delimiter
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("{")
        .chars()
        .collect();
    let end: Vec<_> = options
        .end_delimiter
        .as_deref()
        .unwrap_or("}")
        .chars()
        .collect();
    let overrides: Vec<_> = options.override_.as_deref().unwrap_or("").chars().collect();
    let mut escaped = false;
    let mut count = 0;
    for (offset, ch) in input.char_indices() {
        if escaped {
            output.push(ch);
            let matched;
            (count, matched) = match_sequence(ch, &end, count);
            if matched {
                escaped = false;
            }
            continue;
        }
        let matched;
        (count, matched) = match_sequence(ch, &start, count);
        if matched {
            escaped = true;
            output.truncate(output.len() - (start.len() - 1));
            output.extend(&start);
        } else if !overrides.is_empty() {
            output.push(overrides[offset % overrides.len()]);
        } else {
            let table = accents(ch);
            let length = table.chars().count();
            output.push(if length == 0 {
                ch
            } else {
                table.chars().nth(offset % length).expect("table index")
            });
        }
    }
    output.extend(std::iter::repeat_n(' ', padding));
    output.extend(append.chars());
    let bytes = output.iter().try_fold(0usize, |length, ch| {
        length
            .checked_add(ch.len_utf8())
            .ok_or_else(|| eyre::eyre!("pseudo-localization UTF-8 capacity overflow"))
    })?;
    let mut text = String::new();
    text.try_reserve_exact(bytes)
        .wrap_err("reserving pseudo-localization UTF-8 capacity")?;
    text.extend(output);
    Ok(text)
}

fn pseudo_buffer(lengths: [usize; 4]) -> eyre::Result<Vec<char>> {
    let capacity = lengths.into_iter().try_fold(0usize, |total, length| {
        total
            .checked_add(length)
            .ok_or_else(|| eyre::eyre!("pseudo-localization character capacity overflow"))
    })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(capacity)
        .wrap_err("reserving pseudo-localization character capacity")?;
    Ok(output)
}

fn match_sequence(ch: char, sequence: &[char], count: usize) -> (usize, bool) {
    if sequence.is_empty() {
        return (0, false);
    }
    if count < sequence.len() && ch == sequence[count] {
        if count + 1 == sequence.len() {
            (0, true)
        } else {
            (count + 1, false)
        }
    } else if ch == sequence[0] {
        (1, sequence.len() == 1)
    } else {
        (0, false)
    }
}

fn accents(ch: char) -> &'static str {
    match ch {
        'A' => "ÀÁÂÃÄÅĀĂĄǺȀȂ",
        'a' => "àáâãäåāăąǻȁȃ",
        'B' => "ßƁɃʙ",
        'b' => "ƀƂƃƄƅɓ",
        'C' => "ĆĈĊČ",
        'c' => "ćĉċč",
        'D' => "ĎĐƉƊ",
        'd' => "ďđ",
        'E' => "ĒĔĖĘĚȄȆ",
        'e' => "ēĕėęěȅȇ",
        'F' => "Ƒ",
        'f' => "ƒ",
        'G' => "ĜĞĠĢ",
        'g' => "ĝğġģ",
        'H' => "ĤĦ",
        'h' => "ĥħ",
        'I' => "ĨĪĬĮİ",
        'i' => "ĩīĭįı",
        'J' => "ĵ",
        'j' => "Ĵ",
        'K' => "ĶƘ",
        'k' => "ķĸƙ",
        'L' => "ĹĻĽĿŁ",
        'l' => "ĺļľŀł",
        'N' => "ŃŅŇŊƝ",
        'n' => "ńņňŉŋƞ",
        'O' => "ŌŎŐƠ",
        'o' => "ōŏőơ",
        'P' => "Ƥ",
        'p' => "ƥƿ",
        'Q' => "ǪǬ",
        'q' => "ǫǭɋ",
        'R' => "ŔŖŘƦ",
        'r' => "ŕŗř",
        'S' => "ŚŜŞŠ",
        's' => "śŝşš",
        'T' => "ŢŤŦ",
        't' => "ţťŧ",
        'U' => "ŨŪŬŮŰŲ",
        'u' => "ũūŭůűų",
        'W' => "Ŵ",
        'w' => "ŵ",
        'Y' => "ŶŸƳȲɎ",
        'y' => "ŷƴȳɏ",
        'Z' => "ŹŻŽƵ",
        'z' => "źżžƶ",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localization_pseudo_capacity_arithmetic_is_checked_without_allocation() {
        let overflow = pseudo_buffer([usize::MAX, 1, 0, 0]).unwrap_err();
        assert!(format!("{overflow:#}").contains("character capacity overflow"));
        let overflow =
            pseudo_buffer([0, isize::MAX as usize / size_of::<char>(), 1, 0]).unwrap_err();
        assert!(overflow
            .downcast_ref::<std::collections::TryReserveError>()
            .is_some());
        assert!(
            format!("{overflow:#}").contains("reserving pseudo-localization character capacity")
        );
    }

    #[test]
    fn localization_pseudo_matches_original_go_oracle() {
        // Synthetic cross-product evaluated by Ori 050d85e7's actual Go
        // StringOption/JSONOpts.Convert and rewindable writer, without changes
        // to their algorithms. See localization.LICENSE.txt for attribution.
        let cases: Vec<(String, PseudoLocaleOptions, String)> =
            serde_json::from_str(include_str!("tests/localization_pseudo.json")).unwrap();
        for (input, options, expected) in cases {
            assert_eq!(
                pseudo(&input, &options).unwrap(),
                expected,
                "{input:?}: {options:?}"
            );
        }
    }
}
