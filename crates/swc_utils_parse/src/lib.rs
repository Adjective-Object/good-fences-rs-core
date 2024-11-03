use swc_common::comments::Comments;
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceFile, SourceMap};
use swc_ecma_ast::Module;
use swc_ecma_parser::{lexer::Lexer, StringInput, Syntax};
use swc_ecma_parser::{Capturing, Parser, TsSyntax};

pub fn create_lexer<'a>(fm: &'a SourceFile, comments: Option<&'a dyn Comments>) -> Lexer<'a> {
    let filename = fm.name.to_string();
    let lexer = Lexer::new(
        Syntax::Typescript(TsSyntax {
            tsx: filename.ends_with(".tsx") || filename.ends_with(".jsx"),
            decorators: true,
            ..Default::default()
        }),
        Default::default(),
        StringInput::from(fm),
        comments,
    );
    lexer
}

pub fn parse_ecma_src<TName, TBody>(name_str: TName, body: TBody) -> (Lrc<SourceMap>, Module)
where
    TName: Into<String>,
    TBody: ToString,
{
    parse_ecma_src_comments(name_str, body, None)
}

pub fn parse_ecma_src_comments<TName, TBody>(
    name_str: TName,
    body: TBody,
    comments: Option<&dyn Comments>,
) -> (Lrc<SourceMap>, Module)
where
    TName: Into<String>,
    TBody: ToString,
{
    let cm = Lrc::<SourceMap>::default();
    let fname: Lrc<FileName> = Lrc::new(FileName::Custom(name_str.into()));
    let fm = cm.new_source_file(fname, body.to_string());

    let lexer: Lexer<'_> = create_lexer(&fm, comments);
    let capturing = Capturing::new(lexer);
    let mut parser: Parser<Capturing<Lexer<'_>>> = Parser::new_from(capturing);
    let module = parser.parse_typescript_module().unwrap();

    (cm, module)
}
