extern crate swc_core;
extern crate swc_ecma_parser;
use swc_core::common::comments::Comments;
use swc_core::common::errors::Handler;
use swc_core::common::{Globals, Mark, SourceFile, SourceMap, GLOBALS};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::visit::{fold_module, visit_module};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
use swc_ecma_parser::{Capturing, TsConfig};

pub fn create_lexer<'a>(fm: &'a SourceFile, comments: Option<&'a dyn Comments>) -> Lexer<'a> {
    let filename = fm.name.to_string();
    let lexer = Lexer::new(
        Syntax::Typescript(TsConfig {
            tsx: filename.ends_with(".tsx") || filename.ends_with(".jsx"),
            decorators: true,
            ..Default::default()
        }),
        Default::default(),
        StringInput::from(fm),
        comments,
    );
    return lexer;
}