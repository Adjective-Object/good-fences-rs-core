use std::collections::{HashMap, HashSet};
use swc_core::common::comments::Comments;
use swc_core::common::SourceFile;
use swc_ecma_parser::{lexer::Lexer, StringInput, Syntax};
use swc_ecma_parser::TsConfig;

#[derive(Debug, Default)]
#[napi(js_name = "UnusedFinder")]
pub struct UnusedFinderWrapper {
    unused_finder: UnusedFinder,
}

#[napi]
impl UnusedFinderWrapper {
    #[napi(constructor)]
    pub fn new(config: FindUnusedItemsConfig) -> napi::Result<Self> {
        let finder = UnusedFinder::new(config);
        match finder {
            Ok(finder) => {
                return Ok(Self {
                    unused_finder: finder,
                })
            }
            Err(e) => return Err(e.into()),
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
        self.unused_finder.find_unused_items(files_to_check)
    }

    #[napi]
    pub fn find_all_unused_items(&mut self) -> napi::Result<UnusedFinderReport> {
        self.unused_finder.find_all_unused_items()
    }
}
