use std::fmt::Display;
use std::path::Path;

use logger::Logger;
use once_cell::sync::OnceCell;
use oxc_diagnostics::NamedSource;
use oxc_span::Span;

pub trait SrcFileLogger: Logger {
    fn src_warn(&self, location: Span, message: impl Display);
    fn src_error(&self, location: Span, message: impl Display);
}

#[derive(Clone)]
pub struct WrapFileLogger<TLogger> {
    named_source: NamedSource<String>,
    inner_logger: TLogger,
    line_starts: OnceCell<Vec<u32>>,
}

impl<TLogger: Logger> WrapFileLogger<TLogger> {
    pub fn new(filename: impl AsRef<str>, source: String, inner_logger: TLogger) -> Self {
        Self {
            named_source: NamedSource::new(filename, source),
            inner_logger,
            line_starts: OnceCell::new(),
        }
    }

    /// Returns (1-indexed line, 0-indexed column) for a byte offset in the source.
    fn line_col(&self, byte_offset: u32) -> (usize, usize) {
        let starts = self
            .line_starts
            .get_or_init(|| compute_line_starts(self.named_source.inner()));
        match starts.binary_search(&byte_offset) {
            // Exact match: byte_offset is the first byte of a line.
            Ok(i) => (i + 1, 0),
            // byte_offset falls inside line `i` (0-indexed), which starts at starts[i-1].
            Err(i) => {
                let line = i;
                let col = (byte_offset - starts[i - 1]) as usize;
                (line, col)
            }
        }
    }
}

fn compute_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i as u32 + 1);
        }
    }
    starts
}

impl<TLogger: Logger> Logger for WrapFileLogger<TLogger> {
    fn log(&self, message: impl Display) {
        self.inner_logger.log(message);
    }
    fn error(&self, message: impl Display) {
        self.inner_logger.error(message);
    }
    fn warn(&self, message: impl Display) {
        self.inner_logger.warn(message);
    }
}

impl<TLogger: Logger> SrcFileLogger for WrapFileLogger<TLogger> {
    fn src_warn(&self, location: Span, message: impl Display) {
        let (line, col) = self.line_col(location.start);
        self.warn(format!(
            "{}:{}:{} :: {}",
            self.named_source.name(),
            line,
            col,
            message
        ));
    }
    fn src_error(&self, location: Span, message: impl Display) {
        let (line, col) = self.line_col(location.start);
        self.error(format!(
            "{}:{}:{} :: {}",
            self.named_source.name(),
            line,
            col,
            message
        ));
    }
}

#[derive(Clone)]
pub struct SimpleSourceFileLogger<'a, TLogger: Logger> {
    source_file_path: &'a Path,
    inner_logger: TLogger,
}

impl<'a, TLogger: Logger> SimpleSourceFileLogger<'a, TLogger> {
    pub fn new(source_file_path: &'a Path, inner_logger: TLogger) -> Self {
        Self {
            source_file_path,
            inner_logger,
        }
    }
}

impl<TLogger: Logger> Logger for SimpleSourceFileLogger<'_, TLogger> {
    fn log(&self, message: impl Display) {
        self.inner_logger.log(message);
    }
    fn error(&self, message: impl Display) {
        self.inner_logger.error(message);
    }
    fn warn(&self, message: impl Display) {
        self.inner_logger.warn(message);
    }
}

impl<TLogger: Logger> SrcFileLogger for SimpleSourceFileLogger<'_, TLogger> {
    fn src_warn(&self, location: Span, message: impl Display) {
        self.warn(format!(
            "{}:byte={}:: {}",
            self.source_file_path.display(),
            location.start,
            message
        ));
    }
    fn src_error(&self, location: Span, message: impl Display) {
        self.error(format!(
            "{}:byte={}:: {}",
            self.source_file_path.display(),
            location.start,
            message
        ));
    }
}

#[cfg(feature = "swc-compat")]
mod swc_compat_impl {
    use swc_common::{SourceFile, SourceMap};

    use super::{Logger, WrapFileLogger};

    impl<TLogger: Logger> WrapFileLogger<TLogger> {
        /// Construct a `WrapFileLogger` from a swc `SourceFile`.
        /// Used as a transition adapter while callers still use the swc parser.
        pub fn from_swc_source_file(
            _sm: &SourceMap,
            fm: &SourceFile,
            inner_logger: TLogger,
        ) -> Self {
            let filename = fm.name.to_string();
            let source = (*fm.src).clone();
            Self::new(filename, source, inner_logger)
        }
    }

    /// Convert a `swc_common::Span` to an `oxc_span::Span`.
    /// The byte positions are taken directly from the swc `BytePos` inner `u32` values.
    /// These are source-map-global offsets and may not be file-relative for
    /// multi-file source maps, but are adequate for diagnostic logging during
    /// the transition period.
    pub fn swc_span_to_oxc(span: swc_common::Span) -> oxc_span::Span {
        oxc_span::Span::new(span.lo.0, span.hi.0)
    }
}

#[cfg(feature = "swc-compat")]
pub use swc_compat_impl::swc_span_to_oxc;

#[cfg(test)]
mod tests {
    use super::*;
    use logger::StdioLogger;

    fn make_logger(src: &str) -> WrapFileLogger<StdioLogger> {
        WrapFileLogger::new("test.ts", src.to_string(), StdioLogger::new())
    }

    #[test]
    fn line_col_single_line() {
        let logger = make_logger("hello world");
        assert_eq!(logger.line_col(0), (1, 0));
        assert_eq!(logger.line_col(6), (1, 6));
    }

    #[test]
    fn line_col_lf_endings() {
        // Line 1: "abc\n"  bytes 0-3
        // Line 2: "def\n"  bytes 4-7
        // Line 3: "ghi"    bytes 8-10
        let logger = make_logger("abc\ndef\nghi");
        assert_eq!(logger.line_col(0), (1, 0)); // 'a'
        assert_eq!(logger.line_col(3), (1, 3)); // '\n'
        assert_eq!(logger.line_col(4), (2, 0)); // 'd'
        assert_eq!(logger.line_col(6), (2, 2)); // 'f'
        assert_eq!(logger.line_col(8), (3, 0)); // 'g'
    }

    #[test]
    fn line_col_crlf_endings() {
        // Line 1: "abc\r\n"  bytes 0-4, \n at byte 4
        // Line 2: "def\r\n"  bytes 5-9, \n at byte 9
        // Line 3: "ghi"
        let logger = make_logger("abc\r\ndef\r\nghi");
        assert_eq!(logger.line_col(0), (1, 0)); // 'a'
        assert_eq!(logger.line_col(4), (1, 4)); // '\n'
        assert_eq!(logger.line_col(5), (2, 0)); // 'd'
        assert_eq!(logger.line_col(7), (2, 2)); // 'f'
        assert_eq!(logger.line_col(10), (3, 0)); // 'g'
    }

    #[test]
    fn line_col_mixed_endings() {
        // "a\nb\r\nc"
        // Line 1: "a\n"       bytes 0-1, \n at 1
        // Line 2: "b\r\n"     bytes 2-4, \n at 4
        // Line 3: "c"         bytes 5
        let logger = make_logger("a\nb\r\nc");
        assert_eq!(logger.line_col(0), (1, 0));
        assert_eq!(logger.line_col(2), (2, 0)); // 'b'
        assert_eq!(logger.line_col(5), (3, 0)); // 'c'
    }

    #[cfg(feature = "swc-compat")]
    #[test]
    fn swc_span_to_oxc_round_trip() {
        use swc_common::BytePos;
        let swc_span = swc_common::Span::new(BytePos(3), BytePos(7));
        let oxc_span = swc_span_to_oxc(swc_span);
        assert_eq!(oxc_span, oxc_span::Span::new(3, 7));
    }
}
