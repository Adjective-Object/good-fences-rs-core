use oxc_ast::ast::{Declaration, Statement, VariableDeclaration, VariableDeclarationKind};
use oxc_span::{GetSpan, Span};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;

/// Ordered original-source pieces; at most four spans, stored inline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiSpan(SmallVec<[Span; 4]>);

impl MultiSpan {
    pub fn new(spans: impl IntoIterator<Item = Span>) -> eyre::Result<Self> {
        let mut result = Self::default();
        for span in spans {
            eyre::ensure!(result.0.len() < 4, "MultiSpan exceeds four source pieces");
            result.0.push(span);
        }
        eyre::ensure!(!result.0.is_empty(), "empty MultiSpan");
        Ok(result)
    }

    pub fn as_slice(&self) -> &[Span] {
        &self.0
    }

    pub fn contains(&self, span: Span) -> bool {
        self.0
            .iter()
            .any(|s| s.start <= span.start && span.end <= s.end)
    }
}

impl From<Span> for MultiSpan {
    fn from(span: Span) -> Self {
        Self(smallvec::smallvec![span])
    }
}

impl Serialize for MultiSpan {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        crate::owned::span_serde::list::serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for MultiSpan {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pieces;
        impl<'de> serde::de::Visitor<'de> for Pieces {
            type Value = MultiSpan;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("one to four source spans")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut spans = SmallVec::new();
                while let Some((start, end)) = seq.next_element::<(u32, u32)>()? {
                    if spans.len() == 4 {
                        return Err(serde::de::Error::custom(
                            "MultiSpan exceeds four source pieces",
                        ));
                    }
                    spans.push(Span::new(start, end));
                }
                if spans.is_empty() {
                    return Err(serde::de::Error::custom("empty MultiSpan"));
                }
                Ok(MultiSpan(spans))
            }
        }
        deserializer.deserialize_seq(Pieces)
    }
}

pub(crate) fn compound_declaration<'s, 'a>(
    statement: &'s Statement<'a>,
) -> Option<&'s VariableDeclaration<'a>> {
    let declaration = match statement {
        Statement::VariableDeclaration(declaration) => declaration.as_ref(),
        Statement::ExportDeclaration(export) => match &export.declaration {
            Declaration::VariableDeclaration(declaration) => declaration.as_ref(),
            _ => return None,
        },
        _ => return None,
    };
    (declaration.declarations.len() > 1
        && matches!(
            declaration.kind,
            VariableDeclarationKind::Const
                | VariableDeclarationKind::Let
                | VariableDeclarationKind::Var
        ))
    .then_some(declaration)
}

/// Ownership ranges are disjoint; assembly pieces may share a declaration prefix.
pub(crate) fn statement_sources(statement: &Statement<'_>) -> Vec<(Span, MultiSpan)> {
    let Some(declaration) = compound_declaration(statement) else {
        let span = statement.span();
        return vec![(span, span.into())];
    };
    let prefix = Span::new(
        statement.span().start,
        declaration.declarations[0].span.start,
    );
    declaration
        .declarations
        .iter()
        .map(|declarator| {
            (
                declarator.span,
                MultiSpan(smallvec::smallvec![prefix, declarator.span]),
            )
        })
        .collect()
}
