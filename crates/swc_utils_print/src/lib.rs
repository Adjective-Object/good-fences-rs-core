use swc_common::comments::{Comments, SingleThreadedComments};
use swc_common::sync::Lrc;
use swc_common::SourceMap;
use swc_compiler_base::PrintArgs;
use swc_ecma_ast::Module;

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

    let (cm, parsed) = swc_utils_parse::parse_ecma_src_comments("test.ts", src, pargs.comments);
    ast_to_str(&cm, &parsed, pargs)
}

#[cfg(test)]
mod test {
    use crate::normalise_src;
    use pretty_assertions::assert_eq;

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
