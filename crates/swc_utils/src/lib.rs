extern crate swc_core;
extern crate swc_ecma_parser;
use swc_core::common::comments::Comments;
use swc_core::common::SourceFile;
use swc_ecma_parser::TsConfig;
use swc_ecma_parser::{lexer::Lexer, StringInput, Syntax};

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
