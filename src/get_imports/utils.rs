use swc_common::{source_map::Pos, SourceFile};

pub fn get_specifier_name(fm: &SourceFile, spec: &swc_ecma_ast::ImportSpecifier) -> Option<String> {
    if let Some(default) = spec.as_default() {
        return Some(get_string_of_span(
            &fm.src.as_bytes().to_vec(),
            &default.span,
        ));
    }
    if let Some(named) = spec.as_named() {
        return Some(get_string_of_span(&fm.src.as_bytes().to_vec(), &named.span));
    }
    None
}

pub fn get_string_of_span<'a>(file_text: &'a Vec<u8>, span: &'a swc_common::Span) -> String {
    String::from_utf8_lossy(&file_text[span.lo().to_usize() - 1..span.hi().to_usize() - 1])
        .to_string()
}
