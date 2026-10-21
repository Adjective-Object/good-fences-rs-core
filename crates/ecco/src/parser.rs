use eyre::{bail, ensure, eyre, Result};

use crate::views::{
    BindingView, CallArgumentView, DefinitionView, HazardExprView, LocalFactsView, ReturnsView,
};
use crate::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name(String),
    String(String),
    Symbol(char),
    Rest,
    Merge,
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    cursor: usize,
    length: usize,
}

pub fn parse_declarations(source: &str) -> Result<LocalFacts> {
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '/' if chars.peek().is_some_and(|(_, c)| *c == '/') => {
                for (_, c) in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '"' | '\'' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some((_, c)) = chars.next() {
                    if c == ch {
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        let (_, escaped) = chars
                            .next()
                            .ok_or_else(|| eyre!("byte {offset}: unterminated string escape"))?;
                        ensure!(
                            matches!(escaped, '\\' | '"' | '\''),
                            "byte {offset}: unsupported string escape"
                        );
                        value.push(escaped);
                    } else {
                        ensure!(c != '\n' && c != '\r', "byte {offset}: newline in string");
                        value.push(c);
                    }
                }
                ensure!(closed, "byte {offset}: unterminated string");
                tokens.push((offset, Token::String(value)));
            }
            '.' if source[offset..].starts_with("...") => {
                chars.next();
                chars.next();
                tokens.push((offset, Token::Rest));
            }
            '|' if chars.peek().is_some_and(|(_, c)| *c == '=') => {
                chars.next();
                tokens.push((offset, Token::Merge));
            }
            c if c.is_ascii_alphabetic() || matches!(c, '_' | '$') => {
                let mut name = String::from(c);
                while let Some((_, c)) = chars.peek() {
                    if !c.is_ascii_alphanumeric() && !matches!(c, '_' | '$') {
                        break;
                    }
                    name.push(*c);
                    chars.next();
                }
                tokens.push((offset, Token::Name(name)));
            }
            c if "{}().,=|*".contains(c) => tokens.push((offset, Token::Symbol(c))),
            _ => bail!("byte {offset}: unexpected character {ch:?}"),
        }
    }
    let mut parser = Parser {
        tokens,
        cursor: 0,
        length: source.len(),
    };
    let mut facts = LocalFacts::default();
    while parser.peek().is_some() {
        if parser.word("module") {
            let module = match parser.next()? {
                Token::String(value) => value,
                _ => return parser.error("expected quoted module specifier"),
            };
            parser.symbol('{')?;
            while !parser.take_symbol('}') {
                parser.keyword("export")?;
                parser.keyword("fn")?;
                let path = parser.path()?;
                let definition = parser.function(Binding::Module {
                    module: module.clone(),
                    path,
                })?;
                facts.definitions_mut().push(definition);
            }
        } else {
            let global = parser.word("global");
            if global && parser.word("symbol") {
                let name = parser.name()?;
                parser.symbol('{')?;
                parser.keyword("read_effects")?;
                parser.symbol('=')?;
                let read_effects = if parser.word("true") {
                    true
                } else {
                    parser.keyword("false")?;
                    false
                };
                parser.symbol('}')?;
                facts
                    .global_symbols_mut()
                    .push(GlobalSymbol::new(name, read_effects));
                continue;
            }
            parser.keyword("fn")?;
            let path = parser.path()?;
            ensure!(
                global || path.len() == 1,
                "local function names cannot be dotted"
            );
            let binding = if global {
                Binding::Global(path)
            } else {
                Binding::Local(path[0].clone())
            };
            facts.definitions_mut().push(parser.function(binding)?);
        }
    }
    validate_facts(&facts)?;
    Ok(facts)
}

pub(crate) fn validate_facts(facts: &LocalFacts) -> Result<()> {
    validate_facts_view(facts.into())
}

pub(crate) fn validate_facts_view(facts: LocalFactsView<'_>) -> Result<()> {
    for (index, symbol) in facts.global_symbols().iter().enumerate() {
        ensure!(!symbol.name().is_empty(), "empty global symbol name");
        ensure!(
            !facts
                .global_symbols()
                .iter()
                .take(index)
                .any(|previous| previous.name() == symbol.name()),
            "duplicate global symbol: {}",
            symbol.name()
        );
    }
    let mut bindings = std::collections::HashSet::new();
    for definition in facts.definitions() {
        ensure!(
            bindings.insert(definition.binding()),
            "duplicate definition: {:?}",
            definition.binding()
        );
        validate_definition_view(definition)?;
    }
    Ok(())
}

fn validate_definition(definition: &Definition) -> Result<()> {
    validate_definition_view(definition.into())
}

fn validate_definition_view(definition: DefinitionView<'_>) -> Result<()> {
    match definition.binding() {
        BindingView::Global(path) | BindingView::Module { path, .. } => {
            ensure!(
                !path.is_empty() && path.iter().all(|name| !name.is_empty()),
                "empty definition path"
            );
        }
        BindingView::Local(name) => ensure!(!name.is_empty(), "empty local definition name"),
    }
    for (index, parameter) in definition.parameters().iter().enumerate() {
        ensure!(
            !parameter.is_empty()
                && parameter != "this"
                && !definition
                    .parameters()
                    .iter()
                    .take(index)
                    .any(|v| v == parameter),
            "duplicate or reserved parameter {parameter}"
        );
    }
    if let Some(rest) = definition.rest() {
        ensure!(
            definition.variadic()
                && !rest.is_empty()
                && rest != "this"
                && !definition
                    .parameters()
                    .iter()
                    .any(|parameter| parameter == rest),
            "invalid rest parameter {rest}"
        );
    }
    for fields in [definition.hazards(), definition.return_hazards()] {
        for (index, (slot, _)) in fields.fields().iter().enumerate() {
            ensure!(
                !fields
                    .fields()
                    .iter()
                    .take(index)
                    .any(|(previous, _)| previous == slot),
                "duplicate hazard field {}",
                slot.name()
            );
        }
    }
    let scalar = |name: &str| name == "this" || definition.parameters().iter().any(|p| p == name);
    fn validate(
        expression: HazardExprView<'_>,
        definition: DefinitionView<'_>,
        scalar: &impl Fn(&str) -> bool,
    ) -> Result<()> {
        match expression {
            HazardExprView::Bool(_) | HazardExprView::Unknown => {}
            HazardExprView::Read { value, .. } => ensure!(scalar(value), "unknown value {value}"),
            HazardExprView::Or(expressions) => {
                for expression in expressions {
                    validate(expression, definition, scalar)?;
                }
            }
            HazardExprView::Call {
                function,
                arguments,
                receiver,
            } => {
                ensure!(scalar(function), "unknown callable {function}");
                if let Some(receiver) = receiver {
                    ensure!(scalar(receiver), "unknown receiver {receiver}");
                }
                for argument in arguments {
                    match argument {
                        CallArgumentView::Value(name) => {
                            ensure!(scalar(name), "unknown argument {name}")
                        }
                        CallArgumentView::Spread(name) => ensure!(
                            definition.rest() == Some(name),
                            "unknown argument list {name}"
                        ),
                        CallArgumentView::Forward => {}
                    }
                }
            }
        }
        Ok(())
    }
    ensure!(
        matches!(definition.returns(), ReturnsView::Fresh)
            || (definition.return_hazards().default_expression().is_none()
                && definition.return_hazards().fields().is_empty()),
        "return hazard assignments require an implicit fresh return"
    );
    for expression in definition
        .hazards()
        .fields()
        .iter()
        .chain(definition.return_hazards().fields())
        .map(|(_, e)| e)
        .chain(definition.hazards().default_expression())
        .chain(definition.return_hazards().default_expression())
        .chain(definition.updates().iter().map(|u| u.expression()))
    {
        validate(expression, definition, &scalar)?;
    }
    for update in definition.updates() {
        ensure!(
            scalar(update.target()),
            "unknown update target {}",
            update.target()
        );
    }
    if let ReturnsView::Argument(name) = definition.returns() {
        ensure!(scalar(name), "unknown return value {name}");
    }
    Ok(())
}

impl Parser {
    fn error<T>(&self, message: &str) -> Result<T> {
        bail!(
            "byte {}: {message}",
            self.tokens
                .get(self.cursor)
                .map_or(self.length, |(offset, _)| *offset)
        )
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor).map(|(_, token)| token)
    }

    fn next(&mut self) -> Result<Token> {
        let token = self
            .peek()
            .cloned()
            .ok_or_else(|| eyre!("byte {}: unexpected end of declaration", self.length))?;
        self.cursor += 1;
        Ok(token)
    }

    fn word(&mut self, expected: &str) -> bool {
        if matches!(self.peek(), Some(Token::Name(name)) if name == expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn keyword(&mut self, expected: &str) -> Result<()> {
        if self.word(expected) {
            Ok(())
        } else {
            self.error(&format!("expected {expected}"))
        }
    }

    fn take_symbol(&mut self, expected: char) -> bool {
        if self.peek() == Some(&Token::Symbol(expected)) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn symbol(&mut self, expected: char) -> Result<()> {
        if self.take_symbol(expected) {
            Ok(())
        } else {
            self.error(&format!("expected {expected}"))
        }
    }

    fn name(&mut self) -> Result<String> {
        match self.next()? {
            Token::Name(name) => Ok(name),
            _ => self.error("expected identifier"),
        }
    }

    fn path(&mut self) -> Result<Vec<String>> {
        let mut path = vec![self.name()?];
        while self.take_symbol('.') {
            path.push(self.name()?);
        }
        Ok(path)
    }

    fn slot(&mut self) -> Result<Option<Hazard>> {
        if self.take_symbol('*') {
            self.keyword("_effects")?;
            Ok(None)
        } else {
            let name = self.name()?;
            Hazard::parse(&name)
                .map(Some)
                .ok_or_else(|| eyre!("unknown hazard slot {name:?}"))
        }
    }

    fn fields(&mut self, fields: &mut HazardFields) -> Result<()> {
        let slot = self.slot()?;
        self.symbol('=')?;
        let expression = self.expression()?;
        if let Some(slot) = slot {
            ensure!(
                !fields
                    .fields()
                    .iter()
                    .any(|(previous, _)| *previous == slot),
                "duplicate hazard field {}",
                slot.name()
            );
            fields.fields_mut().push((slot, expression));
        } else {
            ensure!(
                fields.default_expression().is_none(),
                "duplicate wildcard default"
            );
            *fields.default_mut() = Some(expression);
        }
        Ok(())
    }

    fn function(&mut self, binding: Binding) -> Result<Definition> {
        let mut definition = Definition::new(
            binding,
            Vec::new(),
            None,
            false,
            HazardFields::default(),
            Returns::Fresh,
            HazardFields::default(),
            Vec::new(),
        );
        self.symbol('(')?;
        while !self.take_symbol(')') {
            if self.peek() == Some(&Token::Rest) {
                self.next()?;
                *definition.variadic_mut() = true;
                if matches!(self.peek(), Some(Token::Name(_))) {
                    *definition.rest_mut() = Some(self.name()?);
                }
                self.symbol(')')?;
                break;
            }
            let parameter = self.name()?;
            ensure!(
                parameter != "this" && !definition.parameters().contains(&parameter),
                "duplicate or reserved parameter {parameter}"
            );
            definition.parameters_mut().push(parameter);
            if !self.take_symbol(',') {
                self.symbol(')')?;
                break;
            }
        }
        if let Some(rest) = definition.rest() {
            ensure!(
                rest != "this"
                    && !definition
                        .parameters()
                        .iter()
                        .any(|parameter| parameter == rest),
                "duplicate or reserved rest parameter {rest}"
            );
        }
        self.symbol('{')?;
        let mut return_mode = false;
        while !self.take_symbol('}') {
            if self.word("returns") {
                if self.take_symbol('.') {
                    self.fields(definition.return_hazards_mut())?;
                } else {
                    ensure!(!return_mode, "duplicate return identity");
                    return_mode = true;
                    *definition.returns_mut() = if self.word("void") {
                        Returns::Void
                    } else if self.word("unknown") {
                        Returns::Unknown
                    } else {
                        self.symbol('=')?;
                        Returns::Argument(self.name()?)
                    };
                }
            } else if self.word("updates") {
                let target = self.name()?;
                self.symbol('.')?;
                let slot = self.slot()?;
                ensure!(self.next()? == Token::Merge, "updates require |=");
                let expression = self.expression()?;
                definition
                    .updates_mut()
                    .push(Update::new(target, slot, expression));
            } else {
                self.fields(definition.hazards_mut())?;
            }
        }
        ensure!(
            matches!(definition.returns(), Returns::Fresh)
                || (definition.return_hazards().default_expression().is_none()
                    && definition.return_hazards().fields().is_empty()),
            "return hazard assignments require an implicit fresh return"
        );
        validate_definition(&definition)?;
        Ok(definition)
    }

    fn expression(&mut self) -> Result<HazardExpr> {
        let mut expressions = vec![self.atom()?];
        while self.take_symbol('|') {
            expressions.push(self.atom()?);
        }
        Ok(if expressions.len() == 1 {
            expressions.remove(0)
        } else {
            HazardExpr::Or(expressions)
        })
    }

    fn atom(&mut self) -> Result<HazardExpr> {
        if self.word("true") {
            return Ok(HazardExpr::Bool(true));
        }
        if self.word("false") {
            return Ok(HazardExpr::Bool(false));
        }
        if matches!(self.peek(), Some(Token::Name(name)) if name == "unknown")
            && self.tokens.get(self.cursor + 1).map(|(_, token)| token) == Some(&Token::Symbol('('))
        {
            self.next()?;
            self.symbol('(')?;
            self.symbol(')')?;
            return Ok(HazardExpr::Unknown);
        }
        if self.take_symbol('(') {
            let expression = self.expression()?;
            self.symbol(')')?;
            return Ok(expression);
        }
        if matches!(self.peek(), Some(Token::Name(name)) if name == "call")
            && self.tokens.get(self.cursor + 1).map(|(_, token)| token) == Some(&Token::Symbol('('))
        {
            self.next()?;
            self.symbol('(')?;
            let function = self.name()?;
            let mut arguments = Vec::new();
            let mut receiver = None;
            while self.take_symbol(',') {
                if self.word("this") {
                    self.symbol('=')?;
                    ensure!(receiver.is_none(), "duplicate receiver");
                    receiver = Some(self.name()?);
                    self.symbol(')')?;
                    return Ok(HazardExpr::Call {
                        function,
                        arguments,
                        receiver,
                    });
                } else if self.peek() == Some(&Token::Rest) {
                    self.next()?;
                    arguments.push(if matches!(self.peek(), Some(Token::Name(_))) {
                        CallArgument::Spread(self.name()?)
                    } else {
                        CallArgument::Forward
                    });
                } else {
                    arguments.push(CallArgument::Value(self.name()?));
                }
            }
            self.symbol(')')?;
            return Ok(HazardExpr::Call {
                function,
                arguments,
                receiver,
            });
        }
        let value = self.name()?;
        self.symbol('.')?;
        let slot = self
            .slot()?
            .ok_or_else(|| eyre!("wildcard reads are not supported"))?;
        Ok(HazardExpr::Read { value, slot })
    }
}
