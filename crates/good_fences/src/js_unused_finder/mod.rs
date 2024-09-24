use napi_derive::napi;
use unused_finder::{FindUnusedItemsConfig, UnusedFinder, UnusedFinderReport};

/**
 * Provides a napi wrapper for the UnusedFinder, which plumbs
 * the UnusedFinder's functionality to JavaScript.
 */
#[derive(Debug)]
#[napi(js_name = "UnusedFinder")]
pub struct JsUnusedFinder {
    unused_finder: UnusedFinder,
}

#[napi]
impl JsUnusedFinder {
    #[napi(constructor)]
    pub fn new(config: FindUnusedItemsConfig) -> napi::Result<Self> {
        let finder = UnusedFinder::new(config);
        match finder {
            Ok(finder) => Ok(Self {
                unused_finder: finder,
            }),
            Err(e) => Err(e.into()),
        }
    }

    #[napi]
    pub fn refresh_file_list(&mut self) {
        self.unused_finder.refresh_file_list();
    }

    #[napi]
    pub fn find_unused_items(
        &mut self,
        files_to_check: Vec<String>,
    ) -> napi::Result<UnusedFinderReport> {
        self.unused_finder
            .find_unused_items(files_to_check)
            .map_err(|e| e.into())
    }

    #[napi]
    pub fn find_all_unused_items(&mut self) -> napi::Result<UnusedFinderReport> {
        self.unused_finder
            .find_all_unused_items()
            .map_err(|e| e.into())
    }
}
