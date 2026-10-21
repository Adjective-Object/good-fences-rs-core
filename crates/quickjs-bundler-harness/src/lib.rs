#![doc = include_str!("../README.md")]

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
};

use eyre::{Result, bail, eyre};
use rquickjs::{
    Coerced, Context, Ctx, Exception, FromJs, Function, Module, Persistent, Runtime, Value,
    loader::{ImportAttributes, Loader, Resolver},
};

/// Owned observations, with no guest handles or runtime dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRunResult {
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub name: String,
    pub arguments: Vec<serde_json::Value>,
}

/// Finite per-run budgets. Interrupt ticks are QuickJS polls, not exact JS instructions.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub memory_bytes: usize,
    pub stack_bytes: usize,
    pub interrupt_ticks: usize,
    pub jobs: usize,
    pub effects: usize,
    pub values: usize,
    pub recorded_bytes: usize,
    pub source_bytes: usize,
    pub modules: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 32 * 1024 * 1024,
            stack_bytes: 512 * 1024,
            interrupt_ticks: 10_000,
            jobs: 10_000,
            effects: 10_000,
            values: 100_000,
            recorded_bytes: 8 * 1024 * 1024,
            source_bytes: 8 * 1024 * 1024,
            modules: 1024,
        }
    }
}

/// Evaluate one example in a fresh runtime, using only the supplied modules and `harness`.
pub fn run(
    modules: &BTreeMap<PathBuf, String>,
    entry: impl AsRef<Path>,
) -> Result<HarnessRunResult> {
    run_with_limits(modules, entry, Limits::default())
}

pub fn run_with_limits(
    modules: &BTreeMap<PathBuf, String>,
    entry: impl AsRef<Path>,
    limits: Limits,
) -> Result<HarnessRunResult> {
    let sources = Rc::new(validate_modules(modules, limits)?);
    let entry = normalize_path(path_text(entry.as_ref())?)?;
    if !sources.contains_key(&entry) {
        bail!("missing entry module: {entry}");
    }
    if limits.memory_bytes == 0 || limits.stack_bytes == 0 {
        bail!("memory and stack limits must be nonzero");
    }
    let runtime = Runtime::new().map_err(|e| eyre!("create QuickJS runtime: {e}"))?;
    runtime.set_memory_limit(limits.memory_bytes);
    runtime.set_max_stack_size(limits.stack_bytes);
    let exhausted = Rc::new(RefCell::new(None::<String>));
    let interrupt_exhausted = exhausted.clone();
    let mut ticks = limits.interrupt_ticks;
    runtime.set_interrupt_handler(Some(Box::new(move || {
        if ticks == 0 {
            *interrupt_exhausted.borrow_mut() =
                Some("instruction/interrupt budget exceeded".into());
            true
        } else {
            ticks -= 1;
            false
        }
    })));
    runtime.set_loader(
        MemoryResolver(sources.clone()),
        MemoryLoader(sources.clone()),
    );
    let context = Context::full(&runtime).map_err(|e| eyre!("create QuickJS context: {e}"))?;
    let recorder = Rc::new(RefCell::new(Recorder {
        effects: Vec::new(),
        values: 0,
        bytes: 0,
        limits,
        exhausted: exhausted.clone(),
    }));
    let diagnostics = Diagnostics(RefCell::new(None));
    let rejections = Rc::new(RefCell::new(HashMap::new()));
    let pending_rejections = rejections.clone();
    runtime.set_host_promise_rejection_tracker(Some(Box::new(
        move |ctx, promise, reason, handled| {
            let promise = Persistent::save(&ctx, promise);
            if handled {
                pending_rejections.borrow_mut().remove(&promise);
            } else {
                pending_rejections
                    .borrow_mut()
                    .insert(promise, Persistent::save(&ctx, reason));
            }
        },
    )));
    let result = (|| {
        let promise = context.with(|ctx| -> Result<_> {
            let describe_error: Function = ctx
                .eval(include_str!("describe-error.js"))
                .map_err(|e| diagnostics.error(&ctx, e))?;
            *diagnostics.0.borrow_mut() = Some(Persistent::save(&ctx, describe_error));
            let record = recorder_function(ctx.clone(), recorder.clone())
                .map_err(|e| diagnostics.error(&ctx, e))?;
            ctx.globals()
                .set("__quickjs_harness_record", record)
                .map_err(|e| diagnostics.error(&ctx, e))?;
            let builtin = Module::declare(ctx.clone(), "harness", include_str!("harness.js"))
                .and_then(Module::eval)
                .map_err(|e| diagnostics.error(&ctx, e))?;
            builtin
                .1
                .result::<()>()
                .ok_or_else(|| eyre!("built-in harness did not settle"))?
                .map_err(|e| diagnostics.error(&ctx, e))?;
            let promise = Module::declare(ctx.clone(), entry.as_str(), sources[&entry].as_str())
                .and_then(Module::eval)
                .map_err(|e| diagnostics.error(&ctx, e))?
                .1;
            Ok(Persistent::save(&ctx, promise))
        })?;
        let mut jobs = 0;
        while runtime.is_job_pending() {
            if jobs == limits.jobs {
                bail!("pending-job budget exceeded");
            }
            jobs += 1;
            runtime.execute_pending_job().map_err(|e| {
                e.0.with(|ctx| diagnostics.error(&ctx, rquickjs::Error::Exception))
            })?;
        }
        if let Some(message) = exhausted.borrow().as_ref() {
            bail!("{message}");
        }
        context.with(|ctx| -> Result<()> {
            promise
                .restore(&ctx)
                .map_err(|e| diagnostics.error(&ctx, e))?
                .result::<()>()
                .ok_or_else(|| eyre!("entry evaluation is unsettled (no runnable jobs)"))?
                .map_err(|e| diagnostics.error(&ctx, e))?;
            if let Some(reason) = rejections.borrow().values().next() {
                let value = reason
                    .clone()
                    .restore(&ctx)
                    .map_err(|e| diagnostics.error(&ctx, e))?;
                bail!(
                    "unhandled promise rejection: {}",
                    diagnostics.describe(&ctx, value)
                );
            }
            Ok(())
        })
    })();
    // Promise bookkeeping and diagnostics must not outlive their runtime.
    runtime.set_host_promise_rejection_tracker(None);
    rejections.borrow_mut().clear();
    *diagnostics.0.borrow_mut() = None;
    let mut state = recorder.borrow_mut();
    result?;
    Ok(HarnessRunResult {
        effects: std::mem::take(&mut state.effects),
    })
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| eyre!("module paths must be UTF-8"))
}

fn normalize_path(path: &str) -> Result<String> {
    if path.starts_with('/') || path.contains(['\\', ':', '\0', '?', '#']) {
        bail!("invalid example-relative module path: {path:?}");
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!("module path escapes example root: {path:?}");
                }
            }
            component => components.push(component),
        }
    }
    if components.is_empty() {
        bail!("empty module path");
    }
    let normalized = components.join("/");
    if normalized == "harness" {
        bail!("the harness module name is reserved");
    }
    Ok(normalized)
}

fn validate_modules(
    modules: &BTreeMap<PathBuf, String>,
    limits: Limits,
) -> Result<BTreeMap<String, String>> {
    if modules.len() > limits.modules {
        bail!("module count budget exceeded");
    }
    let mut bytes = 0usize;
    let mut sources = BTreeMap::new();
    for (path, source) in modules {
        let path = path_text(path)?;
        bytes = bytes
            .checked_add(path.len())
            .and_then(|bytes| bytes.checked_add(source.len()))
            .ok_or_else(|| eyre!("source byte budget exceeded"))?;
        if bytes > limits.source_bytes {
            bail!("source byte budget exceeded");
        }
        let path = normalize_path(path)?;
        if sources.insert(path.clone(), source.clone()).is_some() {
            bail!("duplicate normalized module path: {path}");
        }
    }
    Ok(sources)
}

struct MemoryResolver(Rc<BTreeMap<String, String>>);

impl Resolver for MemoryResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if attributes.is_some() {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "import attributes are not supported",
            ));
        }
        if name == "harness" {
            return Ok(name.into());
        }
        let resolved = (|| {
            if !(name.starts_with("./") || name.starts_with("../")) {
                bail!("only relative imports and the built-in harness are allowed");
            }
            let parent = base.rsplit_once('/').map_or("", |(parent, _)| parent);
            let path = normalize_path(format!("{parent}/{name}").trim_start_matches('/'))?;
            if !self.0.contains_key(&path) {
                bail!("missing in-memory module: {path}");
            }
            Ok(path)
        })();
        resolved.map_err(|error: eyre::Report| {
            rquickjs::Error::new_resolving_message(base, name, error.to_string())
        })
    }
}

struct MemoryLoader(Rc<BTreeMap<String, String>>);

impl Loader for MemoryLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        let source = if name == "harness" {
            include_str!("harness.js")
        } else {
            self.0
                .get(name)
                .ok_or_else(|| rquickjs::Error::new_loading(name))?
        };
        Module::declare(ctx.clone(), name, source)
    }
}

struct Recorder {
    effects: Vec<Effect>,
    values: usize,
    bytes: usize,
    limits: Limits,
    exhausted: Rc<RefCell<Option<String>>>,
}

impl Recorder {
    fn limit_error(&self, ctx: &Ctx<'_>, resource: &str) -> rquickjs::Error {
        let message = format!("{resource} budget exceeded");
        *self.exhausted.borrow_mut() = Some(message.clone());
        Exception::throw_range(ctx, &message)
    }

    fn add_bytes(&mut self, ctx: &Ctx<'_>, bytes: usize) -> rquickjs::Result<()> {
        if bytes > self.limits.recorded_bytes.saturating_sub(self.bytes) {
            return Err(self.limit_error(ctx, "recorded byte"));
        }
        self.bytes += bytes;
        Ok(())
    }

    fn record<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: Value<'js>,
        serialized: Value<'js>,
    ) -> rquickjs::Result<()> {
        if self.effects.len() >= self.limits.effects {
            return Err(self.limit_error(ctx, "effect"));
        }
        let parsed = (|| {
            let name = String::from_js(ctx, name)
                .map_err(|_| "effect name must be a valid Unicode string")?;
            self.add_bytes(ctx, name.len())
                .map_err(|_| "recorded byte budget exceeded")?;
            let serialized = Option::<String>::from_js(ctx, serialized)
                .map_err(|_| "serialized effect arguments must be a valid Unicode string")?
                .ok_or("unsupported or cyclic effect argument")?;
            self.add_bytes(ctx, serialized.len())
                .map_err(|_| "recorded byte budget exceeded")?;
            let arguments: Vec<serde_json::Value> = serde_json::from_str(&serialized)
                .map_err(|_| "invalid serialized effect arguments")?;
            Ok::<_, &str>((name, arguments))
        })();
        let (name, arguments) = match parsed {
            Ok(parsed) => parsed,
            Err(message) => {
                *self.exhausted.borrow_mut() = Some(message.into());
                return Err(Exception::throw_type(ctx, message));
            }
        };
        fn count(value: &serde_json::Value) -> usize {
            1 + match value {
                serde_json::Value::Array(values) => values.iter().map(count).sum(),
                serde_json::Value::Object(values) => values.values().map(count).sum(),
                _ => 0,
            }
        }
        let values: usize = arguments.iter().map(count).sum();
        if values > self.limits.values.saturating_sub(self.values) {
            return Err(self.limit_error(ctx, "recorded value"));
        }
        self.values += values;
        self.effects.push(Effect { name, arguments });
        Ok(())
    }
}

fn recorder_function<'js>(
    ctx: Ctx<'js>,
    state: Rc<RefCell<Recorder>>,
) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx,
        move |ctx: Ctx<'js>, name: Value<'js>, serialized: Value<'js>| {
            state.borrow_mut().record(&ctx, name, serialized)
        },
    )
}

struct Diagnostics(RefCell<Option<Persistent<Function<'static>>>>);

impl Diagnostics {
    fn error(&self, ctx: &Ctx<'_>, error: rquickjs::Error) -> eyre::Report {
        if matches!(error, rquickjs::Error::Exception) {
            eyre!("JavaScript exception: {}", self.describe(ctx, ctx.catch()))
        } else {
            eyre!("QuickJS: {error}")
        }
    }

    fn describe<'js>(&self, ctx: &Ctx<'js>, value: Value<'js>) -> String {
        if value.is_undefined() {
            "undefined".into()
        } else if value.is_null() {
            "null".into()
        } else if let Some(string) = value.as_string() {
            string
                .to_string()
                .unwrap_or_else(|_| "<non-UTF8 string>".into())
        } else if value.is_number() || value.is_bool() || value.is_big_int() {
            Coerced::<String>::from_js(ctx, value)
                .map(|value| value.0)
                .unwrap_or_else(|_| "<primitive>".into())
        } else {
            let fallback = format!("<thrown {}>", value.type_of().as_str());
            if value.is_error() && !value.is_proxy() {
                if let Some(helper) = self.0.borrow().as_ref() {
                    if let Ok(helper) = helper.clone().restore(ctx) {
                        if let Ok(message) = helper.call::<_, Option<String>>((value,)) {
                            return message.unwrap_or(fallback);
                        }
                        // A diagnostic allocation can fail after memory/interrupt exhaustion.
                        ctx.catch();
                    }
                }
            }
            fallback
        }
    }
}

#[cfg(test)]
mod tests;
