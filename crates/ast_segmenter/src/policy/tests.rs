use super::*;
use std::path::Path;

#[test]
fn structured_policy_names_keep_mutation_and_indirect_uses_distinct() {
    let declarations = crate::PreparedDeclarations::new(ecco::LocalFacts::default()).unwrap();
    let source = "eval('call()'); require = loader; const alias = require; module.require('x');";
    let error = crate::parse_module_with_prepared_declarations(
        &logger::StdioLogger::new(),
        source,
        Path::new("source.cjs"),
        crate::ModuleFormat::CommonJs,
        &declarations,
    )
    .unwrap_err();
    let violations = error.downcast_ref::<PolicyErrors>().unwrap().violations();
    assert_eq!(
        violations.iter().map(|v| v.policy()).collect::<Vec<_>>(),
        [
            PolicyName::UsesEval,
            PolicyName::RequireReassignment,
            PolicyName::RequireIndirectUse,
            PolicyName::ModuleAccess,
        ]
    );
    let parsed = crate::parse_module_reporting_policies(
        &logger::StdioLogger::new(),
        source,
        Path::new("source.cjs"),
        crate::ModuleFormat::CommonJs,
        &declarations,
        |_, found| assert_eq!(found, violations),
    )
    .unwrap();
    assert_eq!(parsed.facts().policy_violations(), violations.to_vec());
}

#[test]
fn policy_excerpts_are_bounded_and_tolerate_invalid_or_absent_spans() {
    let path = Path::new("player.js");
    let source = "\nvar expression_function = eval('[function(){' + val + '}]')[0];";
    let start = source.find("eval").unwrap() as u32;
    let diagnostic = PolicyViolation::new(PolicyName::UsesEval, Span::new(start, start + 4))
        .diagnostic(path, Some(source));
    insta::assert_snapshot!(diagnostic, @r"
    player.js: policy uses-eval at bytes 27..31: unsupported unshadowed eval operation (line 2, column 27)
      var expression_function = eval('[function(){' + val + '}]')[0];
    ");
    for (source, range) in [
        ("éval()", Span::new(1, 4)),
        ("eval()", Span::new(99, 100)),
        ("eval()", Span::new(4, 1)),
        ("", Span::new(0, u32::MAX)),
    ] {
        let diagnostic =
            PolicyViolation::new(PolicyName::UsesEval, range).diagnostic(path, Some(source));
        assert!(diagnostic.contains("bytes "));
        assert!(!diagnostic.contains("(line "));
    }
    let missing =
        PolicyViolation::new(PolicyName::UsesEval, Span::new(1, 4)).diagnostic(path, None);
    assert!(missing.contains("player.js: policy uses-eval at bytes 1..4"));
    let long = format!("{}eval('{}')", "é".repeat(1000), "é".repeat(1000));
    let diagnostic = PolicyViolation::new(PolicyName::UsesEval, Span::new(2000, 2004))
        .diagnostic(path, Some(&long));
    assert!(diagnostic.len() < 600);
    assert!(diagnostic.contains("eval("));
    let multiline = PolicyViolation::new(PolicyName::UsesEval, Span::new(0, 4))
        .diagnostic(path, Some("eval\n('triggering call')"));
    assert!(multiline.contains("('triggering call')"), "{}", multiline);
}
