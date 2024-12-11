use std::fmt::Display;

use logger::Logger;
use swc_common::{Loc, SourceMap, Span};

pub trait SrcLogger: Logger {
    fn src_warn(&self, loc: &Loc, message: impl Display) {
        self.warn(format!(
            "{}:{}:{} :: {}",
            loc.file.name, loc.line, loc.col_display, message,
        ));
    }
    fn src_error(&self, loc: &Loc, message: impl Display) {
        self.error(format!(
            "{}:{}:{} :: {}",
            loc.file.name, loc.line, loc.col_display, message,
        ));
    }
}

pub trait HasSourceMap {
    fn source_map(&self) -> &SourceMap;
}

pub trait SrcFileLogger: Logger + HasSourceMap {
    fn src_warn(&self, location: &Span, message: impl Display) {
        let loc = self.source_map().lookup_char_pos(location.lo);
        self.warn(format!(
            "{}:{}:{} :: {}",
            loc.file.name, loc.line, loc.col_display, message,
        ));
    }
    fn src_error(&self, location: &Span, message: impl Display) {
        let loc = self.source_map().lookup_char_pos(location.lo);
        self.error(format!(
            "{}:{}:{} :: {}",
            loc.file.name, loc.line, loc.col_display, message,
        ));
    }
}

#[derive(Clone)]
pub struct WrapFileLogger<'a, TLogger: Logger> {
    source_map: &'a SourceMap,
    inner_logger: TLogger,
}
impl<'a, TLogger: Logger> WrapFileLogger<'a, TLogger> {
    pub fn new(source_map: &'a SourceMap, inner_logger: TLogger) -> Self {
        Self {
            source_map,
            inner_logger,
        }
    }
}
impl<TLogger: Logger> Logger for WrapFileLogger<'_, TLogger> {
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
impl<TLogger: Logger> HasSourceMap for WrapFileLogger<'_, TLogger> {
    fn source_map(&self) -> &SourceMap {
        self.source_map
    }
}
impl<TLogger: Logger> SrcFileLogger for WrapFileLogger<'_, TLogger> {}
