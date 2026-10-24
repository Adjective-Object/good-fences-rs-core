use oxc_ast::Comment;

/// Return the contiguous slice of comments in `comments` whose spans fall
/// immediately before `statement_start` with no intervening non-whitespace.
///
/// # Rule
///
/// A comment is considered "leading" for a statement if and only if the text
/// between the comment's `span.end` and `statement_start` contains only
/// whitespace. Comments are collected greedily backwards: starting from the
/// last comment before `statement_start`, we walk backwards and include each
/// comment as long as the gap between it and the next item (either the
/// following comment or the statement) contains only whitespace.
///
/// This means an interior comment between two statements is considered leading
/// for the *second* statement (the one it immediately precedes), unless there
/// is non-whitespace between them.
///
/// # Assumptions
///
/// `comments` is sorted by `span.end` (oxc invariant on `Program.comments`).
pub fn leading_comments_at<'a>(
    comments: &'a [Comment],
    source: &str,
    statement_start: u32,
) -> &'a [Comment] {
    if comments.is_empty() {
        return &[];
    }

    // Find the first comment whose span.end > statement_start.
    // All comments before this index have span.end <= statement_start.
    let partition = comments.partition_point(|c| c.span.end <= statement_start);

    if partition == 0 {
        return &[];
    }

    // Walk backwards from partition-1, collecting contiguous comments.
    // "Contiguous" means the gap between the comment's span.end and the next
    // item's position (either the following comment's span.start or statement_start)
    // contains only whitespace.
    let mut start_idx = partition;
    let mut next_boundary = statement_start as usize;

    for i in (0..partition).rev() {
        let comment = &comments[i];
        let gap = &source[comment.span.end as usize..next_boundary];
        if gap.chars().all(|c| c.is_whitespace()) {
            start_idx = i;
            next_boundary = comment.span.start as usize;
        } else {
            break;
        }
    }

    &comments[start_idx..partition]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_ts;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::Statement;
    use oxc_span::GetSpan;

    #[test]
    fn leading_comment_allow_unused_export() {
        let source = "// @ALLOW-UNUSED-EXPORT\nexport const x = 1;";
        let allocator = Allocator::default();
        let result = parse_ts(&allocator, source);
        assert!(
            result.diagnostics.is_empty(),
            "parse errors: {:?}",
            result.diagnostics
        );

        let program = &result.program;
        // Find the exported declaration.
        let export_stmt = program
            .body
            .iter()
            .find(|s| matches!(s, Statement::ExportDeclaration(_)));
        assert!(export_stmt.is_some(), "expected ExportDeclaration");
        let span_start = export_stmt.unwrap().span().start;

        let leading = leading_comments_at(&program.comments, source, span_start);
        assert_eq!(leading.len(), 1);

        let comment_text = &source[leading[0].span.start as usize..leading[0].span.end as usize];
        assert!(
            comment_text.contains("@ALLOW-UNUSED-EXPORT"),
            "expected comment to contain @ALLOW-UNUSED-EXPORT, got: {comment_text}"
        );
    }

    #[test]
    fn interior_comment_leads_second_statement() {
        // An interior comment between two statements should be leading for
        // the second statement (the one it immediately precedes), because the
        // gap between the comment and the second statement is only whitespace.
        let source = "const a = 1;\n// interior\nconst b = 2;";
        let allocator = Allocator::default();
        let result = parse_ts(&allocator, source);
        assert!(
            result.diagnostics.is_empty(),
            "parse errors: {:?}",
            result.diagnostics
        );

        let program = &result.program;
        assert!(program.body.len() >= 2, "expected at least 2 statements");

        let first_start = program.body[0].span().start;
        let second_start = program.body[1].span().start;

        // The interior comment should NOT be leading for the first statement
        let leading_first = leading_comments_at(&program.comments, source, first_start);
        assert!(
            leading_first.is_empty(),
            "expected no leading comments for first statement, got {}",
            leading_first.len()
        );

        // The interior comment SHOULD be leading for the second statement
        let leading_second = leading_comments_at(&program.comments, source, second_start);
        assert_eq!(
            leading_second.len(),
            1,
            "expected 1 leading comment for second statement"
        );

        let comment_text =
            &source[leading_second[0].span.start as usize..leading_second[0].span.end as usize];
        assert!(
            comment_text.contains("interior"),
            "expected comment to contain 'interior', got: {comment_text}"
        );
    }
}
