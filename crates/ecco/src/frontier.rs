//! Transient expression projection. Only lowering touches OXC nodes/semantics.
use std::ops::Range;

use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::{Semantic, SymbolId};
use oxc_span::Span;

#[cfg(test)]
thread_local! {
    static PROJECTED_NODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn projected_nodes() -> usize {
    PROJECTED_NODES.with(std::cell::Cell::get)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Contract,
    Recording,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Value,
    Coerce,
    Iterate,
    Enumerate,
}

pub(crate) enum Op<'s> {
    Primitive,
    This,
    Identifier {
        symbol: Option<SymbolId>,
        name: &'s str,
        span: Span,
    },
    Member {
        property: Option<&'s str>,
        span: Span,
        reference: bool,
    },
    Call {
        pure: bool,
        spread: bool,
    },
    New {
        pure: bool,
    },
    Binary(&'s str),
    Unary(&'s str),
    Logical,
    Conditional,
    Sequence,
    Template,
    Object {
        empty: bool,
        getter: bool,
        prototype: bool,
        unknown_read: bool,
        unsupported: bool,
    },
    Function {
        symbol: Option<SymbolId>,
        span: Span,
        arrow: bool,
    },
    Unknown,
}

pub(crate) struct Node<'s> {
    pub op: Op<'s>,
    pub inputs: Range<usize>,
    pub charge: bool,
    pub end: usize,
}

pub(crate) struct Input {
    pub node: usize,
    pub role: Role,
}

pub(crate) enum Step {
    // Visits retain contract budget boundaries; only Ready reduces summaries.
    Enter(usize),
    Ready(usize),
    // Recording continuations must precede the next operand's recording.
    Input(usize),
}

pub(crate) struct Plan<'s> {
    pub nodes: Vec<Node<'s>>,
    pub inputs: Vec<Input>,
    pub steps: Vec<Step>,
}

struct Child<'s, 'a> {
    expression: &'s Expression<'a>,
    reference: bool,
    role: Role,
}

impl<'s, 'a> Child<'s, 'a> {
    fn value(expression: &'s Expression<'a>) -> Self {
        Self {
            expression,
            reference: false,
            role: Role::Value,
        }
    }
}

enum Lower<'s, 'a> {
    Expression(Child<'s, 'a>, Option<usize>),
    Ready(usize),
    Input(usize),
}

impl<'s> Plan<'s> {
    pub fn lower<'a>(
        expression: &'s Expression<'a>,
        semantic: &Semantic<'a>,
        mode: Mode,
        limit: usize,
    ) -> Option<Self> {
        let mut plan = Self {
            nodes: Vec::new(),
            inputs: Vec::new(),
            steps: Vec::new(),
        };
        let mut pending = vec![Lower::Expression(Child::value(expression), None)];
        let mut children = Vec::new();
        while let Some(task) = pending.pop() {
            match task {
                Lower::Expression(child, input) => {
                    if plan.nodes.len() >= limit {
                        return None;
                    }
                    let id = plan.nodes.len();
                    if let Some(input) = input {
                        plan.inputs[input].node = id;
                    }
                    children.clear();
                    #[cfg(test)]
                    PROJECTED_NODES.with(|count| count.set(count.get() + 1));
                    let op = project(
                        child.expression,
                        child.reference,
                        semantic,
                        mode,
                        &mut children,
                        limit.saturating_sub(plan.inputs.len() + 1),
                    );
                    if plan.inputs.len() + children.len() >= limit {
                        return None;
                    }
                    let start = plan.inputs.len();
                    // Synthetic member-reference nodes cost no AST visit in inference.
                    let charge = !matches!(
                        &op,
                        Op::Member {
                            reference: true,
                            ..
                        }
                    );
                    plan.nodes.push(Node {
                        op,
                        inputs: start..start + children.len(),
                        charge,
                        end: 0,
                    });
                    plan.steps.push(Step::Enter(id));
                    pending.push(Lower::Ready(id));
                    plan.inputs.extend(children.iter().map(|child| Input {
                        node: usize::MAX,
                        role: child.role,
                    }));
                    for (offset, child) in children.drain(..).enumerate().rev() {
                        let input = start + offset;
                        if child.role != Role::Value {
                            pending.push(Lower::Input(input));
                        }
                        pending.push(Lower::Expression(child, Some(input)));
                    }
                }
                Lower::Ready(id) => {
                    plan.nodes[id].end = plan.steps.len();
                    plan.steps.push(Step::Ready(id));
                }
                Lower::Input(input) => {
                    plan.steps.push(Step::Input(input));
                }
            }
        }
        Some(plan)
    }
}

fn project<'s, 'a>(
    expression: &'s Expression<'a>,
    reference: bool,
    semantic: &Semantic<'a>,
    mode: Mode,
    children: &mut Vec<Child<'s, 'a>>,
    limit: usize,
) -> Op<'s> {
    let expression = expression.get_inner_expression();
    match expression {
        Expression::BooleanLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::NullLiteral(_) => Op::Primitive,
        Expression::ThisExpression(_) => Op::This,
        Expression::Identifier(identifier) => Op::Identifier {
            symbol: semantic
                .scoping()
                .get_reference(identifier.reference_id.get().expect("checked references"))
                .symbol_id(),
            name: identifier.name.as_str(),
            span: identifier.span,
        },
        Expression::StaticMemberExpression(member) => {
            children.push(Child::value(&member.object));
            Op::Member {
                property: Some(member.property.name.as_str()),
                span: member.span,
                reference,
            }
        }
        Expression::ComputedMemberExpression(member) => member_inputs(
            &member.object,
            &member.expression,
            member.span,
            reference,
            children,
        ),
        Expression::ChainExpression(chain) if mode == Mode::Recording => match &chain.expression {
            ChainElement::StaticMemberExpression(member) => {
                children.push(Child::value(&member.object));
                Op::Member {
                    property: Some(member.property.name.as_str()),
                    span: member.span,
                    reference,
                }
            }
            ChainElement::ComputedMemberExpression(member) => member_inputs(
                &member.object,
                &member.expression,
                member.span,
                reference,
                children,
            ),
            ChainElement::CallExpression(call) => call_inputs(call, mode, children, limit),
            _ => Op::Unknown,
        },
        Expression::CallExpression(call) => call_inputs(call, mode, children, limit),
        Expression::NewExpression(call) if mode == Mode::Recording => {
            children.push(Child::value(&call.callee));
            arguments(&call.arguments, mode, children, limit);
            Op::New { pure: call.pure }
        }
        Expression::BinaryExpression(binary) => {
            children.push(Child::value(&binary.left));
            children.push(Child::value(&binary.right));
            Op::Binary(binary.operator.as_str())
        }
        Expression::UnaryExpression(unary) if unary.operator.as_str() != "delete" => {
            children.push(Child::value(&unary.argument));
            Op::Unary(unary.operator.as_str())
        }
        Expression::LogicalExpression(logical) => {
            children.push(Child::value(&logical.left));
            children.push(Child::value(&logical.right));
            Op::Logical
        }
        Expression::ConditionalExpression(conditional) => {
            children.push(Child::value(&conditional.test));
            children.push(Child::value(&conditional.consequent));
            children.push(Child::value(&conditional.alternate));
            Op::Conditional
        }
        Expression::SequenceExpression(sequence) => {
            children.extend(
                sequence
                    .expressions
                    .iter()
                    .take(limit.saturating_add(1))
                    .map(Child::value),
            );
            Op::Sequence
        }
        Expression::TemplateLiteral(template) => {
            children.extend(
                template
                    .expressions
                    .iter()
                    .take(limit.saturating_add(1))
                    .map(|expression| Child {
                        expression,
                        reference: false,
                        role: Role::Coerce,
                    }),
            );
            Op::Template
        }
        Expression::ObjectExpression(object) => {
            let mut getter = false;
            let mut prototype = false;
            let mut unknown_read = false;
            let mut unsupported = false;
            for property in &object.properties {
                if children.len() > limit {
                    break;
                }
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        let proto = property
                            .key
                            .static_name()
                            .is_some_and(|name| name == "__proto__");
                        getter |= property.kind == PropertyKind::Get;
                        prototype |= !property.computed && proto;
                        unknown_read |= property.kind == PropertyKind::Get || proto;
                        if property.computed {
                            if let Some(expression) = property.key.as_expression() {
                                children.push(Child {
                                    expression,
                                    reference: false,
                                    role: Role::Coerce,
                                });
                            }
                        }
                        children.push(Child::value(&property.value));
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        if mode == Mode::Contract {
                            unsupported = true;
                            break;
                        }
                        children.push(Child {
                            expression: &spread.argument,
                            reference: false,
                            role: Role::Enumerate,
                        });
                        getter = true;
                    }
                }
            }
            Op::Object {
                empty: object.properties.is_empty(),
                getter,
                prototype,
                unknown_read,
                unsupported,
            }
        }
        Expression::FunctionExpression(function) => Op::Function {
            symbol: function.id.as_ref().and_then(|id| id.symbol_id.get()),
            span: function.span,
            arrow: false,
        },
        Expression::ArrowFunctionExpression(function) => Op::Function {
            symbol: None,
            span: function.span,
            arrow: true,
        },
        _ => Op::Unknown,
    }
}

fn member_inputs<'s, 'a>(
    object: &'s Expression<'a>,
    key: &'s Expression<'a>,
    span: Span,
    reference: bool,
    children: &mut Vec<Child<'s, 'a>>,
) -> Op<'s> {
    children.push(Child::value(object));
    children.push(Child::value(key));
    let property = match key.get_inner_expression() {
        Expression::StringLiteral(literal) => literal.value.as_str(),
        _ => None,
    };
    Op::Member {
        property,
        span,
        reference,
    }
}

fn call_inputs<'s, 'a>(
    call: &'s CallExpression<'a>,
    mode: Mode,
    children: &mut Vec<Child<'s, 'a>>,
    limit: usize,
) -> Op<'s> {
    children.push(Child {
        expression: &call.callee,
        reference: true,
        role: Role::Value,
    });
    let spread = arguments(&call.arguments, mode, children, limit);
    Op::Call {
        pure: call.pure,
        spread,
    }
}

fn arguments<'s, 'a>(
    arguments: &'s [Argument<'a>],
    mode: Mode,
    children: &mut Vec<Child<'s, 'a>>,
    limit: usize,
) -> bool {
    let mut spread = false;
    for argument in arguments {
        if children.len() > limit {
            break;
        }
        if let Argument::SpreadElement(element) = argument {
            spread = true;
            if mode == Mode::Contract {
                break;
            }
            children.push(Child {
                expression: &element.argument,
                reference: false,
                role: Role::Iterate,
            });
        } else {
            children.push(Child::value(
                argument.as_expression().expect("expression argument"),
            ));
        }
    }
    spread
}

/// The generated OXC visitors otherwise recurse through expression chains even
/// in collectors that do not evaluate expressions. Function boundaries suspend
/// this queue so their scope/compile callbacks retain normal visitor semantics.
pub(crate) trait FlatVisit<'a>: Visit<'a> {
    fn expression_queue(&mut self) -> &mut Option<Vec<&'a Expression<'a>>>;

    fn flat_expression(&mut self, expression: &Expression<'a>) {
        let expression = self.alloc(expression);
        if let Some(queue) = self.expression_queue() {
            queue.push(expression);
            return;
        }
        *self.expression_queue() = Some(vec![expression]);
        while let Some(expression) = self
            .expression_queue()
            .as_mut()
            .expect("expression queue")
            .pop()
        {
            let start = self
                .expression_queue()
                .as_ref()
                .expect("expression queue")
                .len();
            walk::walk_expression(self, expression);
            self.expression_queue().as_mut().expect("expression queue")[start..].reverse();
        }
        *self.expression_queue() = None;
    }
}
