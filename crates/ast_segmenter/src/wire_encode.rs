//! Cold extraction encoding. No object API is used by readers.
//! AST vectors preserve extraction order; Ecco's encoder canonicalizes its maps.
use crate::{emission as em, invocation as inv, owned as o, views as v};
use cache_flatbuffers::hash128;
use cache_flatbuffers::wire as w;
use flatbuffers::{FlatBufferBuilder, WIPOffset};

macro_rules! table {
    ($ty:ident { $($field:ident: $value:expr),* $(,)? }) => {{
        let mut object = w::$ty::default();
        $(object.$field = $value;)*
        object
    }};
}
fn span(v: oxc_span::Span) -> w::SpanT {
    w::SpanT {
        start: v.start,
        end: v.end,
    }
}
fn spans(v: &[oxc_span::Span]) -> Vec<w::SpanT> {
    v.iter().copied().map(span).collect()
}
fn ids(v: &[o::BindingId]) -> Vec<u32> {
    v.iter().map(|v| v.0).collect()
}
macro_rules! enumeration {
    ($function:ident, $ty:path, $wire:ident, $($variant:ident),+ $(,)?) => {
        fn $function(v: $ty) -> w::$wire {
            match v { $(<$ty>::$variant => w::$wire::$variant,)+ }
        }
    };
}
enumeration!(
    commonjs,
    o::CommonJsGlobal,
    CommonJsGlobal,
    Require,
    Module,
    Exports
);
enumeration!(
    dependency_kind,
    o::DependencyKind,
    DependencyKind,
    Import,
    ReExport,
    DynamicImport,
    Require,
    ImportType,
    Url,
    ImportMetaEnv
);
enumeration!(
    effect_reason,
    crate::effects::EffectReason,
    EffectReason,
    UnknownExpression,
    UnknownControl,
    Invocation,
    Construction,
    PropertyRead,
    Coercion,
    Mutation,
    SpreadOrIteration,
    Destructuring,
    ClassHeritage,
    ComputedKey,
    StaticBlock,
    UnresolvedRead,
    BindingRead,
    OpaqueCommonJs
);
enumeration!(
    value_use,
    inv::ValueUse,
    ValueUse,
    Discarded,
    StatementValue,
    Required
);
enumeration!(
    lazy_wrapper_kind,
    em::LazyWrapperKind,
    LazyWrapperKind,
    Unknown,
    LazyImport,
    LazyAction
);
enumeration!(
    context_kind,
    crate::execution::ContextKind,
    ContextKind,
    Callable,
    InstanceInitializer
);
enumeration!(shorthand, em::Shorthand, Shorthand, None, Property, Proto);
enumeration!(
    namespace_reason,
    em::NamespaceReason,
    NamespaceReason,
    Other,
    Receiver,
    Tag,
    Write,
    Delete
);

fn binding(v: &o::Binding) -> w::BindingT {
    table!(BindingT {
        id: v.id().0,
        name: v.name().to_owned(),
        declarations: spans(v.declarations()),
        top_level: v.top_level(),
        function_scoped: v.function_scoped(),
        function: v.function(),
        import: v.import(),
        constant: v.constant(),
        type_only: v.type_only(),
        ambient: v.ambient()
    })
}
fn reference(v: &o::Reference) -> w::ReferenceT {
    table!(ReferenceT {
        name: v.name().to_owned(),
        span: span(v.span()),
        binding: v.binding().map(|v| v.0),
        read: v.read(),
        write: v.write(),
        type_only: v.type_only(),
        commonjs: v.commonjs().map(commonjs),
        async_deferred: v.async_deferred()
    })
}
fn dependency(v: &o::Dependency) -> w::DependencyT {
    table!(DependencyT {
        id: v.id().0,
        kind: dependency_kind(v.kind()),
        span: span(v.span()),
        specifier: v.specifier().to_owned(),
        pattern: v.pattern().map(<[String]>::to_vec),
        specifier_span: span(v.specifier_span()),
        type_only: v.type_only(),
        enclosing_function: v.enclosing_function().map(span),
        json_attribute: v.json_attribute().map(span)
    })
}
fn imported(v: &o::ImportedName) -> w::ImportedNameT {
    table!(ImportedNameT {
        value: match v {
            o::ImportedName::Name(v) =>
                w::ImportedNameValueT::NamedImport(Box::new(table!(NamedImportT {
                    name: v.clone()
                }))),
            o::ImportedName::Namespace => w::ImportedNameValueT::NamespaceImport(Box::default()),
        }
    })
}
fn import(v: &o::ImportBinding) -> w::ImportBindingT {
    table!(ImportBindingT {
        dependency: v.dependency().0,
        binding: v.binding().0,
        imported: Box::new(imported(v.imported())),
        span: span(v.span()),
        imported_span: span(v.imported_span()),
        type_only: v.type_only()
    })
}
fn export(v: &o::Export) -> w::ExportT {
    table!(ExportT {
        name: v.name().map(str::to_owned),
        value: match v.value() {
            o::ExportValue::Binding(v) =>
                w::ExportValueT::BindingExport(Box::new(table!(BindingExportT { binding: v.0 }))),
            o::ExportValue::ReExport {
                dependency,
                imported: name,
            } => w::ExportValueT::ReExport(Box::new(table!(ReExportT {
                dependency: dependency.0,
                imported: Box::new(imported(name))
            }))),
            o::ExportValue::Expression(v) =>
                w::ExportValueT::ExpressionExport(Box::new(table!(ExpressionExportT {
                    span: span(*v)
                }))),
        },
        span: span(v.span()),
        name_span: v.name_span().map(span),
        type_only: v.type_only()
    })
}
fn directive(v: &o::Directive) -> w::DirectiveT {
    table!(DirectiveT {
        value: v.value().to_owned(),
        span: span(v.span())
    })
}
fn effect(v: &crate::effects::EffectSite) -> w::EffectSiteT {
    table!(EffectSiteT {
        span: span(v.span()),
        reason: effect_reason(v.reason()),
        deferred: v.deferred().map(span)
    })
}
fn identifier(v: &inv::IdentifierCallee) -> w::IdentifierCalleeT {
    table!(IdentifierCalleeT {
        name: v.name().to_owned(),
        binding: v.binding().map(|v| v.0)
    })
}
fn invocation(v: &inv::Invocation) -> w::InvocationT {
    table!(InvocationT {
        span: span(v.span()),
        callee: span(v.callee()),
        shape: match v.shape() {
            inv::CalleeShape::Identifier(v) =>
                w::CalleeShapeT::IdentifierCallee(Box::new(identifier(v))),
            inv::CalleeShape::Member {
                receiver,
                receiver_identifier,
                property,
                computed,
            } => w::CalleeShapeT::MemberCallee(Box::new(table!(MemberCalleeT {
                receiver: span(*receiver),
                receiver_identifier: receiver_identifier
                    .as_ref()
                    .map(|v| Box::new(identifier(v))),
                property: property.clone(),
                computed: *computed
            }))),
            inv::CalleeShape::Other => w::CalleeShapeT::OtherCallee(Box::default()),
        },
        arguments: v
            .arguments()
            .iter()
            .map(|v| table!(InvocationArgumentT {
                span: span(v.span()),
                spread: v.spread()
            }))
            .collect(),
        construct: v.construct(),
        optional_call: v.optional_call(),
        optional_chain: v.optional_chain(),
        pure_: v.pure().map(span),
        deferred: v.deferred().map(span),
        value_use: value_use(v.value_use())
    })
}
fn execution(v: &crate::execution::ExecutionFacts) -> w::ExecutionFactsT {
    table!(ExecutionFactsT {
        contexts: v
            .contexts()
            .iter()
            .map(|v| table!(ExecutionContextT {
                span: span(v.span()),
                parent: v.parent(),
                kind: context_kind(v.kind()),
                immediate: v.immediate()
            }))
            .collect(),
        references: v
            .references()
            .iter()
            .map(|v| table!(OptionalIndexT { value: *v }))
            .collect(),
        invocations: v
            .invocations()
            .iter()
            .map(|v| table!(OptionalIndexT { value: *v }))
            .collect()
    })
}
fn slot(v: &em::SourceSlot) -> w::SourceSlotT {
    table!(SourceSlotT {
        span: span(v.span()),
        operations: v.operations().to_vec()
    })
}
fn chain(v: &em::CalleePlan) -> w::CalleePlanT {
    table!(CalleePlanT {
        base: Box::new(slot(v.base())),
        steps: v
            .steps()
            .iter()
            .map(|v| table!(ChainStepT {
                value: match v {
                    em::ChainStep::Static { property, optional } =>
                        w::ChainStepValueT::StaticChainStep(Box::new(table!(StaticChainStepT {
                            property: property.clone(),
                            optional: *optional
                        }))),
                    em::ChainStep::Computed { property, optional } =>
                        w::ChainStepValueT::ComputedChainStep(Box::new(table!(
                            ComputedChainStepT {
                                property: Box::new(slot(property)),
                                optional: *optional
                            }
                        ))),
                    em::ChainStep::Call { suffix } =>
                        w::ChainStepValueT::CallChainStep(Box::new(table!(CallChainStepT {
                            suffix: Box::new(slot(suffix))
                        }))),
                }
            }))
            .collect()
    })
}
fn lowering(v: &em::PureLowering) -> w::PureLoweringT {
    table!(PureLoweringT {
        callee: Box::new(slot(v.callee())),
        chain: v.chain().map(|v| Box::new(chain(v))),
        arguments: v.arguments().iter().map(slot).collect(),
        guard_start: v.guard_start(),
        crosses_await_guard: v.crosses_await_guard()
    })
}
fn operation(v: &em::Operation) -> w::OperationT {
    table!(OperationT {
        parent: v.parent(),
        kind: match v.kind() {
            em::OperationKind::Binding {
                binding,
                declaration,
                shorthand: s,
            } => w::OperationKindT::BindingOperation(Box::new(table!(BindingOperationT {
                binding: binding.0,
                declaration: *declaration,
                shorthand: shorthand(*s)
            }))),
            em::OperationKind::Reference {
                reference,
                shorthand: s,
            } => w::OperationKindT::ReferenceOperation(Box::new(table!(ReferenceOperationT {
                reference: *reference,
                shorthand: shorthand(*s)
            }))),
            em::OperationKind::Namespace { usage } =>
                w::OperationKindT::NamespaceOperation(Box::new(table!(NamespaceOperationT {
                    usage: *usage
                }))),
            em::OperationKind::Dependency { dependency } =>
                w::OperationKindT::DependencyOperation(Box::new(table!(DependencyOperationT {
                    dependency: dependency.0
                }))),
            em::OperationKind::SourceHint { span: s } =>
                w::OperationKindT::SourceHintOperation(Box::new(table!(SourceHintOperationT {
                    span: span(*s)
                }))),
            em::OperationKind::Invocation {
                invocation,
                lowering: l,
            } => w::OperationKindT::InvocationOperation(Box::new(table!(InvocationOperationT {
                invocation: *invocation,
                lowering: Box::new(lowering(l))
            }))),
            em::OperationKind::Export { span: s, form } =>
                w::OperationKindT::ExportOperation(Box::new(table!(ExportOperationT {
                    span: span(*s),
                    form: match form {
                        em::ExportForm::Remove => w::ExportFormT::RemoveExport(Box::default()),
                        em::ExportForm::Declaration { declaration } =>
                            w::ExportFormT::DeclarationExport(Box::new(table!(
                                DeclarationExportT {
                                    declaration: span(*declaration)
                                }
                            ))),
                        em::ExportForm::Default { value, hoisted } =>
                            w::ExportFormT::DefaultExport(Box::new(table!(DefaultExportT {
                                value: Box::new(slot(value)),
                                hoisted: *hoisted
                            }))),
                    }
                }))),
        }
    })
}
fn recipe(v: &em::SegmentRecipe) -> w::SegmentRecipeT {
    let p = v.placement();
    table!(SegmentRecipeT {
        source: spans(v.source().as_slice()),
        connector_only: v.connector_only(),
        placement: Box::new(table!(PlacementFactsT {
            declarations: ids(p.declarations()),
            hoisted: ids(p.hoisted()),
            instantiable: p.instantiable(),
            default_expression: p.default_expression(),
            forward_references: ids(p.forward_references()),
            eager_reads: p.eager_reads().to_vec(),
            eager_writes: p.eager_writes().to_vec(),
            writes: p.writes().to_vec()
        })),
        names: Box::new(table!(NameRestrictionsT {
            named: ids(v.names().named()),
            inference_sensitive: v.names().inference_sensitive()
        })),
        namespaces: v
            .namespaces()
            .iter()
            .map(|v| table!(NamespaceUseT {
                reference: v.reference(),
                access: match v.access() {
                    em::NamespaceAccess::DirectRead { member, property } =>
                        w::NamespaceAccessT::DirectNamespaceRead(Box::new(table!(
                            DirectNamespaceReadT {
                                member: span(*member),
                                property: property.clone()
                            }
                        ))),
                    em::NamespaceAccess::Native(reason) =>
                        w::NamespaceAccessT::NativeNamespace(Box::new(table!(NativeNamespaceT {
                            reason: namespace_reason(*reason)
                        }))),
                }
            }))
            .collect(),
        operations: v.operations().iter().map(operation).collect(),
        deferred: match v.deferred() {
            em::DeferredForm::Unsupported => w::DeferredFormT::UnsupportedDeferred(Box::default()),
            em::DeferredForm::Function => w::DeferredFormT::FunctionDeferred(Box::default()),
            em::DeferredForm::Effect => w::DeferredFormT::EffectDeferred(Box::default()),
            em::DeferredForm::Variable {
                binding,
                initializer,
            } => w::DeferredFormT::VariableDeferred(Box::new(table!(VariableDeferredT {
                binding: binding.0,
                initializer: initializer.as_ref().map(|v| Box::new(slot(v)))
            }))),
        }
    })
}
fn emission(v: &em::EmissionFacts) -> w::EmissionFactsT {
    table!(EmissionFactsT {
        source_map: v.source_map().map(|s| w::SpanT {
            start: s.start,
            end: s.end
        }),
        execution: Box::new(execution(v.execution())),
        segments: v.segments().iter().map(recipe).collect(),
        nested_bindings: ids(v.nested_bindings()),
        free_references: v.free_references().to_vec(),
        dynamic_namespaces: v
            .dynamic_namespaces()
            .iter()
            .map(|v| table!(DynamicNamespaceT {
                dependency: v.dependency().0,
                properties: v.properties().map(|values| values
                    .iter()
                    .map(|(name, receiver)| table!(NamespacePropertyT {
                        name: name.clone(),
                        receiver: *receiver
                    }))
                    .collect())
            }))
            .collect(),
        uses_this: v.uses_this(),
        lazy_modules: v
            .lazy_modules()
            .iter()
            .map(|v| table!(LazyModuleCandidateT {
                segment: v.segment(),
                binding: v.binding().0,
                constructor: v.constructor(),
                dependency: v.dependency().0,
                import_span: span(v.import_span()),
                declaration: span(v.declaration()),
                construction: span(v.construction()),
                arguments: v.arguments().iter().map(slot).collect(),
                options_properties: v.options_properties().map(|properties| properties
                    .iter()
                    .map(|p| table!(LazyOptionPropertyT {
                        key: span(p.key()),
                        value: span(p.value())
                    }))
                    .collect())
            }))
            .collect(),
        lazy_selectors: v
            .lazy_selectors()
            .iter()
            .map(|v| table!(LazySelectorCandidateT {
                segment: v.segment(),
                result_binding: v.result_binding().map(|b| b.0),
                result_declaration: v.result_declaration().map(span),
                kind: lazy_wrapper_kind(v.kind()),
                constructor: v.constructor(),
                module_binding: v.module_binding().0,
                module_reference: v.module_reference(),
                getter_reference: v.getter_reference(),
                property: v.property().to_owned(),
                property_span: span(v.property_span()),
                construction: span(v.construction()),
                arguments: v.arguments().iter().map(slot).collect()
            }))
            .collect()
    })
}
fn facts<'b>(v: &o::ModuleFacts, b: &mut FlatBufferBuilder<'b>) -> WIPOffset<w::ModuleFacts<'b>> {
    let policies = v
        .policy_violations()
        .iter()
        .map(|v| {
            table!(PolicyViolationT {
                policy: w::PolicyName(v.policy() as u16),
                span: span(v.span())
            })
            .pack(b)
        })
        .collect::<Vec<_>>();
    let policy_violations = Some(b.create_vector(&policies));
    let ecco = v.ecco().map(|v| v.encode_flatbuffer(b));
    let hash = w::Fingerprint::new(&v.ecco_declarations_hash().bytes());
    let top_level_await = b.create_vector(
        &v.top_level_await()
            .iter()
            .map(|v| w::Span::new(v.start, v.end))
            .collect::<Vec<_>>(),
    );
    macro_rules! vector {
        ($name:ident, $encode:ident) => {{
            let offsets = v
                .$name()
                .iter()
                .map(|v| $encode(v).pack(b))
                .collect::<Vec<_>>();
            Some(b.create_vector(&offsets))
        }};
    }
    let effects = vector!(effects, effect);
    let invocations = vector!(invocations, invocation);
    let emission = emission(v.emission()).pack(b);
    let bindings = vector!(bindings, binding);
    let references = vector!(references, reference);
    let dependencies = vector!(dependencies, dependency);
    let imports = vector!(imports, import);
    let exports = vector!(exports, export);
    let directives = vector!(directives, directive);
    let hashbang = v.hashbang().map(|v| directive(v).pack(b));
    w::ModuleFacts::create(
        b,
        &w::ModuleFactsArgs {
            policy_violations,
            ecco,
            ecco_declarations_hash: Some(&hash),
            commonjs: v.commonjs(),
            top_level_await: Some(top_level_await),
            effects,
            invocations,
            emission: Some(emission),
            bindings,
            references,
            dependencies,
            imports,
            exports,
            directives,
            hashbang,
        },
    )
}

impl crate::ParserConfig {
    pub fn wire(self) -> w::ParserConfig {
        w::ParserConfig::new(
            self.typescript(),
            self.jsx(),
            self.definition(),
            self.commonjs(),
            self.detect_esm(),
        )
    }
}

impl crate::ParsedModule {
    /// Encode sealed extraction only; byte-backed modules reuse their exact buffer.
    pub fn to_flatbuffer(&self, source: &str, oxc: &str) -> eyre::Result<Vec<u8>> {
        let ranges = self.segments().iter().map(|s| s.span).collect::<Vec<_>>();
        self.provenance().verify(
            source,
            self.provenance().config(),
            self.body(),
            &ranges,
            self.facts(),
        )?;
        if let Some(wire) = self.provenance().wire_record() {
            eyre::ensure!(wire.root().oxc() == oxc, "cache parser revision mismatch");
            return Ok(wire.bytes().to_vec());
        }
        let v::ModuleFacts::Owned(owned) = self.facts() else {
            eyre::bail!("wire facts already have immutable encoding");
        };
        let encoded;
        let module_bytes = match self.provenance().module_bytes() {
            Some(bytes) => bytes,
            None => {
                encoded = owned.encode_module(self.body(), &ranges);
                &encoded
            }
        };
        let hash = w::Hash128::new(&hash128(source.as_bytes()));
        let records_hash = w::Hash128::new(&self.provenance().records_hash());
        let config = self.provenance().config().wire();
        let mut outer = FlatBufferBuilder::new();
        let module = outer.create_vector(module_bytes);
        let oxc = outer.create_string(oxc);
        let provenance = w::SourceProvenance::create(
            &mut outer,
            &w::SourceProvenanceArgs {
                source_hash: Some(&hash),
                config: Some(&config),
                records_hash: Some(&records_hash),
            },
        );
        let root = w::CacheRecord::create(
            &mut outer,
            &w::CacheRecordArgs {
                schema: cache_flatbuffers::SCHEMA_VERSION,
                extractor: cache_flatbuffers::EXTRACTOR_VERSION,
                oxc: Some(oxc),
                source_len: source.len() as u64,
                source_hash: Some(&hash),
                config: Some(&config),
                module: Some(module),
                provenance: Some(provenance),
            },
        );
        w::finish_size_prefixed_cache_record_buffer(&mut outer, root);
        cache_flatbuffers::frame(outer.finished_data())
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static MODULE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl o::ModuleFacts {
    /// Construction-only typed module bytes. This does not create provenance or
    /// a checked parsed module; consumers must use the sealed cache boundary.
    pub fn encode_module(&self, body: crate::ModuleBody, ranges: &[oxc_span::Span]) -> Vec<u8> {
        #[cfg(test)]
        MODULE_ENCODINGS.with(|count| count.set(count.get() + 1));
        let mut b = FlatBufferBuilder::new();
        let facts = facts(self, &mut b);
        let segments = b.create_vector(
            &ranges
                .iter()
                .map(|v| w::Span::new(v.start, v.end))
                .collect::<Vec<_>>(),
        );
        let (body_type, body) = match body {
            crate::ModuleBody::Esm => (
                w::ModuleBody::EsmBody,
                w::EsmBody::create(&mut b, &w::EsmBodyArgs {}).as_union_value(),
            ),
            crate::ModuleBody::CommonJs { span: s } => {
                let span = w::Span::new(s.start, s.end);
                (
                    w::ModuleBody::CommonJsBody,
                    w::CommonJsBody::create(&mut b, &w::CommonJsBodyArgs { span: Some(&span) })
                        .as_union_value(),
                )
            }
        };
        let module = w::CachedModule::create(
            &mut b,
            &w::CachedModuleArgs {
                body_type,
                body: Some(body),
                segments: Some(segments),
                facts: Some(facts),
            },
        );
        b.finish(module, Some(cache_flatbuffers::MODULE_IDENTIFIER));
        b.finished_data().to_vec()
    }
}
