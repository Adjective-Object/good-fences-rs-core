use napi_derive::napi;
use unused_finder::{
    logger::StdioLogger, UnusedFinder, UnusedFinderJSONConfig, UnusedFinderReport,
};

/**
 * Provides a napi wrapper for the UnusedFinder, which plumbs
 * the UnusedFinder's functionality to JavaScript.
 */
#[napi(js_name = "UnusedFinder")]
pub struct JsUnusedFinder {
    unused_finder: UnusedFinder,
}

#[napi]
impl JsUnusedFinder {
    #[napi(constructor)]
    pub fn new(config: UnusedFinderJSONConfig) -> napi::Result<Self> {
        let unused_finder = UnusedFinder::new_from_json_config(&StdioLogger::new(), config)?;
        Ok(Self { unused_finder })
    }

    #[napi]
    pub fn refresh_file_list(&mut self) {
        self.unused_finder.mark_all_dirty();
    }

    #[napi]
    pub fn find_unused_items(
        &mut self,
        files_to_check: Vec<String>,
    ) -> napi::Result<UnusedFinderReport> {
        self.unused_finder.mark_dirty(files_to_check);
        self.get_report()
    }

    #[napi]
    pub fn find_all_unused_items(&mut self) -> napi::Result<UnusedFinderReport> {
        self.unused_finder.mark_all_dirty();
        self.get_report()
    }

    fn get_report(&mut self) -> napi::Result<UnusedFinderReport> {
        let logger = StdioLogger::new();
        let result = self.unused_finder.find_unused(&logger)?;
        Ok(result.get_report())
    }
}
