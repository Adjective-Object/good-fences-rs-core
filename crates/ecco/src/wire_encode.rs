//! Cold-path construction only. Readers use `views`, never the object API.
use cache_flatbuffers::wire as w;
use flatbuffers::{FlatBufferBuilder, WIPOffset};

use crate::solver::Instruction;
use crate::*;

// flatc's object tables are non_exhaustive across crate boundaries.
macro_rules! table {
    ($ty:ident { $($field:ident: $value:expr),* $(,)? }) => {{
        let mut object = w::$ty::default();
        $(object.$field = $value;)*
        object
    }};
}

impl ParsedFile {
    /// Encode into the caller's builder, allowing a containing AST module to
    /// embed typed Ecco tables directly (not a second serialized blob).
    pub fn encode_flatbuffer<'b>(
        &self,
        builder: &mut FlatBufferBuilder<'b>,
    ) -> WIPOffset<w::ParsedFile<'b>> {
        file(self).pack(builder)
    }

    /// Standalone precursor bytes; production cache framing remains unchanged.
    pub fn to_flatbuffer(&self) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let root = self.encode_flatbuffer(&mut builder);
        builder.finish(root, None);
        builder.finished_data().to_vec()
    }
}

fn span(v: oxc_span::Span) -> w::SpanT {
    w::SpanT {
        start: v.start,
        end: v.end,
    }
}
fn hazard(v: Hazard) -> w::Hazard {
    w::Hazard(v as u8)
}
fn hazards(v: Hazards) -> w::HazardsT {
    w::HazardsT {
        property_read: v.get(Hazard::PropertyRead),
        to_primitive: v.get(Hazard::ToPrimitive),
        membership: v.get(Hazard::In),
        instance_of: v.get(Hazard::Instanceof),
        call: v.get(Hazard::Call),
        construct: v.get(Hazard::Construct),
        iterate: v.get(Hazard::Iterate),
        prototype_read: v.get(Hazard::PrototypeRead),
    }
}
fn binding(v: &Binding) -> w::EccoBindingT {
    table!(EccoBindingT {
        value: match v {
            Binding::Global(path) =>
                w::EccoBindingValueT::GlobalBinding(Box::new(table!(GlobalBindingT {
                    path: path.clone()
                }))),
            Binding::Module { module, path } =>
                w::EccoBindingValueT::ModuleBinding(Box::new(table!(ModuleBindingT {
                    module: module.clone(),
                    path: path.clone()
                }))),
            Binding::Local(name) =>
                w::EccoBindingValueT::LocalBinding(Box::new(table!(LocalBindingT {
                    name: name.clone()
                }))),
        }
    })
}
fn argument(v: &CallArgument) -> w::CallArgumentT {
    table!(CallArgumentT {
        value: match v {
            CallArgument::Value(name) =>
                w::CallArgumentValueT::ValueArgument(Box::new(table!(ValueArgumentT {
                    name: name.clone()
                }))),
            CallArgument::Spread(name) =>
                w::CallArgumentValueT::SpreadArgument(Box::new(table!(SpreadArgumentT {
                    name: name.clone()
                }))),
            CallArgument::Forward => w::CallArgumentValueT::ForwardArgument(Box::default()),
        }
    })
}
fn expression(v: &HazardExpr) -> w::HazardExprT {
    table!(HazardExprT {
        value: match v {
            HazardExpr::Bool(value) =>
                w::HazardExprValueT::BooleanHazard(Box::new(table!(BooleanHazardT {
                    value: *value
                }))),
            HazardExpr::Unknown => w::HazardExprValueT::UnknownHazard(Box::default()),
            HazardExpr::Read { value, slot } =>
                w::HazardExprValueT::ReadHazard(Box::new(table!(ReadHazardT {
                    value: value.clone(),
                    slot: hazard(*slot)
                }))),
            HazardExpr::Or(v) => w::HazardExprValueT::OrHazard(Box::new(table!(OrHazardT {
                expressions: v.iter().map(expression).collect()
            }))),
            HazardExpr::Call {
                function,
                arguments,
                receiver,
            } => w::HazardExprValueT::CallHazard(Box::new(table!(CallHazardT {
                function: function.clone(),
                arguments: arguments.iter().map(argument).collect(),
                receiver: receiver.clone()
            }))),
        }
    })
}
fn fields(v: &HazardFields) -> w::HazardFieldsT {
    table!(HazardFieldsT {
        default_expression: v.default_expression().map(|v| Box::new(expression(v))),
        fields: v
            .fields()
            .iter()
            .map(|(slot, v)| table!(HazardFieldT {
                slot: hazard(*slot),
                expression: Box::new(expression(v))
            }))
            .collect(),
    })
}
fn returns(v: &Returns) -> w::ReturnsT {
    table!(ReturnsT {
        value: match v {
            Returns::Fresh => w::ReturnsValueT::FreshReturn(Box::default()),
            Returns::Argument(name) =>
                w::ReturnsValueT::ArgumentReturn(Box::new(table!(ArgumentReturnT {
                    name: name.clone()
                }))),
            Returns::Void => w::ReturnsValueT::VoidReturn(Box::default()),
            Returns::Unknown => w::ReturnsValueT::UnknownReturn(Box::default()),
        }
    })
}
fn definition(v: &Definition) -> w::DefinitionT {
    table!(DefinitionT {
        binding: Box::new(binding(v.binding())),
        parameters: v.parameters().to_vec(),
        rest: v.rest().map(str::to_owned),
        variadic: v.variadic(),
        hazards: Box::new(fields(v.hazards())),
        returns: Box::new(returns(v.returns())),
        return_hazards: Box::new(fields(v.return_hazards())),
        updates: v
            .updates()
            .iter()
            .map(|v| table!(UpdateT {
                target: v.target().into(),
                slot: v.slot().map(hazard),
                expression: Box::new(expression(v.expression())),
            }))
            .collect(),
    })
}
fn instruction(v: &Instruction) -> w::InstructionT {
    use w::InstructionValueT as I;
    table!(InstructionT {
        value: match v {
            Instruction::Value(value, h) =>
                I::ValueInstruction(Box::new(table!(ValueInstructionT {
                    value: value.0 as u64,
                    hazards: hazards(*h)
                }))),
            Instruction::Any(effect, inputs) =>
                I::AnyInstruction(Box::new(table!(AnyInstructionT {
                    effect: effect.0 as u64,
                    inputs: inputs.iter().map(|v| v.0 as u64).collect()
                }))),
            Instruction::Equality(effect, left, right) =>
                I::EqualityInstruction(Box::new(table!(EqualityInstructionT {
                    effect: effect.0 as u64,
                    left: left.0 as u64,
                    right: right.0 as u64
                }))),
            Instruction::Operation(value, h) =>
                I::OperationInstruction(Box::new(table!(OperationInstructionT {
                    value: value.0 as u64,
                    hazard: hazard(*h)
                }))),
            Instruction::Update(value, h, effect) =>
                I::UpdateInstruction(Box::new(table!(UpdateInstructionT {
                    value: value.0 as u64,
                    hazard: hazard(*h),
                    effect: effect.0 as u64
                }))),
            Instruction::Read(value, source) =>
                I::ReadInstruction(Box::new(table!(ReadInstructionT {
                    value: value.0 as u64,
                    source: source.0 as u64
                }))),
            Instruction::Retain(value, locals) =>
                I::RetainInstruction(Box::new(table!(RetainInstructionT {
                    value: value.0 as u64,
                    locals: locals.iter().map(|v| v.0 as u64).collect()
                }))),
            Instruction::Bind(target, value) =>
                I::BindInstruction(Box::new(table!(BindInstructionT {
                    target: target.0 as u64,
                    value: value.0 as u64
                }))),
            Instruction::Join(value, inputs) =>
                I::JoinInstruction(Box::new(table!(JoinInstructionT {
                    value: value.0 as u64,
                    inputs: inputs.iter().map(|v| v.0 as u64).collect()
                }))),
            Instruction::Call(result, function, arguments, receiver, pure) =>
                I::CallInstruction(Box::new(table!(CallInstructionT {
                    result: Box::new(table!(CallResultT {
                        value: result.value().0 as u64,
                        effects: result.effects().0 as u64
                    })),
                    function: function.0 as u64,
                    arguments: arguments.iter().map(|v| v.0 as u64).collect(),
                    receiver: receiver.0 as u64,
                    pure_: *pure,
                }))),
            Instruction::Closure(value, function) =>
                I::ClosureInstruction(Box::new(table!(ClosureInstructionT {
                    value: value.0 as u64,
                    function: *function as u64
                }))),
            Instruction::External(value, reference) =>
                I::ExternalInstruction(Box::new(table!(ExternalInstructionT {
                    value: value.0 as u64,
                    reference: *reference as u64
                }))),
            Instruction::Opaque => I::OpaqueInstruction(Box::default()),
        }
    })
}
fn function(v: &OwnedFunction) -> w::OwnedFunctionT {
    let s = v.signature();
    table!(OwnedFunctionT {
        signature: Box::new(table!(FunctionSignatureT {
            captures: s.captures().iter().map(|v| v.0 as u64).collect(),
            parameters: s.parameters().iter().map(|v| v.0 as u64).collect(),
            receiver: s.receiver().map(|v| v.0 as u64),
            self_binding: s.self_binding().map(|v| v.0 as u64),
            constructible: s.constructible(),
            arguments: s.arguments().map(|v| v.0 as u64),
        })),
        instructions: v.instructions().iter().map(instruction).collect(),
        returns: v.returns().iter().map(|v| v.0 as u64).collect(),
        effects: v.effects().map(|v| v.0 as u64),
        sources: v
            .sources()
            .iter()
            .map(|(effect, (value, h))| table!(EffectSourceT {
                effect: effect.0 as u64,
                value: value.0 as u64,
                hazard: hazard(*h)
            }))
            .collect(),
    })
}
fn file(v: &ParsedFile) -> w::ParsedFileT {
    table!(ParsedFileT {
        version: v.version(),
        facts: Box::new(table!(LocalFactsT {
            definitions: v
                .declarations()
                .definitions()
                .iter()
                .map(definition)
                .collect(),
            global_symbols: v
                .declarations()
                .global_symbols()
                .iter()
                .map(|v| table!(GlobalSymbolT {
                    name: v.name().into(),
                    read_effects: v.read_effects()
                }))
                .collect(),
        })),
        values: v
            .values()
            .iter()
            .map(|v| table!(ParsedValueT {
                hazards: hazards(v.hazards()),
                definition: v.definition().map(|v| v as u64),
                alias: v.alias().map(|v| v.0 as u64)
            }))
            .collect(),
        functions: v.functions().iter().map(function).collect(),
        setup: Box::new(function(v.setup())),
        symbols: v
            .symbols()
            .iter()
            .map(|v| table!(ParsedSymbolT {
                id: v.id().0,
                name: v.name().into(),
                span: span(v.span()),
                value: v.value().0 as u64
            }))
            .collect(),
        roots: v
            .roots()
            .iter()
            .map(|v| table!(ParsedRootT {
                span: span(v.span()),
                function: Box::new(function(v.function()))
            }))
            .collect(),
        externals: v
            .external_references()
            .iter()
            .map(|v| table!(ExternalReferenceT {
                binding: Box::new(binding(v.binding())),
                span: span(v.span())
            }))
            .collect(),
        matches: v
            .definition_matches()
            .iter()
            .map(|v| table!(DefinitionMatchT {
                span: span(v.span()),
                binding: Box::new(binding(v.binding()))
            }))
            .collect(),
    })
}
