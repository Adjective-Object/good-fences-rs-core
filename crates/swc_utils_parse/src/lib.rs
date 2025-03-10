use swc_common::comments::{Comments, SingleThreadedComments};
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceFile, SourceMap};
use swc_ecma_ast::Module;
use swc_ecma_parser::{lexer::Lexer, StringInput, Syntax};
use swc_ecma_parser::{Capturing, Parser, TsSyntax};
use swc_ecma_visit::{Visit, VisitWith};

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

pub fn create_parser<'a>(
    fm: &'a Lrc<SourceFile>,
    comments: Option<&'a dyn Comments>,
) -> Parser<Capturing<Lexer<'a>>> {
    let lexer = create_lexer(fm, comments);
    let capturing = Capturing::new(lexer);

    Parser::new_from(capturing)
}

pub fn parse_and_visit(
    src: &str,
    visitor: &mut impl Visit,
) -> Result<(), swc_ecma_parser::error::Error> {
    let cm = Lrc::<SourceMap>::default();
    let comments = SingleThreadedComments::default();
    let fm = cm.new_source_file(
        Lrc::new(FileName::Custom("test.ts".into())),
        src.to_string(),
    );

    let mut parser = create_parser(&fm, Some(&comments));
    let module = parser.parse_typescript_module()?;

    module.visit_with(visitor);
    Ok(())
}
