use super::*;

fn imported(namespace: bool) -> Box<w::ImportedNameT> {
    Box::new(w::ImportedNameT {
        value: if namespace {
            w::ImportedNameValueT::NamespaceImport(Box::default())
        } else {
            w::ImportedNameValueT::NamedImport(Box::new(w::NamedImportT {
                name: "default".into(),
            }))
        },
    })
}

fn expression(value: w::HazardExprValueT) -> w::HazardExprT {
    w::HazardExprT { value }
}

fn complete_module() -> w::CachedModuleT {
    let mut module = module();
    let facts = &mut module.facts;
    facts.top_level_await = vec![w::SpanT { start: 0, end: 7 }];
    facts.effects = w::EffectReason::ENUM_VALUES
        .iter()
        .map(|&reason| w::EffectSiteT {
            reason,
            deferred: Some(w::SpanT { start: 0, end: 7 }),
            ..Default::default()
        })
        .collect();
    facts.dependencies = w::DependencyKind::ENUM_VALUES
        .iter()
        .map(|&kind| w::DependencyT {
            kind,
            specifier: "./*.js".into(),
            pattern: Some(vec!["./".into(), ".js".into()]),
            enclosing_function: Some(w::SpanT { start: 1, end: 7 }),
            ..Default::default()
        })
        .collect();
    facts.imports = [false, true]
        .into_iter()
        .map(|namespace| w::ImportBindingT {
            imported: imported(namespace),
            ..Default::default()
        })
        .collect();
    facts.exports = [
        w::ExportValueT::BindingExport(Box::default()),
        w::ExportValueT::ReExport(Box::new(w::ReExportT {
            imported: imported(true),
            ..Default::default()
        })),
        w::ExportValueT::ExpressionExport(Box::default()),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, value)| w::ExportT {
        name: (index != 1).then(|| "result".into()),
        value,
        name_span: Some(w::SpanT { start: 1, end: 2 }),
        ..Default::default()
    })
    .collect();
    facts.directives.push(w::DirectiveT {
        value: "use strict".into(),
        ..Default::default()
    });
    facts.hashbang = Some(Box::new(w::DirectiveT {
        value: "/usr/bin/env node".into(),
        ..Default::default()
    }));
    facts.invocations = [
        w::CalleeShapeT::IdentifierCallee(Box::new(w::IdentifierCalleeT {
            name: "call".into(),
            binding: Some(0),
        })),
        w::CalleeShapeT::MemberCallee(Box::new(w::MemberCalleeT {
            receiver_identifier: Some(Box::new(w::IdentifierCalleeT {
                name: "receiver".into(),
                binding: None,
            })),
            property: Some("property".into()),
            computed: true,
            ..Default::default()
        })),
        w::CalleeShapeT::OtherCallee(Box::default()),
    ]
    .into_iter()
    .zip(w::ValueUse::ENUM_VALUES)
    .map(|(shape, &value_use)| w::InvocationT {
        shape,
        value_use,
        arguments: vec![w::InvocationArgumentT {
            spread: true,
            ..Default::default()
        }],
        construct: true,
        optional_call: true,
        optional_chain: true,
        pure_: Some(w::SpanT { start: 0, end: 1 }),
        deferred: Some(w::SpanT { start: 0, end: 7 }),
        ..Default::default()
    })
    .collect();
    facts.emission.execution.contexts = w::ContextKind::ENUM_VALUES
        .iter()
        .map(|&kind| w::ExecutionContextT {
            kind,
            parent: Some(0),
            immediate: Some(0),
            ..Default::default()
        })
        .collect();
    let chain = w::CalleePlanT {
        steps: vec![
            w::ChainStepT {
                value: w::ChainStepValueT::StaticChainStep(Box::new(w::StaticChainStepT {
                    property: "method".into(),
                    optional: true,
                })),
            },
            w::ChainStepT {
                value: w::ChainStepValueT::ComputedChainStep(Box::default()),
            },
            w::ChainStepT {
                value: w::ChainStepValueT::CallChainStep(Box::default()),
            },
        ],
        ..Default::default()
    };
    let mut operations = vec![
        w::OperationKindT::BindingOperation(Box::new(w::BindingOperationT {
            shorthand: w::Shorthand::Property,
            ..Default::default()
        })),
        w::OperationKindT::ReferenceOperation(Box::new(w::ReferenceOperationT {
            shorthand: w::Shorthand::Proto,
            ..Default::default()
        })),
        w::OperationKindT::NamespaceOperation(Box::default()),
        w::OperationKindT::DependencyOperation(Box::default()),
        w::OperationKindT::SourceHintOperation(Box::default()),
        w::OperationKindT::InvocationOperation(Box::new(w::InvocationOperationT {
            lowering: Box::new(w::PureLoweringT {
                callee: Box::new(w::SourceSlotT {
                    operations: vec![0, 1],
                    ..Default::default()
                }),
                chain: Some(Box::new(chain)),
                arguments: vec![w::SourceSlotT::default()],
                guard_start: Some(0),
                crosses_await_guard: true,
            }),
            ..Default::default()
        })),
    ];
    operations.extend(
        [
            w::ExportFormT::RemoveExport(Box::default()),
            w::ExportFormT::DeclarationExport(Box::default()),
            w::ExportFormT::DefaultExport(Box::new(w::DefaultExportT {
                hoisted: true,
                ..Default::default()
            })),
        ]
        .into_iter()
        .map(|form| {
            w::OperationKindT::ExportOperation(Box::new(w::ExportOperationT {
                form,
                ..Default::default()
            }))
        }),
    );
    facts.emission.segments.push(w::SegmentRecipeT {
        source: (0..4)
            .map(|start| w::SpanT {
                start,
                end: start + 1,
            })
            .collect(),
        namespaces: std::iter::once(w::NamespaceUseT {
            reference: 0,
            access: w::NamespaceAccessT::DirectNamespaceRead(Box::new(w::DirectNamespaceReadT {
                property: "member".into(),
                ..Default::default()
            })),
        })
        .chain(
            w::NamespaceReason::ENUM_VALUES
                .iter()
                .map(|&reason| w::NamespaceUseT {
                    reference: 0,
                    access: w::NamespaceAccessT::NativeNamespace(Box::new(w::NativeNamespaceT {
                        reason,
                    })),
                }),
        )
        .collect(),
        operations: operations
            .into_iter()
            .enumerate()
            .map(|(index, kind)| w::OperationT {
                parent: (index > 0).then_some(0),
                kind,
            })
            .collect(),
        ..Default::default()
    });
    complete_ecco(facts.ecco.as_mut().unwrap());
    module
}

fn complete_ecco(file: &mut w::ParsedFileT) {
    let expressions = vec![
        expression(w::HazardExprValueT::BooleanHazard(Box::new(
            w::BooleanHazardT { value: true },
        ))),
        unknown_expression(),
        expression(w::HazardExprValueT::ReadHazard(Box::new(w::ReadHazardT {
            value: "parameter".into(),
            slot: w::Hazard::Iterate,
        }))),
        expression(w::HazardExprValueT::OrHazard(Box::new(w::OrHazardT {
            expressions: vec![unknown_expression()],
        }))),
        expression(w::HazardExprValueT::CallHazard(Box::new(w::CallHazardT {
            function: "callable".into(),
            arguments: vec![
                w::CallArgumentT {
                    value: w::CallArgumentValueT::ValueArgument(Box::new(w::ValueArgumentT {
                        name: "value".into(),
                    })),
                },
                w::CallArgumentT {
                    value: w::CallArgumentValueT::SpreadArgument(Box::new(w::SpreadArgumentT {
                        name: "rest".into(),
                    })),
                },
                w::CallArgumentT {
                    value: w::CallArgumentValueT::ForwardArgument(Box::default()),
                },
            ],
            receiver: Some("receiver".into()),
        }))),
    ];
    file.facts.definitions = expressions.into_iter().map(definition).collect();
    for (definition, returns) in file.facts.definitions.iter_mut().zip([
        w::ReturnsValueT::FreshReturn(Box::default()),
        w::ReturnsValueT::ArgumentReturn(Box::new(w::ArgumentReturnT {
            name: "parameter".into(),
        })),
        w::ReturnsValueT::VoidReturn(Box::default()),
        w::ReturnsValueT::UnknownReturn(Box::default()),
    ]) {
        definition.returns.value = returns;
        definition.parameters = vec!["parameter".into()];
        definition.rest = Some("rest".into());
        definition.variadic = true;
        definition.hazards.fields = w::Hazard::ENUM_VALUES
            .iter()
            .map(|&slot| w::HazardFieldT {
                slot,
                expression: Box::new(unknown_expression()),
            })
            .collect();
        definition.updates = vec![w::UpdateT {
            target: "parameter".into(),
            slot: Some(w::Hazard::PrototypeRead),
            expression: Box::new(unknown_expression()),
        }];
    }
    file.facts.global_symbols.push(w::GlobalSymbolT {
        name: "effectful".into(),
        read_effects: true,
    });
    file.values.push(w::ParsedValueT {
        hazards: w::HazardsT {
            property_read: true,
            to_primitive: true,
            membership: true,
            instance_of: true,
            call: true,
            construct: true,
            iterate: true,
            prototype_read: true,
        },
        definition: Some(0),
        alias: Some(0),
    });
    *file.setup.signature = w::FunctionSignatureT {
        captures: vec![0],
        parameters: vec![1],
        receiver: Some(0),
        self_binding: Some(0),
        constructible: true,
        arguments: Some(0),
    };
    file.setup.effects = Some(0);
    file.setup.returns = vec![0];
    file.setup.instructions = vec![
        w::InstructionValueT::ValueInstruction(Box::default()),
        w::InstructionValueT::AnyInstruction(Box::new(w::AnyInstructionT {
            inputs: vec![0, 1],
            ..Default::default()
        })),
        w::InstructionValueT::EqualityInstruction(Box::default()),
        w::InstructionValueT::OperationInstruction(Box::new(w::OperationInstructionT {
            hazard: w::Hazard::Call,
            ..Default::default()
        })),
        w::InstructionValueT::UpdateInstruction(Box::new(w::UpdateInstructionT {
            hazard: w::Hazard::Construct,
            ..Default::default()
        })),
        w::InstructionValueT::ReadInstruction(Box::default()),
        w::InstructionValueT::RetainInstruction(Box::new(w::RetainInstructionT {
            locals: vec![0, 1],
            ..Default::default()
        })),
        w::InstructionValueT::BindInstruction(Box::default()),
        w::InstructionValueT::JoinInstruction(Box::new(w::JoinInstructionT {
            inputs: vec![0, 1],
            ..Default::default()
        })),
        w::InstructionValueT::CallInstruction(Box::new(w::CallInstructionT {
            arguments: vec![0, 1],
            construct: true,
            ..Default::default()
        })),
        w::InstructionValueT::ClosureInstruction(Box::default()),
        w::InstructionValueT::ExternalInstruction(Box::default()),
        w::InstructionValueT::OpaqueInstruction(Box::default()),
    ]
    .into_iter()
    .map(|value| w::InstructionT { value })
    .collect();
    file.setup.sources = w::Hazard::ENUM_VALUES
        .iter()
        .enumerate()
        .map(|(effect, &hazard)| w::EffectSourceT {
            effect: effect as u64,
            value: 0,
            hazard,
        })
        .collect();
    file.functions.push(*file.setup.clone());
    file.roots.push(w::ParsedRootT {
        function: file.setup.clone(),
        ..Default::default()
    });
    file.symbols.push(w::ParsedSymbolT {
        name: "symbol".into(),
        value: 0,
        ..Default::default()
    });
    file.externals = [
        w::EccoBindingValueT::GlobalBinding(Box::new(w::GlobalBindingT {
            path: vec!["global".into()],
        })),
        w::EccoBindingValueT::ModuleBinding(Box::new(w::ModuleBindingT {
            module: "module".into(),
            path: vec!["export".into(), "member".into()],
        })),
        w::EccoBindingValueT::LocalBinding(Box::new(w::LocalBindingT {
            name: "local".into(),
        })),
    ]
    .into_iter()
    .map(|value| w::ExternalReferenceT {
        binding: Box::new(w::EccoBindingT { value }),
        ..Default::default()
    })
    .collect();
    file.matches = file
        .externals
        .iter()
        .map(|external| w::DefinitionMatchT {
            binding: external.binding.clone(),
            ..Default::default()
        })
        .collect();
}

#[test]
fn typed_views_cover_syntax_emission_execution_and_recursive_ecco_variants() {
    let module = complete_module();
    let owner = load(frame(&payload(&record(&module))).unwrap()).unwrap();
    let facts = owner.module().facts();
    assert_eq!(facts.effects().len(), 15);
    assert_eq!(
        facts
            .dependencies()
            .get(0)
            .pattern()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        ["./", ".js"]
    );
    assert_eq!(facts.exports().get(1).name(), None);
    assert_eq!(
        facts.imports().get(1).imported().value_type(),
        w::ImportedNameValue::NamespaceImport
    );
    assert_eq!(facts.invocations().get(1).pure_().unwrap().end(), 1);
    let recipe = facts.emission().segments().get(0);
    assert_eq!(recipe.source().len(), 4);
    assert_eq!(recipe.operations().get(0).parent(), None);
    assert_eq!(recipe.operations().get(1).parent(), Some(0));
    let lowering = recipe
        .operations()
        .get(5)
        .kind_as_invocation_operation()
        .unwrap()
        .lowering();
    assert_eq!(lowering.guard_start(), Some(0));
    assert!(lowering.crosses_await_guard());
    let chain = lowering.chain().unwrap();
    assert_eq!(chain.steps().len(), 3);
    assert_eq!(
        chain
            .steps()
            .get(0)
            .value_as_static_chain_step()
            .unwrap()
            .property(),
        "method"
    );
    let ecco = facts.ecco().unwrap();
    assert_eq!(ecco.setup().instructions().len(), 13);
    assert_eq!(ecco.setup().sources().len(), 8);
    assert_eq!(ecco.setup().signature().receiver(), Some(0));
    assert_eq!(ecco.setup().effects(), Some(0));
    assert_eq!(ecco.values().get(0).alias(), Some(0));
    assert!(ecco.values().get(0).hazards().prototype_read());
    let contract = ecco
        .facts()
        .definitions()
        .get(4)
        .hazards()
        .default_expression()
        .unwrap();
    let call = contract.value_as_call_hazard().unwrap();
    assert_eq!(call.receiver(), Some("receiver"));
    assert_eq!(
        call.arguments().get(2).value_type(),
        w::CallArgumentValue::ForwardArgument
    );
    let external = ecco
        .externals()
        .get(1)
        .binding()
        .value_as_module_binding()
        .unwrap();
    assert_eq!(external.module(), "module");
    assert_eq!(
        external.path().iter().collect::<Vec<_>>(),
        ["export", "member"]
    );
}

#[test]
fn opaque_cjs_is_a_distinct_body_without_ecco() {
    let mut module = module();
    module.body = w::ModuleBodyT::CommonJsBody(Box::new(w::CommonJsBodyT {
        span: w::SpanT {
            start: 0,
            end: SOURCE.len() as u32,
        },
    }));
    module.facts.commonjs = true;
    module.facts.ecco = None;
    let mut record = record(&module);
    record.config.commonjs = true;
    record.provenance.config.commonjs = true;
    let config = record.config.pack();
    let owner = WireRecord::new(
        frame(&payload(&record)).unwrap().into(),
        SOURCE,
        config,
        OXC,
    )
    .unwrap();
    assert!(owner.module().facts().ecco().is_none());
    assert_eq!(
        owner
            .module()
            .body_as_common_js_body()
            .unwrap()
            .span()
            .end() as usize,
        SOURCE.len()
    );
}
