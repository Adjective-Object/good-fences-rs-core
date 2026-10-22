//! Source-local byte provenance becomes UTF-16 mappings only at final assembly.
use std::{collections::HashMap, path::Path, sync::Arc};

use cache_flatbuffers::WireSourceMap;
use oxc_sourcemap::{SourceMap, SourceMapBuilder};
use oxc_span::Span;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Source {
    url: String,
    text: Arc<str>,
    checkpoints: Vec<(usize, Position)>,
    upstream: Option<WireSourceMap>,
}

impl Source {
    #[cfg(test)]
    pub(crate) fn new(path: &Path, text: &str) -> eyre::Result<Arc<Self>> {
        Self::with_upstream(path, text, None)
    }

    pub(crate) fn with_upstream(
        path: &Path,
        text: &str,
        upstream: Option<WireSourceMap>,
    ) -> eyre::Result<Arc<Self>> {
        let url = url::Url::from_file_path(path).map_err(|()| {
            eyre::eyre!(
                "source map requires an absolute file path: {}",
                path.display()
            )
        })?;
        let mut position = Position::default();
        let mut checkpoints = vec![(0, position)];
        for (offset, ch) in text.char_indices() {
            if offset - checkpoints.last().unwrap().0 >= 256 {
                checkpoints.push((offset, position));
            }
            position.advance(ch);
        }
        Ok(Arc::new(Self {
            url: url.into(),
            text: text.into(),
            checkpoints,
            upstream,
        }))
    }

    pub(crate) fn slice(&self, span: Span) -> &str {
        &self.text[span.start as usize..span.end as usize]
    }

    fn position(&self, offset: usize) -> Position {
        let index = self
            .checkpoints
            .partition_point(|&(start, _)| start <= offset)
            - 1;
        let (start, mut position) = self.checkpoints[index];
        for ch in self.text[start..offset].chars() {
            position.advance(ch);
        }
        position
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Position {
    line: u32,
    column: u32,
    after_cr: bool,
}

impl Position {
    fn advance(&mut self, ch: char) {
        match ch {
            '\n' if self.after_cr => {}
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                self.line += 1;
                self.column = 0;
            }
            _ => self.column += ch.len_utf16() as u32,
        }
        self.after_cr = ch == '\r';
    }
}

#[derive(Default)]
pub(crate) struct Writer {
    text: String,
    position: Position,
    map: Option<SourceMapBuilder>,
    contents: HashMap<String, Option<Arc<str>>>,
}

impl Writer {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            map: enabled.then(SourceMapBuilder::default),
            ..Self::default()
        }
    }

    pub(crate) fn generated(&mut self, text: &str) {
        self.push(text, None);
    }

    pub(crate) fn source(&mut self, source: &Source, span: Span) {
        self.push(source.slice(span), Some((source, span.start, true, None)));
    }

    pub(crate) fn named(&mut self, text: &str, source: &Source, offset: u32, name: &str) {
        self.push(text, Some((source, offset, false, Some(name))));
    }

    pub(crate) fn replacement(&mut self, text: &str, source: &Source, offset: u32) {
        self.push(text, Some((source, offset, false, None)));
    }

    fn push(&mut self, text: &str, origin: Option<(&Source, u32, bool, Option<&str>)>) {
        if let Some(map) = &mut self.map {
            let mut original = origin
                .map(|(source, offset, _, _)| source.position(offset as usize))
                .unwrap_or_default();
            let exact = origin.is_some_and(|(_, _, exact, _)| exact);
            let mut needs_mapping = true;
            for ch in text.chars() {
                let newline = matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}');
                if !newline && (exact || needs_mapping) {
                    let mapped = origin.and_then(|(source, _, _, name)| {
                        if let Some(upstream) = &source.upstream {
                            let token = upstream.lookup(original.line, original.column)?;
                            if token.source() < 0 {
                                return None;
                            }
                            let root = upstream.root();
                            let input = root.sources().get(token.source() as usize);
                            let name = if token.name() < 0 {
                                name
                            } else {
                                Some(root.names().get(token.name() as usize))
                            };
                            Some((
                                input.url(),
                                input.content(),
                                token.original_line(),
                                token.original_column(),
                                name,
                            ))
                        } else {
                            Some((
                                source.url.as_str(),
                                Some(source.text.as_ref()),
                                original.line,
                                original.column,
                                name,
                            ))
                        }
                    });
                    let source_id = mapped.map(|(url, content, _, _, _)| {
                        let stored = self.contents.entry(url.to_owned()).or_default();
                        if stored.is_none() {
                            *stored = content.map(Arc::from);
                        }
                        map.add_source_and_content(url, content.unwrap_or(""))
                    });
                    let name_id =
                        mapped.and_then(|(_, _, _, _, name)| name.map(|name| map.add_name(name)));
                    let (line, column) =
                        mapped.map_or((0, 0), |(_, _, line, column, _)| (line, column));
                    map.add_token(
                        self.position.line,
                        self.position.column,
                        line,
                        column,
                        source_id,
                        name_id,
                    );
                }
                self.position.advance(ch);
                if exact {
                    original.advance(ch);
                }
                needs_mapping = newline;
            }
        }
        self.text.push_str(text);
    }

    pub(crate) fn finish(self, destination: &Path) -> (String, Option<SourceMap>) {
        let map = self.map.map(|mut builder| {
            builder.set_file(&destination.file_name().unwrap().to_string_lossy());
            let mut map = builder.into_sourcemap();
            let contents = map
                .get_sources()
                .map(|source| {
                    self.contents
                        .get(source.as_ref())
                        .and_then(|content| content.as_deref())
                })
                .collect();
            map.set_source_contents(contents);
            map
        });
        (self.text, map)
    }
}

pub(crate) fn path(output: &Path) -> std::path::PathBuf {
    let mut path = output.as_os_str().to_owned();
    path.push(".map");
    path.into()
}

pub(crate) fn executable(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| matches!(ext.to_str(), Some("mjs" | "cjs" | "js")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_line_fragments_keep_utf16_positions_when_reordered() {
        let text = "😀é,x,".repeat(20_000);
        let source = Source::new(
            &std::env::current_dir().unwrap().join("minified.mjs"),
            &text,
        )
        .unwrap();
        let mut writer = Writer::new(true);
        for index in (0..20_000).rev() {
            let offset = (index * "😀é,x,".len() + "😀é,".len()) as u32;
            writer.source(&source, Span::new(offset, offset + 1));
            writer.generated("\n");
        }
        let (_, map) = writer.finish(Path::new("out.mjs"));
        let map = map.unwrap();
        let table = map.generate_lookup_table();
        for line in [0, 1, 10_000, 19_999] {
            let token = map.lookup_source_view_token(&table, line, 0).unwrap();
            assert_eq!(token.get_src_line(), 0);
            assert_eq!(token.get_src_col(), (19_999 - line) * 6 + 4);
        }
    }

    #[test]
    fn utf16_coordinates_and_generated_boundaries_survive_fragment_moves() {
        let text = "α😀z\r\nb\rc\nd\u{2028}e\u{2029}f";
        let source =
            Source::new(&std::env::current_dir().unwrap().join("unicode.mjs"), text).unwrap();
        let mut writer = Writer::new(true);
        writer.generated("😀 ");
        writer.source(&source, Span::new(0, text.len() as u32));
        writer.generated(" synthetic");
        writer.generated("\r");
        writer.generated("\n");
        let offset = text.find('z').unwrap() as u32;
        writer.named("renamed", &source, offset, "z");
        writer.generated(";");
        let (_, map) = writer.finish(Path::new("out.mjs"));
        let map = SourceMap::from_json_string(&map.unwrap().to_json_string()).unwrap();
        let table = map.generate_lookup_table();
        for (line, column, original_line, original_column) in [
            (0, 3, 0, 0),
            (0, 4, 0, 1),
            (0, 6, 0, 3),
            (1, 0, 1, 0),
            (2, 0, 2, 0),
            (3, 0, 3, 0),
            (4, 0, 4, 0),
            (5, 0, 5, 0),
            (6, 0, 0, 3),
        ] {
            let token = map.lookup_source_view_token(&table, line, column).unwrap();
            assert_eq!(
                (token.get_src_line(), token.get_src_col()),
                (original_line, original_column)
            );
            assert_eq!(token.get_source_content().unwrap().as_ref(), text);
        }
        for (line, column) in [(0, 0), (5, 1), (6, 7)] {
            assert!(map
                .lookup_source_view_token(&table, line, column)
                .unwrap()
                .get_source()
                .is_none());
        }
        assert_eq!(
            map.lookup_source_view_token(&table, 6, 0)
                .unwrap()
                .get_name()
                .unwrap()
                .as_ref(),
            "z"
        );
        assert_eq!(map.get_file().unwrap().as_ref(), "out.mjs");
    }
}
