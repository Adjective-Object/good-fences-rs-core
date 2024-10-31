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

impl From<GoodFencesOptions> for good_fences::GoodFencesOptions {
    fn from(val: GoodFencesOptions) -> Self {
        good_fences::GoodFencesOptions {
            paths: val.paths,
            project: val.project,
            base_url: val.base_url,
            err_output_path: val.err_output_path,
            ignore_external_fences: val.ignore_external_fences.map(Into::into),
            ignored_dirs: val.ignored_dirs,
        }
    }
}

#[derive(Eq, Debug, PartialEq)]
#[napi]
pub enum ExternalFences {
    Include = 0,
    Ignore = 1,
}

impl From<ExternalFences> for good_fences::ExternalFences {
    fn from(val: ExternalFences) -> Self {
        match val {
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

impl From<good_fences::GoodFencesResult> for GoodFencesResult {
    fn from(val: good_fences::GoodFencesResult) -> Self {
        GoodFencesResult {
            result_type: val.result_type.into(),
            message: val.message,
            source_file: val.source_file,
            raw_import: val.raw_import,
            fence_path: val.fence_path,
            detailed_message: val.detailed_message,
        }
    }
}

#[derive(Eq, Debug, PartialEq)]
#[napi]
pub enum GoodFencesResultType {
    FileNotResolved = 0,
    Violation = 1,
}

impl From<good_fences::GoodFencesResultType> for GoodFencesResultType {
    fn from(val: good_fences::GoodFencesResultType) -> Self {
        match val {
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
