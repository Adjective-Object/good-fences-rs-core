//! FlatBuffers intentionally permits unknown enum values for schema evolution.
//! Cache records do not: every current discriminant must be understood before use.
use eyre::{ensure, Result};

use crate::wire as w;

macro_rules! known {
    ($value:expr) => {
        ensure!(
            $value.variant_name().is_some(),
            "unknown cache enum discriminant"
        )
    };
}

macro_rules! union {
    ($value:expr) => {
        known!($value);
        ensure!($value.0 != 0, "missing cache union value")
    };
}

pub(super) fn module(module: w::CachedModule<'_>) -> Result<()> {
    union!(module.body_type());
    let facts = module.facts();
    for reference in facts.references() {
        if let Some(global) = reference.commonjs() {
            known!(global);
        }
    }
    for dependency in facts.dependencies() {
        known!(dependency.kind());
    }
    for import in facts.imports() {
        imported(import.imported())?;
    }
    for export in facts.exports() {
        union!(export.value_type());
        if let Some(reexport) = export.value_as_re_export() {
            imported(reexport.imported())?;
        }
    }
    for effect in facts.effects() {
        known!(effect.reason());
    }
    for invocation in facts.invocations() {
        union!(invocation.shape_type());
        known!(invocation.value_use());
    }
    emission(facts.emission())?;
    if let Some(parsed) = facts.ecco() {
        ecco(parsed)?;
    }
    Ok(())
}

fn imported(name: w::ImportedName<'_>) -> Result<()> {
    union!(name.value_type());
    Ok(())
}

fn emission(emission: w::EmissionFacts<'_>) -> Result<()> {
    for context in emission.execution().contexts() {
        known!(context.kind());
    }
    for segment in emission.segments() {
        ensure!(
            (1..=4).contains(&segment.source().len()),
            "invalid MultiSpan length"
        );
        for namespace in segment.namespaces() {
            union!(namespace.access_type());
            if let Some(native) = namespace.access_as_native_namespace() {
                known!(native.reason());
            }
        }
        for operation in segment.operations() {
            union!(operation.kind_type());
            if let Some(binding) = operation.kind_as_binding_operation() {
                known!(binding.shorthand());
            }
            if let Some(reference) = operation.kind_as_reference_operation() {
                known!(reference.shorthand());
            }
            if let Some(export) = operation.kind_as_export_operation() {
                union!(export.form_type());
            }
            if let Some(invocation) = operation.kind_as_invocation_operation() {
                if let Some(chain) = invocation.lowering().chain() {
                    for step in chain.steps() {
                        union!(step.value_type());
                    }
                }
            }
        }
    }
    Ok(())
}

fn binding(binding: w::EccoBinding<'_>) -> Result<()> {
    union!(binding.value_type());
    Ok(())
}

fn expression(value: w::HazardExpr<'_>) -> Result<()> {
    union!(value.value_type());
    if let Some(read) = value.value_as_read_hazard() {
        known!(read.slot());
    }
    if let Some(or) = value.value_as_or_hazard() {
        for child in or.expressions() {
            expression(child)?;
        }
    }
    if let Some(call) = value.value_as_call_hazard() {
        for argument in call.arguments() {
            union!(argument.value_type());
        }
    }
    Ok(())
}

fn fields(fields: w::HazardFields<'_>) -> Result<()> {
    if let Some(default) = fields.default_expression() {
        expression(default)?;
    }
    let mut present = 0u16;
    for field in fields.fields() {
        known!(field.slot());
        let bit = 1u16 << field.slot().0;
        ensure!(present & bit == 0, "duplicate Ecco hazard field");
        present |= bit;
        expression(field.expression())?;
    }
    Ok(())
}

pub(super) fn ecco(file: w::ParsedFile<'_>) -> Result<()> {
    for value in file.values() {
        ids(value.definition().into_iter().chain(value.alias()))?;
    }
    ids(file.symbols().iter().map(|v| v.value()))?;
    for definition in file.facts().definitions() {
        binding(definition.binding())?;
        fields(definition.hazards())?;
        fields(definition.return_hazards())?;
        union!(definition.returns().value_type());
        for update in definition.updates() {
            if let Some(slot) = update.slot() {
                known!(slot);
            }
            expression(update.expression())?;
        }
    }
    function(file.setup())?;
    for value in file.functions() {
        function(value)?;
    }
    for root in file.roots() {
        function(root.function())?;
    }
    for external in file.externals() {
        binding(external.binding())?;
    }
    for matched in file.matches() {
        binding(matched.binding())?;
    }
    Ok(())
}

fn function(function: w::OwnedFunction<'_>) -> Result<()> {
    let signature = function.signature();
    ids(signature
        .captures()
        .iter()
        .chain(signature.parameters())
        .chain(signature.receiver())
        .chain(signature.self_binding())
        .chain(signature.arguments()))?;
    ids(function.returns().iter().chain(function.effects()))?;
    let mut previous = None;
    for source in function.sources() {
        ids([source.effect(), source.value()])?;
        known!(source.hazard());
        ensure!(
            previous.is_none_or(|previous| previous < source.effect()),
            "unordered or duplicate Ecco effect source"
        );
        previous = Some(source.effect());
    }
    for instruction in function.instructions() {
        union!(instruction.value_type());
        if let Some(v) = instruction.value_as_value_instruction() {
            ids([v.value()])?;
        }
        if let Some(v) = instruction.value_as_any_instruction() {
            ids(std::iter::once(v.effect()).chain(v.inputs()))?;
        }
        if let Some(v) = instruction.value_as_equality_instruction() {
            ids([v.effect(), v.left(), v.right()])?;
        }
        if let Some(operation) = instruction.value_as_operation_instruction() {
            known!(operation.hazard());
            ids([operation.value()])?;
        }
        if let Some(update) = instruction.value_as_update_instruction() {
            known!(update.hazard());
            ids([update.value(), update.effect()])?;
        }
        if let Some(v) = instruction.value_as_read_instruction() {
            ids([v.value(), v.source()])?;
        }
        if let Some(v) = instruction.value_as_retain_instruction() {
            ids(std::iter::once(v.value()).chain(v.locals()))?;
        }
        if let Some(v) = instruction.value_as_bind_instruction() {
            ids([v.target(), v.value()])?;
        }
        if let Some(v) = instruction.value_as_join_instruction() {
            ids(std::iter::once(v.value()).chain(v.inputs()))?;
        }
        if let Some(v) = instruction.value_as_call_instruction() {
            ids([
                v.result().value(),
                v.result().effects(),
                v.function(),
                v.receiver(),
            ]
            .into_iter()
            .chain(v.arguments()))?;
        }
        if let Some(v) = instruction.value_as_closure_instruction() {
            ids([v.value(), v.function()])?;
        }
        if let Some(v) = instruction.value_as_external_instruction() {
            ids([v.value(), v.reference()])?;
        }
    }
    Ok(())
}

fn ids(values: impl IntoIterator<Item = u64>) -> Result<()> {
    for value in values {
        ensure!(usize::try_from(value).is_ok(), "Ecco ID exceeds host width");
    }
    Ok(())
}
