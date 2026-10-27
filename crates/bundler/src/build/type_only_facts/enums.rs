use super::*;
use cache_flatbuffers::{TYPE_ONLY_STRING_TOTAL_BYTES, TYPE_ONLY_STRING_VALUE_BYTES};
use oxc_ecmascript::{ToInt32, ToUint32};
use oxc_syntax::{
    constant_value::ConstantValue,
    operator::{BinaryOperator, UnaryOperator},
    scope::ScopeId,
};

const RETRIES: usize = 64;
const EXPRESSION_BUDGET: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Seed {
    Semantic,
    Bounded,
}

#[derive(Clone, Copy)]
pub(super) struct StringLimits {
    pub(super) value: usize,
    pub(super) total: usize,
}

impl Default for StringLimits {
    fn default() -> Self {
        Self {
            value: TYPE_ONLY_STRING_VALUE_BYTES,
            total: TYPE_ONLY_STRING_TOTAL_BYTES,
        }
    }
}

struct Strings {
    limits: StringLimits,
    expanded: usize,
}

impl Strings {
    fn allocate(&mut self, length: usize) -> Result<String, &'static str> {
        if length > self.limits.value {
            return Err("enum string exceeds per-value expanded-byte budget");
        }
        let total = self
            .expanded
            .checked_add(length)
            .ok_or("enum string expanded-byte count overflow")?;
        if total > self.limits.total {
            return Err("enum strings exceed aggregate expanded-byte budget");
        }
        self.expanded = total;
        let mut output = String::new();
        output
            .try_reserve_exact(length)
            .map_err(|_| "enum string expanded-byte allocation failed")?;
        Ok(output)
    }

    fn copy(&mut self, text: &str) -> Result<String, &'static str> {
        let mut output = self.allocate(text.len())?;
        output.push_str(text);
        Ok(output)
    }

    fn concatenate(&mut self, left: &Value, right: &Value) -> Result<Value, &'static str> {
        let mut left_buffer = dragonbox_ecma::Buffer::new();
        let mut right_buffer = dragonbox_ecma::Buffer::new();
        let left = left.text(&mut left_buffer);
        let right = right.text(&mut right_buffer);
        let length = left
            .len()
            .checked_add(right.len())
            .ok_or("enum string expanded-byte count overflow")?;
        let mut output = self.allocate(length)?;
        output.push_str(left);
        output.push_str(right);
        Ok(Value::String(Arc::new(output)))
    }

    fn join(&mut self, parts: &[Value]) -> Result<Value, &'static str> {
        let mut length = 0usize;
        let mut buffer = dragonbox_ecma::Buffer::new();
        for part in parts {
            length = length
                .checked_add(part.text(&mut buffer).len())
                .ok_or("enum string expanded-byte count overflow")?;
        }
        let mut output = self.allocate(length)?;
        for part in parts {
            output.push_str(part.text(&mut buffer));
        }
        Ok(Value::String(Arc::new(output)))
    }
}

// References share expanded storage; cloning a value never copies string bytes.
// Only Strings may create/copy expanded strings, including semantic seeds and DTOs.
#[derive(Clone)]
enum Value {
    Number(f64),
    String(Arc<String>),
}

impl Value {
    fn text<'v>(&'v self, buffer: &'v mut dragonbox_ecma::Buffer) -> &'v str {
        match self {
            Self::Number(n) => buffer.format(*n),
            Self::String(s) => s.as_str(),
        }
    }
}

type EnumScopes = HashMap<SymbolId, Vec<ScopeId>>;

struct Member<'a> {
    declaration: &'a TSEnumDeclaration<'a>,
    member: &'a TSEnumMember<'a>,
    symbol: Option<SymbolId>,
    previous: Option<usize>,
}

/// Original-AST callers can reuse pinned semantic values. Standalone metadata
/// disables upstream evaluation and uses only this resource-bounded evaluator.
pub(super) fn extract<'a>(
    program: &'a Program<'a>,
    semantic: &Semantic<'a>,
    facts: &mut wire::FactsT,
    seed: Seed,
) -> eyre::Result<()> {
    extract_with_limits(program, semantic, facts, seed, StringLimits::default())
}

pub(super) fn extract_with_limits<'a>(
    program: &'a Program<'a>,
    semantic: &Semantic<'a>,
    facts: &mut wire::FactsT,
    seed: Seed,
    limits: StringLimits,
) -> eyre::Result<()> {
    let declarations: Vec<_> = program
        .body
        .iter()
        .filter_map(|s| match s {
            Statement::TSEnumDeclaration(d) => Some(d.as_ref()),
            Statement::ExportDeclaration(d) => match &d.declaration {
                Declaration::TSEnumDeclaration(d) => Some(d.as_ref()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let mut members = Vec::new();
    let mut ranges = Vec::new();
    let mut values = HashMap::new();
    let mut locations = HashMap::new();
    let mut unsupported = HashMap::new();
    let mut scopes = EnumScopes::new();
    let mut strings = Strings {
        limits,
        expanded: 0,
    };
    for declaration in &declarations {
        if seed == Seed::Semantic {
            ensure!(
                declaration
                    .id
                    .symbol_id
                    .get()
                    .and_then(|id| semantic.scoping().get_enum_body_scopes(id))
                    .is_some_and(|scopes| declaration
                        .body
                        .scope_id
                        .get()
                        .is_some_and(|scope| scopes.contains(&scope))),
                "TypeOnlyFacts requires SemanticBuilder::with_enum_eval(true) for enum at {}..{}",
                declaration.span.start,
                declaration.span.end
            );
        }
        let enum_symbol = declaration
            .id
            .symbol_id
            .get()
            .ok_or_else(|| eyre::eyre!("TypeOnlyFacts enum lacks semantic identity"))?;
        let scope = declaration
            .body
            .scope_id
            .get()
            .ok_or_else(|| eyre::eyre!("TypeOnlyFacts enum lacks semantic scope"))?;
        scopes.entry(enum_symbol).or_default().push(scope);
        let start = members.len();
        let mut previous = None;
        for member in &declaration.body.members {
            let symbol = declaration.body.scope_id.get().and_then(|scope| {
                member
                    .id
                    .static_name()
                    .as_str()
                    .and_then(|name| semantic.scoping().get_binding(scope, name.into()))
            });
            if let Some(symbol) = symbol {
                if seed == Seed::Semantic {
                    if let Some(value) = semantic.scoping().get_enum_member_value(symbol) {
                        let value = match value {
                            ConstantValue::Number(n) => Ok(Value::Number(*n)),
                            ConstantValue::String(s) => {
                                strings.copy(s.as_str()).map(|s| Value::String(Arc::new(s)))
                            }
                        };
                        match value {
                            Ok(value) if valid(&value) => {
                                values.insert(symbol, value);
                            }
                            Ok(_) => {
                                unsupported.insert(members.len(), "non-finite enum constant");
                            }
                            Err(reason) => {
                                unsupported.insert(members.len(), reason);
                            }
                        }
                    }
                }
                locations.insert(symbol, members.len());
            }
            members.push(Member {
                declaration,
                member,
                symbol,
                previous,
            });
            previous = Some(members.len() - 1);
        }
        ranges.push(start..members.len());
    }
    for _ in 0..members.len().min(RETRIES) {
        let mut progress = false;
        for (index, pending) in members.iter().enumerate() {
            let Some(symbol) = pending.symbol else {
                continue;
            };
            if values.contains_key(&symbol) || unsupported.contains_key(&index) {
                continue;
            }
            let value = if let Some(init) = &pending.member.initializer {
                let context = Context {
                    semantic,
                    scope: pending.declaration.body.scope_id.get(),
                    enumeration: pending.declaration.id.symbol_id.get(),
                    scopes: &scopes,
                    values: &values,
                };
                let mut budget = EXPRESSION_BUDGET;
                evaluate(init, &context, &mut budget, &mut strings)
            } else {
                Ok(match pending.previous {
                    None => Some(Value::Number(0.0)),
                    Some(previous) => members[previous]
                        .symbol
                        .and_then(|symbol| values.get(&symbol))
                        .and_then(|value| match value {
                            Value::Number(n) => Some(Value::Number(n + 1.0)),
                            Value::String(_) => None,
                        }),
                })
            };
            match value {
                Ok(Some(value)) if valid(&value) => {
                    values.insert(symbol, value);
                    progress = true;
                }
                Ok(Some(_)) => {
                    unsupported.insert(index, "non-finite enum constant");
                }
                Err(reason) => {
                    unsupported.insert(index, reason);
                }
                Ok(None) => {}
            }
        }
        if !progress {
            break;
        }
    }
    let cycles = cycle_dependents(&members, semantic, &values, &locations, &scopes);
    for (declaration, range) in declarations
        .into_iter()
        .zip(ranges)
        .filter(|(d, _)| d.r#const)
    {
        let mut enumeration = wire::ConstEnumT::default();
        enumeration.name = name(declaration.id.name.as_str(), declaration.id.span);
        enumeration.span = span(declaration.span);
        enumeration.declared =
            declaration.declare || program.source_type.is_typescript_definition();
        for pending in &members[range] {
            let member = pending.member;
            let mut item = wire::MemberT::default();
            let member_name = member.id.static_name();
            let member_name = member_name.as_str().ok_or_else(|| {
                eyre::eyre!(
                    "non-UTF-8 enum member at {}..{}",
                    member.span.start,
                    member.span.end
                )
            })?;
            item.name = name(member_name, member.id.span());
            item.span = span(member.span);
            item.initializer = member.initializer.as_ref().map(|e| span(e.span()));
            let value = pending.symbol.and_then(|symbol| values.get(&symbol));
            let mut storage_failure = None;
            match value {
                Some(Value::Number(n)) => {
                    item.kind = wire::ConstantKind::Number;
                    item.number = Some(*n);
                }
                Some(Value::String(s)) => match strings.copy(s.as_str()) {
                    Ok(s) => {
                        item.kind = wire::ConstantKind::String;
                        item.string = Some(s);
                    }
                    Err(reason) => storage_failure = Some(reason),
                },
                None => {}
            }
            if value.is_none() || storage_failure.is_some() {
                let mut diagnostic = wire::DiagnosticT::default();
                let index = pending.symbol.and_then(|symbol| locations.get(&symbol));
                let reason = storage_failure
                    .or_else(|| index.and_then(|index| unsupported.get(index)).copied());
                diagnostic.kind = if reason.is_some() {
                    wire::DiagnosticKind::UnsupportedConstant
                } else if index.is_some_and(|index| cycles[*index]) {
                    wire::DiagnosticKind::CyclicConstant
                } else {
                    wire::DiagnosticKind::UnresolvedConstant
                };
                diagnostic.span = item.span.clone();
                let fallback = if diagnostic.kind == wire::DiagnosticKind::CyclicConstant {
                    "cyclic same-file enum dependency"
                } else {
                    "unresolved enum dependency or invalid implicit increment; bounded same-file evaluation exhausted"
                };
                let reason = reason.unwrap_or(fallback);
                diagnostic.message = format!(
                    "{}.{member_name}: {} (original bytes {}..{})",
                    declaration.id.name, reason, member.span.start, member.span.end
                );
                facts.diagnostics.push(diagnostic);
            }
            enumeration.members.push(item);
        }
        facts.const_enums.push(enumeration);
    }
    Ok(())
}

fn valid(value: &Value) -> bool {
    match value {
        Value::Number(n) => n.is_finite(),
        Value::String(_) => true,
    }
}

struct Context<'s, 'a> {
    semantic: &'s Semantic<'a>,
    scope: Option<ScopeId>,
    enumeration: Option<SymbolId>,
    scopes: &'s EnumScopes,
    values: &'s HashMap<SymbolId, Value>,
}

impl Context<'_, '_> {
    fn lexical_symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        id.reference_id
            .get()
            .and_then(|id| self.semantic.scoping().get_reference(id).symbol_id())
            .or_else(|| {
                self.scope
                    .and_then(|scope| self.semantic.scoping().find_binding(scope, id.name))
            })
    }
    fn symbol(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        let lexical = self.lexical_symbol(id);
        let scoping = self.semantic.scoping();
        // A genuine current/nested lexical binding wins over a merged sibling.
        if lexical.is_some_and(|symbol| {
            self.scope.is_some_and(|scope| {
                scoping
                    .scope_ancestors(scoping.symbol_scope_id(symbol))
                    .any(|ancestor| ancestor == scope)
            })
        }) {
            return lexical;
        }
        self.enumeration
            .and_then(|id| self.scopes.get(&id))
            .and_then(|scopes| {
                scopes.iter().find_map(|scope| {
                    if Some(*scope) == self.scope {
                        None
                    } else {
                        scoping.get_binding(*scope, id.name)
                    }
                })
            })
            .or(lexical)
    }
    fn reference(&self, id: &IdentifierReference<'_>) -> Option<Value> {
        if let Some(symbol) = self.symbol(id) {
            return self.values.get(&symbol).cloned();
        }
        match id.name.as_str() {
            "Infinity" => Some(Value::Number(f64::INFINITY)),
            "NaN" => Some(Value::Number(f64::NAN)),
            _ => None,
        }
    }
    fn member(&self, object: &Expression<'_>, property: &str) -> Option<Value> {
        self.member_symbol(object, property)
            .and_then(|symbol| self.values.get(&symbol))
            .cloned()
    }
    fn member_symbol(&self, object: &Expression<'_>, property: &str) -> Option<SymbolId> {
        let Expression::Identifier(id) = object else {
            return None;
        };
        let symbol = self.lexical_symbol(id)?;
        let scopes = self.scopes.get(&symbol)?;
        scopes
            .iter()
            .find_map(|scope| self.semantic.scoping().get_binding(*scope, property.into()))
    }
}

// Eliminate acyclic unresolved dependencies iteratively. What remains either
// participates in, or depends on, a real same-file cycle, not an external miss.
fn cycle_dependents(
    members: &[Member<'_>],
    semantic: &Semantic<'_>,
    values: &HashMap<SymbolId, Value>,
    locations: &HashMap<SymbolId, usize>,
    scopes: &EnumScopes,
) -> Vec<bool> {
    let mut counts = vec![0; members.len()];
    let mut users = vec![Vec::new(); members.len()];
    for (index, member) in members.iter().enumerate() {
        if member.symbol.is_some_and(|id| values.contains_key(&id)) {
            continue;
        }
        let context = Context {
            semantic,
            scope: member.declaration.body.scope_id.get(),
            enumeration: member.declaration.id.symbol_id.get(),
            scopes,
            values,
        };
        let mut dependencies = Dependencies {
            context,
            symbols: Vec::new(),
        };
        if let Some(init) = &member.member.initializer {
            dependencies.visit_expression(init);
        } else if let Some(previous) = member.previous.and_then(|index| members[index].symbol) {
            dependencies.symbols.push(previous);
        }
        dependencies.symbols.sort_unstable();
        dependencies.symbols.dedup();
        for symbol in dependencies.symbols {
            if values.contains_key(&symbol) {
                continue;
            }
            if let Some(dependency) = locations.get(&symbol) {
                counts[index] += 1;
                users[*dependency].push(index);
            }
        }
    }
    let mut ready: std::collections::VecDeque<_> = counts
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect();
    while let Some(index) = ready.pop_front() {
        for user in &users[index] {
            counts[*user] -= 1;
            if counts[*user] == 0 {
                ready.push_back(*user);
            }
        }
    }
    counts.into_iter().map(|count| count > 0).collect()
}

struct Dependencies<'s, 'a> {
    context: Context<'s, 'a>,
    symbols: Vec<SymbolId>,
}

impl<'a> Visit<'a> for Dependencies<'_, 'a> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if let Some(symbol) = self.context.symbol(id) {
            self.symbols.push(symbol);
        }
    }
    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if matches!(member.object, Expression::Identifier(_)) {
            if let Some(symbol) = self
                .context
                .member_symbol(&member.object, member.property.name.as_str())
            {
                self.symbols.push(symbol);
            }
            // A receiver uses lexical lookup, not bare-member sibling lookup.
            return;
        }
        walk::walk_static_member_expression(self, member);
    }
    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        if matches!(member.object, Expression::Identifier(_)) {
            if let Expression::StringLiteral(property) = &member.expression {
                if let Some(symbol) = property
                    .value
                    .as_str()
                    .and_then(|property| self.context.member_symbol(&member.object, property))
                {
                    self.symbols.push(symbol);
                }
                return;
            }
        }
        walk::walk_computed_member_expression(self, member);
    }
}

fn evaluate(
    expr: &Expression<'_>,
    context: &Context<'_, '_>,
    budget: &mut usize,
    strings: &mut Strings,
) -> Result<Option<Value>, &'static str> {
    if *budget == 0 {
        return Err("enum constant expression exceeds bounded evaluation budget");
    }
    *budget -= 1;
    // Keep recursion bounded as well as total work; hostile metadata must not
    // turn the dependency retry budget into an unbounded recursive evaluator.
    evaluate_inner(expr, context, budget, strings, 0)
}

fn evaluate_inner(
    expr: &Expression<'_>,
    context: &Context<'_, '_>,
    budget: &mut usize,
    strings: &mut Strings,
    depth: usize,
) -> Result<Option<Value>, &'static str> {
    if depth >= 64 || *budget == 0 {
        return Err("enum constant expression exceeds bounded evaluation budget");
    }
    *budget -= 1;
    let mut eval =
        |expr, strings: &mut Strings| evaluate_inner(expr, context, budget, strings, depth + 1);
    let value = match expr {
        Expression::NumericLiteral(n) => Value::Number(n.value),
        Expression::StringLiteral(s) => {
            let Some(s) = s.value.as_str() else {
                return Err("non-UTF-8 string enum constant");
            };
            Value::String(Arc::new(strings.copy(s)?))
        }
        Expression::TemplateLiteral(template) => {
            if template.quasis.len() > *budget {
                return Err("enum constant expression exceeds bounded evaluation budget");
            }
            *budget -= template.quasis.len();
            let mut parts = Vec::new();
            for (i, quasi) in template.quasis.iter().enumerate() {
                let text = match quasi.value.cooked {
                    Some(cooked) => cooked.as_str().ok_or("non-UTF-8 template enum constant")?,
                    None => quasi.value.raw.as_str(),
                };
                parts.push(Value::String(Arc::new(strings.copy(text)?)));
                if let Some(expression) = template.expressions.get(i) {
                    let Some(value) =
                        evaluate_inner(expression, context, budget, strings, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    parts.push(value);
                }
            }
            strings.join(&parts)?
        }
        Expression::Identifier(id) => return Ok(context.reference(id)),
        Expression::StaticMemberExpression(m) => {
            if !matches!(m.object, Expression::Identifier(_)) {
                return Err("unsupported enum member receiver");
            }
            return Ok(context.member(&m.object, m.property.name.as_str()));
        }
        Expression::ComputedMemberExpression(m) => {
            if !matches!(m.object, Expression::Identifier(_)) {
                return Err("unsupported enum member receiver");
            }
            let Expression::StringLiteral(property) = &m.expression else {
                return Err("nonliteral computed enum member");
            };
            return Ok(property
                .value
                .as_str()
                .and_then(|property| context.member(&m.object, property)));
        }
        Expression::ParenthesizedExpression(p) => return eval(&p.expression, strings),
        Expression::UnaryExpression(u) => {
            if !matches!(
                u.operator,
                UnaryOperator::UnaryPlus | UnaryOperator::UnaryNegation | UnaryOperator::BitwiseNot
            ) {
                return Err("unsupported unary enum expression");
            }
            let Some(value) = eval(&u.argument, strings)? else {
                return Ok(None);
            };
            let Value::Number(n) = value else {
                return Err("unary string enum constant");
            };
            Value::Number(match u.operator {
                UnaryOperator::UnaryPlus => n,
                UnaryOperator::UnaryNegation => -n,
                UnaryOperator::BitwiseNot => f64::from(!n.to_int_32()),
                _ => return Err("unsupported unary enum expression"),
            })
        }
        Expression::BinaryExpression(b) => {
            if !matches!(
                b.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
                    | BinaryOperator::Exponential
                    | BinaryOperator::BitwiseOR
                    | BinaryOperator::BitwiseAnd
                    | BinaryOperator::BitwiseXOR
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
                    | BinaryOperator::ShiftRightZeroFill
            ) {
                return Err("unsupported binary enum expression");
            }
            let left = eval(&b.left, strings)?;
            let right = eval(&b.right, strings)?;
            let (Some(left), Some(right)) = (left, right) else {
                return Ok(None);
            };
            if b.operator == BinaryOperator::Addition
                && (matches!(left, Value::String(_)) || matches!(right, Value::String(_)))
            {
                strings.concatenate(&left, &right)?
            } else {
                let (Value::Number(l), Value::Number(r)) = (left, right) else {
                    return Err("nonnumeric enum arithmetic");
                };
                Value::Number(match b.operator {
                    BinaryOperator::Addition => l + r,
                    BinaryOperator::Subtraction => l - r,
                    BinaryOperator::Multiplication => l * r,
                    BinaryOperator::Division => l / r,
                    BinaryOperator::Remainder => l % r,
                    BinaryOperator::Exponential => l.powf(r),
                    BinaryOperator::BitwiseOR => f64::from(l.to_int_32() | r.to_int_32()),
                    BinaryOperator::BitwiseAnd => f64::from(l.to_int_32() & r.to_int_32()),
                    BinaryOperator::BitwiseXOR => f64::from(l.to_int_32() ^ r.to_int_32()),
                    BinaryOperator::ShiftLeft => {
                        f64::from(l.to_int_32().wrapping_shl(r.to_uint_32()))
                    }
                    BinaryOperator::ShiftRight => {
                        f64::from(l.to_int_32().wrapping_shr(r.to_uint_32()))
                    }
                    BinaryOperator::ShiftRightZeroFill => {
                        f64::from(l.to_uint_32().wrapping_shr(r.to_uint_32()))
                    }
                    _ => return Err("unsupported binary enum expression"),
                })
            }
        }
        _ => return Err("unsupported/nonconstant enum expression; code is never executed"),
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_byte_overflow_and_capacity_errors_remain_diagnostic_evidence() {
        let mut strings = Strings {
            limits: StringLimits {
                value: usize::MAX,
                total: usize::MAX,
            },
            expanded: usize::MAX - 1,
        };
        assert_eq!(
            strings.copy("xx").unwrap_err(),
            "enum string expanded-byte count overflow"
        );
        strings.expanded = 0;
        // Exceeds isize::MAX: try_reserve_exact reports capacity overflow without
        // making an enormous allocation or invoking the infallible OOM handler.
        assert_eq!(
            strings.allocate(usize::MAX).unwrap_err(),
            "enum string expanded-byte allocation failed"
        );
    }
}
