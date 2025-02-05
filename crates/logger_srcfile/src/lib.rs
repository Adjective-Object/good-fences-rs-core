use std::{borrow::Borrow, fmt::Display, path::Path};

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

pub trait SrcFileLogger: Logger {
    fn src_warn(&self, location: &Span, message: impl Display);
    fn src_error(&self, location: &Span, message: impl Display);
}

#[derive(Clone)]
pub struct WrapFileLogger<TSrcMap, TLogger> {
    source_map: TSrcMap,
    inner_logger: TLogger,
}
impl<TSourceMap: Borrow<SourceMap> + Clone, TLogger: Logger> WrapFileLogger<TSourceMap, TLogger> {
    pub fn new(source_map: TSourceMap, inner_logger: TLogger) -> Self {
        Self {
            source_map,
            inner_logger,
        }
    }
}
impl<TSourceMap: Borrow<SourceMap> + Clone, TLogger: Logger> Logger
    for WrapFileLogger<TSourceMap, TLogger>
{
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
impl<TSourceMap: Borrow<SourceMap> + Clone, TLogger: Logger> HasSourceMap
    for WrapFileLogger<TSourceMap, TLogger>
{
    fn source_map(&self) -> &SourceMap {
        self.source_map.borrow()
    }
}
impl<TSourceMap: Borrow<SourceMap> + Clone, TLogger: Logger> SrcFileLogger
    for WrapFileLogger<TSourceMap, TLogger>
{
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
    fn src_warn(&self, _location: &Span, message: impl Display) {
        self.warn(format!(
            "{} :: {}",
            self.source_file_path.display(),
            message,
        ));
    }
    fn src_error(&self, _location: &Span, message: impl Display) {
        self.error(format!(
            "{} :: {}",
            self.source_file_path.display(),
            message,
        ));
    }
}
