use napi_derive::napi;

#[napi(object)]
pub struct GoodFencesOptions {
    pub paths: Vec<String>,
    pub project: String,
    pub base_url: Option<String>,
    pub err_output_path: Option<String>,
    pub ignore_external_fences: Option<ExternalFences>,
    pub ignored_dirs: Option<Vec<String>>,
}

impl Into<good_fences::GoodFencesOptions> for GoodFencesOptions {
    fn into(self) -> good_fences::GoodFencesOptions {
        good_fences::GoodFencesOptions {
            paths: self.paths,
            project: self.project,
            base_url: self.base_url,
            err_output_path: self.err_output_path,
            ignore_external_fences: self.ignore_external_fences.map(Into::into),
            ignored_dirs: self.ignored_dirs,
        }
    }
}

#[derive(Eq, Debug, PartialEq)]
#[napi]
pub enum ExternalFences {
    Include = 0,
    Ignore = 1,
}

impl Into<good_fences::ExternalFences> for ExternalFences {
    fn into(self) -> good_fences::ExternalFences {
        match self {
            ExternalFences::Include => good_fences::ExternalFences::Include,
            ExternalFences::Ignore => good_fences::ExternalFences::Ignore,
        }
    }
}

#[napi(object)]
pub struct GoodFencesResult {
    pub result_type: GoodFencesResultType,
    pub message: String,
    pub source_file: Option<String>,
    pub raw_import: Option<String>,
    pub fence_path: Option<String>,
    pub detailed_message: String,
}

impl Into<GoodFencesResult> for good_fences::GoodFencesResult {
    fn into(self) -> GoodFencesResult {
        GoodFencesResult {
            result_type: self.result_type.into(),
            message: self.message,
            source_file: self.source_file,
            raw_import: self.raw_import,
            fence_path: self.fence_path,
            detailed_message: self.detailed_message,
        }
    }
}

#[derive(Eq, Debug, PartialEq)]
#[napi]
pub enum GoodFencesResultType {
    FileNotResolved = 0,
    Violation = 1,
}

impl Into<GoodFencesResultType> for good_fences::GoodFencesResultType {
    fn into(self) -> GoodFencesResultType {
        match self {
            good_fences::GoodFencesResultType::FileNotResolved => {
                GoodFencesResultType::FileNotResolved
            }
            good_fences::GoodFencesResultType::Violation => GoodFencesResultType::Violation,
        }
    }
}

#[napi]
pub fn good_fences(opts: GoodFencesOptions) -> Vec<GoodFencesResult> {
    let opts_native = opts.into();
    let eval_results = good_fences::good_fences(opts_native);
    eval_results.into_iter().map(Into::into).collect()
}
