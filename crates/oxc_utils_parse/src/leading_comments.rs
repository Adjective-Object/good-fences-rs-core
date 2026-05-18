use oxc_ast::ast::{Comment, CommentPosition};

/// Return the contiguous slice of comments in `comments` whose span falls
/// immediately before `statement_start` with no intervening non-whitespace.
///
/// # Leading rule
///
/// A comment is considered to "lead" a statement when all of the following hold:
///
/// 1. OXC has classified it as `CommentPosition::Leading` (i.e. it is not a
///    same-line trailing comment of the preceding token).
/// 2. The comment ends at or before `statement_start` (i.e. `span.end <=
///    statement_start`).
/// 3. The gap between the comment's `span.end` and the next element's start
///    (the following comment's `span.start`, or `statement_start` itself) is
///    **whitespace-only and contains fewer than two `\n` characters** — i.e.
///    there is no blank line separating them.
///
/// Implication for interior comments: given
///
/// ```text
/// const a = 1;
///
/// // interior
///
/// const b = 2;
/// ```
///
/// the blank lines on either side of `// interior` mean it does not lead
/// `const b`. Callers querying `leading_comments_at(comments, source,
/// b_start)` receive an empty slice.
///
/// If instead the source has no blank lines around the interior comment, it
/// *does* lead the immediately following statement.
///
/// # Assumptions
///
/// * `comments` is sorted by `span.start` (OXC invariant on `Program.comments`).
///   Since OXC comments never overlap, this is equivalent to sorting by
///   `span.end`, so a `partition_point` on `span.end` is a valid binary search.
/// * Byte offsets in `source` correspond exactly to `Span` positions.
pub fn leading_comments_at<'a>(
    comments: &'a [Comment],
    source: &str,
    statement_start: u32,
) -> &'a [Comment] {
    // Index of the first comment whose span.end > statement_start.
    // Everything before end_idx has span.end <= statement_start.
    let end_idx = comments.partition_point(|c| c.span.end <= statement_start);

    if end_idx == 0 {
        return &[];
    }

    // Trim trailing non-leading (i.e. trailing) comments.  OXC marks same-line
    // comments as `CommentPosition::Trailing`; exclude them so that a trailing
    // comment on the previous statement is never counted as a leading comment
    // for the next one.
    let end_idx = {
        let mut i = end_idx;
        while i > 0 && comments[i - 1].position == CommentPosition::Trailing {
            i -= 1;
        }
        i
    };
    if end_idx == 0 {
        return &[];
    }

    // Check the gap between the last candidate comment and the statement.
    let last = &comments[end_idx - 1];
    if !is_attached_gap(source, last.span.end, statement_start) {
        return &[];
    }

    // Walk backwards to collect the full contiguous block.
    let mut start_idx = end_idx - 1;
    while start_idx > 0 {
        let prev = &comments[start_idx - 1];
        let curr = &comments[start_idx];
        if !is_attached_gap(source, prev.span.end, curr.span.start) {
            break;
        }
        start_idx -= 1;
    }

    &comments[start_idx..end_idx]
}

/// Returns `true` when the source slice `[from, to)` is whitespace-only and
/// does not contain a blank line (two or more `\n` characters).
fn is_attached_gap(source: &str, from: u32, to: u32) -> bool {
    let gap = &source[from as usize..to as usize];
    if !gap.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
        return false;
    }
    gap.bytes().filter(|&b| b == b'\n').count() < 2
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_span::GetSpan;

    use super::*;
    use crate::parse_ts;

    /// Helper: text of a comment's content (without delimiters).
    fn comment_text<'s>(comment: &Comment, source: &'s str) -> &'s str {
        let cs = comment.content_span();
        &source[cs.start as usize..cs.end as usize]
    }

    #[test]
    fn leading_comment_before_export() {
        let source = "// @ALLOW-UNUSED-EXPORT\nexport const x = 1;";
        let alloc = Allocator::default();
        let ret = parse_ts(&alloc, source);
        assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

        let program = &ret.program;
        // Find the export statement's start position.
        let export_stmt = program.body.first().expect("expected a statement");
        let stmt_start = export_stmt.span().start;

        let leading = leading_comments_at(&program.comments, source, stmt_start);
        assert_eq!(leading.len(), 1);
        assert!(
            comment_text(&leading[0], source).contains("@ALLOW-UNUSED-EXPORT"),
            "comment text was {:?}",
            comment_text(&leading[0], source)
        );
    }

    #[test]
    fn interior_comment_blank_lines_leads_neither() {
        // Blank lines on both sides of the interior comment: it leads neither
        // neighbour.
        let source = "const a = 1;\n\n// interior\n\nconst b = 2;";
        let alloc = Allocator::default();
        let ret = parse_ts(&alloc, source);
        assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

        let program = &ret.program;
        let b_stmt = program.body.get(1).expect("expected second statement");
        let b_start = b_stmt.span().start;

        let leading = leading_comments_at(&program.comments, source, b_start);
        assert!(
            leading.is_empty(),
            "expected no leading comments for `const b`, got {:?}",
            leading.iter().map(|c| comment_text(c, source)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn interior_comment_no_blank_line_leads_second() {
        // No blank lines: the comment leads `const b`.
        let source = "const a = 1;\n// interior\nconst b = 2;";
        let alloc = Allocator::default();
        let ret = parse_ts(&alloc, source);
        assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);

        let program = &ret.program;
        let b_stmt = program.body.get(1).expect("expected second statement");
        let b_start = b_stmt.span().start;

        let leading = leading_comments_at(&program.comments, source, b_start);
        assert_eq!(leading.len(), 1);
        assert!(comment_text(&leading[0], source).contains("interior"));
    }
}
