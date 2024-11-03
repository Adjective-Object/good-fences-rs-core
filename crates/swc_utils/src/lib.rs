use swc_common::comments::{Comments, SingleThreadedComments};
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceFile, SourceMap};
use swc_compiler_base::PrintArgs;
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

pub fn ast_to_str(cm: &Lrc<SourceMap>, module: &Module, print_args: PrintArgs<'_>) -> String {
    let out_str = swc_compiler_base::print(cm.clone(), module, print_args).unwrap();
    out_str.code
}

pub fn normalise_src(src: &str, print_args: PrintArgs) -> String {
    let mut pargs = print_args;

    // Backup value for comments in case it is not provided.
    //
    // Declare this at the function level so that it will be dropped at
    // the end of the function, even though it is only ever used / initialized
    // in the if branch below.
    let own_comments: Option<SingleThreadedComments>;
    if pargs.comments.is_none() {
        let comments = SingleThreadedComments::default();
        own_comments = Some(comments);
        pargs.comments = own_comments.as_ref().map(|c| c as &dyn Comments);
    }

    let (cm, parsed) = parse_ecma_src_comments("test.ts", src, pargs.comments);
    ast_to_str(&cm, &parsed, pargs)
}

#[cfg(test)]
mod test {
    use crate::normalise_src;

    #[test]
    fn test_normalise_src() {
        assert_eq!(
            normalise_src(
                r#"
                const used_1 = 1;
                const used_2 = 1;
                const unused = 2;
                // comments should be retained!
                export { used_1, used_2 }
                "#,
                Default::default()
            ),
            r#"const used_1 = 1;
const used_2 = 1;
const unused = 2;
// comments should be retained!
export { used_1, used_2 };
"#
        );
    }
}
