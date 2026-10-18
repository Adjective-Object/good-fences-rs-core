//! Checked source edits lower module-scope binding operations to shared cells.
use super::*;
use ast_segmenter::owned::{Binding, ModuleFacts};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

pub(super) struct Lowered {
    pub segments: BTreeMap<u32, String>,
    pub hoisted_default: bool,
}

pub(super) fn lower(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    file: u32,
    module: &Module<'_>,
    chunk_runtime: &str,
) -> eyre::Result<Lowered> {
    let mut source = String::new();
    let mut ranges = Vec::new();
    // PURE recipes run before lowering, while every occurrence still has its
    // checked original byte range. New facts below describe generated syntax.
    for (index, segment) in module.parsed.segments.iter().enumerate() {
        let segment_key = SegmentKey::new(file, index as u32);
        if !retained(plan, GraphNode::Segment(segment_key)) {
            continue;
        }
        let recipes = plan
            .retention
            .effects
            .iter()
            .find(|e| e.key == segment_key)
            .map(|e| e.summary.rewrites.as_slice())
            .unwrap_or_default();
        let rendered = crate::emit::render_segment(
            module.source,
            segment.span,
            recipes,
            &module.parsed.facts,
        )?;
        let start = source.len() as u32;
        source.push_str(&rendered);
        ranges.push((Span::new(start, source.len() as u32), index as u32));
        source.push('\n');
    }
    crate::rewrite::count_output_parse();
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        &source,
        std::path::Path::new("generated.mjs"),
        ast_segmenter::ModuleFormat::Esm,
    )?;
    let allocator = Allocator::default();
    crate::rewrite::count_output_parse();
    let ast = Parser::new(&allocator, &source, SourceType::mjs()).parse();
    let mut runtime = "__gf$".to_owned();
    while parsed
        .facts
        .bindings
        .iter()
        .map(|b| &b.name)
        .chain(parsed.facts.references.iter().map(|r| &r.name))
        .any(|name| name.starts_with(&runtime))
    {
        runtime.push('$');
    }
    let mut transform = Lower {
        graph,
        file,
        source: &source,
        facts: &parsed.facts,
        runtime: &runtime,
        class_this: Vec::new(),
        original_ranges: &ranges,
        edits: Vec::new(),
        shorthand: BTreeSet::new(),
        target: BTreeMap::new(),
        generator_depth: 0,
        this_depth: 0,
        iterator_left: false,
        iterator_labels: Vec::new(),
        fresh: 0,
    };
    let mut shorthand = Shorthand::default();
    shorthand.visit_program(&ast.program);
    transform.shorthand = shorthand.0;
    transform.class_this = shorthand.1;
    // Declaration identifiers use initialization setters; reference writes use
    // ordinary setters. Destructuring preserves native left-to-right semantics.
    for b in parsed
        .facts
        .bindings
        .iter()
        .filter(|b| b.top_level && !b.import)
    {
        for span in &b.declarations {
            transform.target.insert(span.start, (b, b.function_scoped));
        }
    }
    for reference in parsed.facts.references.iter().filter(|r| !r.type_only) {
        let Some(b) = parsed
            .facts
            .bindings
            .iter()
            .find(|b| b.top_level && Some(b.id) == reference.binding)
        else {
            continue;
        };
        let value = format!("{runtime}.cell({}).v", key(graph, file, &b.name));
        let value = if reference.write {
            value
        } else {
            format!("(0,{value})")
        };
        transform.edits.push(crate::rewrite::Edit {
            span: reference.span,
            replacement: transform.shorthand(reference.span, &b.name, value),
        });
    }
    transform.visit_program(&ast.program);
    let mut result = BTreeMap::new();
    let mut hoisted_default = false;
    for &(range, index) in &ranges {
        let mut instantiate = String::new();
        for b in parsed
            .facts
            .bindings
            .iter()
            .filter(|b| b.top_level && !b.type_only)
        {
            if !b.declarations.iter().any(|s| contains(range, *s)) {
                continue;
            }
            let original = module
                .parsed
                .facts
                .bindings
                .iter()
                .find(|v| v.top_level && v.name == b.name);
            if b.import {
                let original = original.ok_or_eyre("missing source import binding")?;
                let import = module
                    .parsed
                    .facts
                    .imports
                    .iter()
                    .find(|i| i.binding == original.id)
                    .unwrap();
                instantiate.push_str(&format!(
                    "{runtime}.alias({}, ()=>({}));\n",
                    key(graph, file, &b.name),
                    imported_value(graph, file, import)?
                ));
            } else {
                let kind = if b.function_scoped || b.function {
                    "var"
                } else if b.constant {
                    "const"
                } else {
                    "let"
                };
                instantiate.push_str(&format!(
                    "{runtime}.cell({}, {}, {});\n",
                    key(graph, file, &b.name),
                    quote(&b.name),
                    quote(kind)
                ));
            }
        }
        let mut execution = transform.render(range)?;
        // Hoisted declarations are instantiated when their owning code arrives,
        // before any original module in the root starts evaluation.
        for statement in ast
            .program
            .body
            .iter()
            .filter(|s| contains(range, s.span()))
        {
            let function = match statement {
                Statement::FunctionDeclaration(f) => Some(f.as_ref()),
                Statement::ExportNamedDeclaration(e) => match &e.declaration {
                    Some(Declaration::FunctionDeclaration(f)) => Some(f.as_ref()),
                    _ => None,
                },
                Statement::ExportDefaultDeclaration(e) => match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => Some(f.as_ref()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(function) = function {
                let name = function
                    .id
                    .as_ref()
                    .map(|i| i.name.as_str())
                    .unwrap_or("*default*");
                hoisted_default |= name == "*default*";
                let public_name = if name == "*default*" { "default" } else { name };
                let mut function_edits: Vec<_> = transform
                    .edits
                    .iter()
                    .filter(|e| contains(function.span, e.span))
                    .cloned()
                    .collect();
                if let Some(identifier) = &function.id {
                    function_edits.push(crate::rewrite::Edit {
                        span: identifier.span,
                        replacement: String::new(),
                    });
                }
                let function_source = render(&source, function.span, &function_edits)?;
                instantiate.push_str(&format!(
                    "{runtime}.cell({}, {}, 'var').i = (()=>{{'use strict'; return ({{[{}]:({function_source})}})[{}];}})();\n",
                    key(graph,file,name),quote(public_name),quote(public_name),quote(public_name)
                ));
                execution.clear();
            }
        }
        result.insert(index, format!(
            "(function({runtime}) {{\n{instantiate}{runtime}.segment({}, {index}, function*() {{'use strict';\n{execution}\n}});\n}})({chunk_runtime});\n",
            quote(&id(graph,file))
        ));
    }
    Ok(Lowered {
        segments: result,
        hoisted_default,
    })
}

fn render(source: &str, range: Span, edits: &[crate::rewrite::Edit]) -> eyre::Result<String> {
    let edits: Vec<_> = edits
        .iter()
        .filter(|e| contains(range, e.span))
        .map(|e| crate::rewrite::Edit {
            span: Span::new(e.span.start - range.start, e.span.end - range.start),
            replacement: e.replacement.clone(),
        })
        .collect();
    crate::rewrite::apply(&source[range.start as usize..range.end as usize], &edits)
}

struct Lower<'a> {
    graph: &'a SourceGraph,
    file: u32,
    source: &'a str,
    facts: &'a ModuleFacts,
    runtime: &'a str,
    class_this: Vec<Span>,
    original_ranges: &'a [(Span, u32)],
    edits: Vec<crate::rewrite::Edit>,
    shorthand: BTreeSet<u32>,
    target: BTreeMap<u32, (&'a Binding, bool)>,
    generator_depth: usize,
    this_depth: usize,
    iterator_left: bool,
    iterator_labels: Vec<String>,
    fresh: usize,
}

impl Lower<'_> {
    fn fresh(&mut self) -> String {
        let name = format!("{}temp{}", self.runtime, self.fresh);
        self.fresh += 1;
        name
    }
    fn edit(&mut self, span: Span, replacement: impl Into<String>) {
        self.edits.retain(|e| !contains(span, e.span));
        self.edits.push(crate::rewrite::Edit {
            span,
            replacement: replacement.into(),
        });
    }
    fn render(&self, span: Span) -> eyre::Result<String> {
        render(self.source, span, &self.edits)
    }
    fn shorthand(&self, span: Span, name: &str, value: String) -> String {
        if self.shorthand.contains(&span.start) {
            format!("{name}: {value}")
        } else {
            value
        }
    }

    fn named(&self, expression: &Expression<'_>, name: &str) -> String {
        let value = self
            .render(expression.span())
            .expect("checked initializer edits");
        let anonymous = match expression.get_inner_expression() {
            Expression::ArrowFunctionExpression(_) => true,
            Expression::FunctionExpression(f) => f.id.is_none(),
            Expression::ClassExpression(c) => c.id.is_none(),
            _ => false,
        };
        if anonymous {
            format!("({{[{}]:({value})}})[{}]", quote(name), quote(name))
        } else {
            value
        }
    }

    fn captured_global(&self, name: &str) -> bool {
        [
            RUNTIME,
            "module",
            "exports",
            "require",
            "__filename",
            "__dirname",
        ]
        .contains(&name)
            || (name == "arguments" && self.this_depth == 0)
    }

    fn initialize(&mut self, pattern: &BindingPattern<'_>, value: &str) -> String {
        let runtime = self.runtime;
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                let key = key(self.graph, self.file, identifier.name.as_str());
                format!("{runtime}.cell({key}).i = {value}; yield {{init:{key}}};\n")
            }
            BindingPattern::AssignmentPattern(pattern) => {
                let temp = self.fresh();
                let default = if let BindingPattern::BindingIdentifier(identifier) = &pattern.left {
                    self.named(&pattern.right, identifier.name.as_str())
                } else {
                    self.render(pattern.right.span())
                        .expect("checked default initializer")
                };
                let next = self.initialize(&pattern.left, &temp);
                format!(
                    "let {temp} = {value}; if ({temp} === undefined) {temp} = ({default}); {next}"
                )
            }
            BindingPattern::ObjectPattern(pattern) => {
                let object = self.fresh();
                let excluded = self.fresh();
                let mut code = format!(
                    "const {object} = {runtime}.coercible({value}); const {excluded} = [];\n"
                );
                for property in &pattern.properties {
                    let property_key = self.fresh();
                    let expression = if property.computed {
                        format!(
                            "{runtime}.propertyKey({})",
                            self.render(property.key.span())
                                .expect("checked computed binding key")
                        )
                    } else {
                        // A computed object literal applies the same ToPropertyKey
                        // operation for identifier, numeric and string spellings.
                        match &property.key {
                            PropertyKey::StaticIdentifier(identifier) => {
                                quote(identifier.name.as_str())
                            }
                            _ => format!(
                                "{runtime}.propertyKey({})",
                                self.render(property.key.span())
                                    .expect("checked binding key")
                            ),
                        }
                    };
                    let temp = self.fresh();
                    code.push_str(&format!("const {property_key} = {expression}; {excluded}.push({property_key}); const {temp} = {object}[{property_key}];\n"));
                    code.push_str(&self.initialize(&property.value, &temp));
                }
                if let Some(rest) = &pattern.rest {
                    let temp = self.fresh();
                    code.push_str(&format!(
                        "const {temp} = {runtime}.rest({object},{excluded});\n"
                    ));
                    code.push_str(&self.initialize(&rest.argument, &temp));
                }
                code
            }
            BindingPattern::ArrayPattern(pattern) => {
                let iterator = self.fresh();
                let throwing = self.fresh();
                let error = self.fresh();
                let mut code = format!("const {iterator} = {runtime}.iterator({value}); let {throwing} = false; try {{\n");
                for element in &pattern.elements {
                    let temp = self.fresh();
                    code.push_str(&format!(
                        "const {temp} = {runtime}.iteratorStep({iterator},{});\n",
                        element.is_some()
                    ));
                    if let Some(element) = element {
                        code.push_str(&self.initialize(element, &temp));
                    }
                }
                if let Some(rest) = &pattern.rest {
                    let temp = self.fresh();
                    code.push_str(&format!(
                        "const {temp} = {runtime}.iteratorRest({iterator});\n"
                    ));
                    code.push_str(&self.initialize(&rest.argument, &temp));
                }
                code.push_str(&format!("}} catch ({error}) {{ {throwing} = true; throw {error}; }} finally {{ {runtime}.iteratorClose({iterator},{throwing}); }}\n"));
                code
            }
        }
    }
}

impl<'a> Visit<'a> for Lower<'_> {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        self.edit(declaration.span, "");
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        self.edit(declaration.span, "");
    }

    fn visit_export_named_declaration(&mut self, declaration: &ExportNamedDeclaration<'a>) {
        if let Some(inner) = &declaration.declaration {
            walk::walk_declaration(self, inner);
            self.edit(Span::new(declaration.span.start, inner.span().start), "");
        } else {
            self.edit(declaration.span, "");
        }
    }

    fn visit_export_default_declaration(&mut self, declaration: &ExportDefaultDeclaration<'a>) {
        let runtime = self.runtime;
        walk::walk_export_default_declaration(self, declaration);
        match &declaration.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(_) => {}
            ExportDefaultDeclarationKind::ClassDeclaration(class) if class.id.is_some() => {
                self.edit(Span::new(declaration.span.start, class.span.start), "");
            }
            value => {
                let range = value.span();
                let value = self.render(range).expect("checked generated edits");
                self.edit(declaration.span,format!(
                    "{runtime}.cell({key}, 'default', 'const').i = ({{['default']:({value})}}).default; yield {{init:{key}}};",
                    key=key(self.graph,self.file,"*default*")
                ));
            }
        }
    }

    fn visit_variable_declaration(&mut self, declaration: &VariableDeclaration<'a>) {
        let runtime = self.runtime;
        walk::walk_variable_declaration(self, declaration);
        let top_level = declaration.declarations.iter().any(|d| {
            self.target
                .keys()
                .any(|&start| d.id.span().start <= start && start < d.id.span().end)
        });
        if !top_level {
            return;
        }
        if declaration.kind != VariableDeclarationKind::Var {
            let mut code = String::new();
            for declarator in &declaration.declarations {
                let temp = self.fresh();
                let value = match &declarator.init {
                    Some(value) => match &declarator.id {
                        BindingPattern::BindingIdentifier(identifier) => {
                            self.named(value, identifier.name.as_str())
                        }
                        _ => self
                            .render(value.span())
                            .expect("checked lexical initializer"),
                    },
                    None => "void 0".into(),
                };
                code.push_str(&format!("const {temp} = ({value});\n"));
                code.push_str(&self.initialize(&declarator.id, &temp));
            }
            self.edit(declaration.span, code);
            return;
        }
        let mut values = Vec::new();
        for declarator in &declaration.declarations {
            let mut edits: Vec<_> = self
                .edits
                .iter()
                .filter(|e| contains(declarator.span, e.span))
                .cloned()
                .collect();
            for (&start, &(binding, var)) in &self.target {
                let Some(span) = binding
                    .declarations
                    .iter()
                    .find(|s| s.start == start && contains(declarator.id.span(), **s))
                else {
                    continue;
                };
                let field = if var { "v" } else { "i" };
                let value = format!(
                    "{runtime}.cell({}).{field}",
                    key(self.graph, self.file, &binding.name)
                );
                edits.push(crate::rewrite::Edit {
                    span: *span,
                    replacement: self.shorthand(*span, &binding.name, value),
                });
            }
            let pattern =
                render(self.source, declarator.id.span(), &edits).expect("checked binding edits");
            let value = match &declarator.init {
                Some(value) => {
                    let initializer = match &declarator.id {
                        BindingPattern::BindingIdentifier(identifier) => {
                            self.named(value, identifier.name.as_str())
                        }
                        _ => self
                            .render(value.span())
                            .expect("checked initializer edits"),
                    };
                    format!("{pattern} = ({initializer})")
                }
                None if self.iterator_left => pattern,
                None if declaration.kind == VariableDeclarationKind::Var => "void 0".into(),
                None => format!("{pattern} = void 0"),
            };
            values.push(value);
        }
        let replacement = if self.iterator_left {
            values.join(",")
        } else {
            format!("({})", values.join(","))
        };
        let semicolon =
            &self.source[declaration.span.start as usize..declaration.span.end as usize];
        self.edit(
            declaration.span,
            format!(
                "{replacement}{}",
                if semicolon.trim_end().ends_with(';') {
                    ";"
                } else {
                    ""
                }
            ),
        );
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        let runtime = self.runtime;
        if let Some(identifier) = &class.id {
            if let Some((binding, _)) = self.target.get(&identifier.span.start) {
                // A named class expression retains its immutable inner name,
                // including the TDZ during heritage/computed-key evaluation.
                self.edits.retain(|edit| {
                    !self.facts.references.iter().any(|reference| {
                        reference.binding == Some(binding.id)
                            && reference.span == edit.span
                            && contains(class.span, reference.span)
                    })
                });
            }
        }
        walk::walk_class(self, class);
        let Some(identifier) = &class.id else { return };
        let Some((binding, _)) = self.target.get(&identifier.span.start) else {
            return;
        };
        let value = self.render(class.span).expect("checked class edits");
        let key = key(self.graph, self.file, &binding.name);
        self.edit(
            class.span,
            format!("{runtime}.cell({key}).i = ({value}); yield {{init:{key}}};"),
        );
    }

    fn visit_assignment_pattern(&mut self, pattern: &AssignmentPattern<'a>) {
        walk::walk_assignment_pattern(self, pattern);
        if let BindingPattern::BindingIdentifier(identifier) = &pattern.left {
            let value = self.named(&pattern.right, identifier.name.as_str());
            self.edit(pattern.right.span(), value);
        }
    }

    fn visit_assignment_expression(&mut self, expression: &AssignmentExpression<'a>) {
        walk::walk_assignment_expression(self, expression);
        if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &expression.left {
            let operator = self.source
                [identifier.span.end as usize..expression.right.span().start as usize]
                .trim();
            if ["=", "||=", "&&=", "??="].contains(&operator) {
                let value = self.named(&expression.right, identifier.name.as_str());
                self.edit(expression.right.span(), value);
            }
        }
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        property: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        walk::walk_assignment_target_property_identifier(self, property);
        if let Some(initializer) = &property.init {
            let value = self.named(initializer, property.binding.name.as_str());
            self.edit(initializer.span(), value);
        }
    }

    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.generator_depth += 1;
        self.this_depth += 1;
        walk::walk_function(self, function, flags);
        self.this_depth -= 1;
        self.generator_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.generator_depth += 1;
        walk::walk_arrow_function_expression(self, function);
        self.generator_depth -= 1;
    }

    fn visit_this_expression(&mut self, expression: &ThisExpression) {
        if self.this_depth == 0
            && !self
                .class_this
                .iter()
                .any(|&s| contains(s, expression.span))
        {
            self.edit(expression.span, "void 0");
        }
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        if self.captured_global(identifier.name.as_str()) {
            if let Some(reference) = self
                .facts
                .references
                .iter()
                .find(|r| r.span == identifier.span && r.binding.is_none())
            {
                let value = format!(
                    "{}.globalCell({}).v",
                    self.runtime,
                    quote(identifier.name.as_str())
                );
                let value = if reference.write {
                    value
                } else {
                    format!("(0,{value})")
                };
                self.edit(
                    identifier.span,
                    self.shorthand(identifier.span, identifier.name.as_str(), value),
                );
            }
        }
    }

    fn visit_unary_expression(&mut self, expression: &UnaryExpression<'a>) {
        walk::walk_unary_expression(self, expression);
        if expression.operator.as_str() == "typeof" {
            if let Expression::Identifier(identifier) = expression.argument.get_inner_expression() {
                if self.captured_global(identifier.name.as_str())
                    && self
                        .facts
                        .references
                        .iter()
                        .any(|r| r.span == identifier.span && r.binding.is_none())
                {
                    self.edit(
                        expression.span,
                        format!(
                            "{}.typeofGlobal({})",
                            self.runtime,
                            quote(identifier.name.as_str())
                        ),
                    );
                }
            }
        }
    }

    fn visit_await_expression(&mut self, expression: &AwaitExpression<'a>) {
        walk::walk_await_expression(self, expression);
        if self.generator_depth == 0 {
            let value = self
                .render(expression.argument.span())
                .expect("checked await edits");
            self.edit(expression.span, format!("(yield {{await:({value})}})"));
        }
    }

    fn visit_labeled_statement(&mut self, statement: &LabeledStatement<'a>) {
        let mut labels = vec![statement.label.name.to_string()];
        let mut body = &statement.body;
        while let Statement::LabeledStatement(inner) = body {
            labels.push(inner.label.name.to_string());
            body = &inner.body;
        }
        if let Statement::ForOfStatement(iteration) = body {
            if iteration.r#await && self.generator_depth == 0 {
                self.iterator_labels = labels;
                self.visit_for_of_statement(iteration);
                let lowered = self
                    .render(iteration.span)
                    .expect("checked labeled iterator");
                self.edit(statement.span, lowered);
                return;
            }
        }
        walk::walk_labeled_statement(self, statement);
    }

    fn visit_for_of_statement(&mut self, statement: &ForOfStatement<'a>) {
        let labels: String = std::mem::take(&mut self.iterator_labels)
            .into_iter()
            .map(|label| format!("{label}: "))
            .collect();
        let runtime = self.runtime;
        let previous = self.iterator_left;
        self.iterator_left = true;
        self.visit_for_statement_left(&statement.left);
        self.iterator_left = previous;
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
        if statement.r#await && self.generator_depth == 0 {
            let n = self.fresh;
            self.fresh += 1;
            let iterator = format!("{runtime}iterator{n}");
            let step = format!("{runtime}step{n}");
            let close_needed = format!("{runtime}close{n}");
            let throwing = format!("{runtime}throwing{n}");
            let close = format!("{runtime}return{n}");
            let value = format!("{runtime}value{n}");
            let error = format!("{runtime}error{n}");
            let left = self
                .render(statement.left.span())
                .expect("checked iterator left");
            let right = self
                .render(statement.right.span())
                .expect("checked iterator right");
            let body = self
                .render(statement.body.span())
                .expect("checked iterator body");
            self.edit(statement.span,format!(
                "{{const {iterator} = {runtime}.asyncIterator({right}); let {close_needed} = false, {throwing} = false;\ntry {{{labels}while (true) {{ {close_needed} = false;\nconst {step} = {runtime}.iteratorResult(yield {{await:{iterator}.next()}}); if ({step}.done) break;\nconst {value} = {step}.value; {close_needed} = true;\n{left} = {value}; {body}\n}} }} catch ({error}) {{ {throwing} = true; throw {error}; }} finally {{ if ({close_needed}) {{try {{const {close} = {iterator}.return; if ({close} != null) {runtime}.iteratorResult(yield {{await:{close}.call({iterator})}}); }} catch ({error}) {{if (!{throwing}) throw {error};}} }} }} }}"
            ));
        }
    }

    fn visit_for_in_statement(&mut self, statement: &ForInStatement<'a>) {
        let previous = self.iterator_left;
        self.iterator_left = true;
        self.visit_for_statement_left(&statement.left);
        self.iterator_left = previous;
        self.visit_expression(&statement.right);
        self.visit_statement(&statement.body);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        // Generated offsets differ after PURE/dead-part projection. Match each
        // surviving literal by its original dependency occurrence in source order.
        let Expression::StringLiteral(literal) = &expression.source else {
            return;
        };
        let (generated, index) = self
            .original_ranges
            .iter()
            .find(|(range, _)| contains(*range, expression.span))
            .unwrap();
        let original_segment = self.graph.file_segments(self.file).unwrap()[*index as usize].span;
        let before = self
            .facts
            .dependencies
            .iter()
            .filter(|d| {
                contains(*generated, d.span)
                    && d.kind == DependencyKind::DynamicImport
                    && d.specifier == literal.value.as_str()
            })
            .position(|d| d.span == expression.span)
            .unwrap();
        let original = self
            .graph
            .file_facts(self.file)
            .unwrap()
            .dependencies
            .iter()
            .filter(|d| {
                contains(original_segment, d.span)
                    && d.kind == DependencyKind::DynamicImport
                    && d.specifier == literal.value.as_str()
            })
            .nth(before)
            .unwrap();
        self.edit(
            expression.span,
            dynamic_import(self.graph, self.file, original),
        );
    }
}

#[derive(Default)]
struct Shorthand(BTreeSet<u32>, Vec<Span>);
impl<'a> Visit<'a> for Shorthand {
    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        if let Some(value) = &property.value {
            self.1.push(value.span());
        }
        walk::walk_property_definition(self, property);
    }
    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.1.push(block.span);
        walk::walk_static_block(self, block);
    }
    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if property.shorthand {
            self.0.insert(property.value.span().start);
        }
        walk::walk_object_property(self, property);
    }
    fn visit_binding_property(&mut self, property: &BindingProperty<'a>) {
        if property.shorthand {
            self.0.insert(property.value.span().start);
        }
        walk::walk_binding_property(self, property);
    }
    fn visit_assignment_target_property_identifier(
        &mut self,
        property: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.0.insert(property.binding.span.start);
        walk::walk_assignment_target_property_identifier(self, property);
    }
}
