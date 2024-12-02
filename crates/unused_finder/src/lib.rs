extern crate import_resolver;
extern crate serde_json;
extern crate tsconfig_paths;

#[macro_use]
extern crate anyhow;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

#[cfg(test)]
extern crate test_tmpdir;

mod cfg;
mod graph;
mod ignore_file;
mod parse;
mod report;
mod tag;
#[cfg(test)]
mod test;
mod unused_finder;
mod walk;
mod walked_file;

pub use cfg::{UnusedFinderConfig, UnusedFinderJSONConfig};
pub use parse::data::ResolvedImportExportInfo;
pub use report::{SymbolReport, SymbolReportWithTags, UnusedFinderReport};
pub use tag::UsedTagEnum;
pub use unused_finder::{UnusedFinder, UnusedFinderResult};

pub fn find_unused_items(
    logger: impl logger::Logger + Sync,
    config: UnusedFinderJSONConfig,
) -> Result<UnusedFinderReport, js_err::JsErr> {
    let mut finder = UnusedFinder::new_from_json_config(&logger, config)
        .map_err(js_err::JsErr::generic_failure)?;
    let result = finder
        .find_unused(&logger)
        .map_err(js_err::JsErr::generic_failure)?;
    Ok(result.get_report())
}
