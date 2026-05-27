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

fn compute_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

pub struct WrapFileLogger<TLogger> {
    named_source: NamedSource<String>,
    inner_logger: TLogger,
    line_starts: OnceCell<Vec<u32>>,
}

impl<TLogger: Clone> Clone for WrapFileLogger<TLogger> {
    fn clone(&self) -> Self {
        Self {
            named_source: NamedSource::new(
                self.named_source.name(),
                self.named_source.inner().clone(),
            ),
            inner_logger: self.inner_logger.clone(),
            line_starts: self.line_starts.clone(),
        }
    }
}

impl<TLogger: Logger> WrapFileLogger<TLogger> {
    pub fn new(filename: impl AsRef<str>, source: String, inner_logger: TLogger) -> Self {
        Self {
            named_source: NamedSource::new(filename, source),
            inner_logger,
            line_starts: OnceCell::new(),
        }
    }

    /// Returns (1-based line, 0-based col_display) matching swc's `lookup_char_pos` format.
    fn line_col(&self, byte_offset: u32) -> (usize, usize) {
        let starts = self
            .line_starts
            .get_or_init(|| compute_line_starts(self.named_source.inner()));
        let line_idx = match starts.binary_search(&byte_offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = (byte_offset - starts[line_idx]) as usize;
        (line_idx + 1, col)
    }

    pub fn filename(&self) -> &str {
        self.named_source.name()
    }
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
            message,
        ));
    }
    fn src_error(&self, location: Span, message: impl Display) {
        let (line, col) = self.line_col(location.start);
        self.error(format!(
            "{}:{}:{} :: {}",
            self.named_source.name(),
            line,
            col,
            message,
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
            "{}:byte={} :: {}",
            self.source_file_path.display(),
            location.start,
            message,
        ));
    }
    fn src_error(&self, location: Span, message: impl Display) {
        self.error(format!(
            "{}:byte={} :: {}",
            self.source_file_path.display(),
            location.start,
            message,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct CollectLogger {
        messages: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl CollectLogger {
        fn new() -> Self {
            Self {
                messages: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            }
        }
        fn messages(&self) -> Vec<String> {
            self.messages.borrow().clone()
        }
    }

    impl Logger for CollectLogger {
        fn log(&self, message: impl Display) {
            self.messages
                .borrow_mut()
                .push(format!("LOG: {}", message));
        }
        fn warn(&self, message: impl Display) {
            self.messages
                .borrow_mut()
                .push(format!("WARN: {}", message));
        }
        fn error(&self, message: impl Display) {
            self.messages
                .borrow_mut()
                .push(format!("ERR: {}", message));
        }
    }

    #[test]
    fn line_col_lf_endings() {
        // "abc\ndef\nghi" — lines at offsets 0, 4, 8
        let logger = WrapFileLogger::new("test.ts", "abc\ndef\nghi".to_string(), CollectLogger::new());
        // 'a' at offset 0 → line 1, col 0
        assert_eq!(logger.line_col(0), (1, 0));
        // 'd' at offset 4 → line 2, col 0
        assert_eq!(logger.line_col(4), (2, 0));
        // 'e' at offset 5 → line 2, col 1
        assert_eq!(logger.line_col(5), (2, 1));
        // 'g' at offset 8 → line 3, col 0
        assert_eq!(logger.line_col(8), (3, 0));
        // 'i' at offset 10 → line 3, col 2
        assert_eq!(logger.line_col(10), (3, 2));
    }

    #[test]
    fn line_col_crlf_endings() {
        // "ab\r\ncd\r\ne" — \n at offsets 3 and 7, line starts: 0, 4, 8
        let logger = WrapFileLogger::new("test.ts", "ab\r\ncd\r\ne".to_string(), CollectLogger::new());
        assert_eq!(logger.line_col(0), (1, 0)); // 'a'
        assert_eq!(logger.line_col(4), (2, 0)); // 'c'
        assert_eq!(logger.line_col(5), (2, 1)); // 'd'
        assert_eq!(logger.line_col(8), (3, 0)); // 'e'
    }

    #[test]
    fn src_error_format() {
        let collect = CollectLogger::new();
        let logger = WrapFileLogger::new("foo.ts", "abc\ndef\n".to_string(), &collect);
        logger.src_error(Span::new(4, 7), "bad token");
        let msgs = collect.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "LOG: ERROR: foo.ts:2:0 :: bad token");
    }

    #[test]
    fn simple_logger_format() {
        let collect = CollectLogger::new();
        let logger = SimpleSourceFileLogger::new(Path::new("bar.ts"), &collect);
        logger.src_warn(Span::new(42, 50), "something");
        let msgs = collect.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], "LOG: WARN: bar.ts:byte=42 :: something");
    }
}

