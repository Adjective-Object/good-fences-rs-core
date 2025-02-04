use std::{fmt::Display, sync::Arc};

use napi::{
    threadsafe_function::{
        ErrorStrategy::{self},
        ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
    },
    JsFunction, JsObject, Result, Status,
};

use logger::Logger;

#[derive(Clone)]
pub struct ConsoleLogger {
    logfn: Arc<ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>>,
}

impl ConsoleLogger {
    pub fn new(console: JsObject) -> Result<Self> {
        let logfn = console.get_named_property::<JsFunction>("log")?;
        Ok(Self {
            logfn: Arc::new(logfn.create_threadsafe_function(
                // allow queueing console responses?
                100,
                |ctx: ThreadSafeCallContext<String>| {
                    let js_str = ctx.env.create_string(&ctx.value)?;
                    // return as an argv array
                    Ok(vec![js_str])
                },
            )?),
        })
    }
}

impl Logger for ConsoleLogger {
    fn log(&self, message: impl Display) {
        let message_string: String = message.to_string();
        let status = self
            .logfn
            .call(Ok(message_string), ThreadsafeFunctionCallMode::Blocking);
        match status {
            Status::Ok => {}
            _ => {
                eprintln!();
                panic!("Error calling console.log from Rust. Unexpected threadsafe function call mode {}", status);
            }
        }
    }
}
