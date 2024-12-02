/**

       // declaration statements

       // class declaration
       class ClassDecl {}
*/

#[cfg(test)]
mod test {

    #[test]
    fn basic_statements() {
        let src: &str = r#"
        // Non-declaration statement (pure side-effect)
        nonDeclarationStatement();
        "#;
        let (_sourcemap, parsed_module) = swc_utils_parse::parse_ecma_src("test.ts", src);
    }

    /*
    // const declarations
    const constDecl = "asdf";
    export const constDecl = "asdf";

    // let declaration
    let letDecl = "asdf";
    export let letDecl = "asdf";

    // function declaration
    function functionDecl() {}
    export function functionDecl() {}

    // var declaration (should warn because non-lexical scoping of variables makes them unsafe)
    var varDecl = "asdf";
    export var varDecl = "asdf";
    */
}
