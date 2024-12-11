use anyhow::Error;
use std::fmt::Display;

#[derive(Debug, Copy, Clone)]
pub enum Status {
    // ok
    Ok,
    InvalidArg,
    ObjectExpected,
    StringExpected,
    NameExpected,
    FunctionExpected,
    NumberExpected,
    BooleanExpected,
    ArrayExpected,
    GenericFailure,
    PendingException,
    Cancelled,
    EscapeCalledTwice,
    HandleScopeMismatch,
    CallbackScopeMismatch,
    QueueFull,
    Closing,
    BigintExpected,
    DateExpected,
    ArrayBufferExpected,
    DetachableArraybufferExpected,
    WouldDeadlock,
    NoExternalBuffersAllowed,
    Unknown,
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match &self {
                Status::Ok => "ok",
                Status::InvalidArg => "invalid_arg",
                Status::ObjectExpected => "object_expected",
                Status::StringExpected => "string_expected",
                Status::NameExpected => "name_expected",
                Status::FunctionExpected => "function_expected",
                Status::NumberExpected => "number_expected",
                Status::BooleanExpected => "boolean_expected",
                Status::ArrayExpected => "array_expected",
                Status::GenericFailure => "generic_failure",
                Status::PendingException => "pending_exception",
                Status::Cancelled => "cancelled",
                Status::EscapeCalledTwice => "escape_called_twice",
                Status::HandleScopeMismatch => "handle_scope_mismatch",
                Status::CallbackScopeMismatch => "callback_scope_mismatch",
                Status::QueueFull => "queue_full",
                Status::Closing => "closing",
                Status::BigintExpected => "bigint_expected",
                Status::DateExpected => "date_expected",
                Status::ArrayBufferExpected => "array_buffer_expected",
                Status::DetachableArraybufferExpected => "detachable_arraybuffer_expected",
                Status::WouldDeadlock => "would_deadlock",
                Status::NoExternalBuffersAllowed => "no_external_buffers_allowed",
                Status::Unknown => "unknown",
            }
        )?;
        Ok(())
    }
}

// equivalent to napi::Error, but declared separately so
// it can be used in tested modules
//
// Test modules can't reference napi::Error directly, since
// that would lead to a reference to `napi_delete_reference`,
// which only exists when the library is linked with node.
#[derive(Debug)]
pub struct JsErr {
    status: Status,
    err: Error,
}

impl JsErr {
    pub fn new(status: Status, err: Error) -> Self {
        if err.is::<JsErr>() {
            // unwrap is safe because we know the error is a JsErr
            //
            // We do this as a weird hoop jump with if { cast } instead of
            // if let Ok(downcasted) = .. because .downcast takes ownership
            // of the error, but we need it in the `else` branch.
            let js_err = err.downcast::<JsErr>().unwrap();
            Self {
                status: js_err.status,
                err: js_err.err,
            }
        } else {
            Self { status, err }
        }
    }
    pub fn ok(err: impl Into<Error>) -> Self {
        Self::new(Status::Ok, err.into())
    }
    pub fn invalid_arg(err: impl Into<Error>) -> Self {
        Self::new(Status::InvalidArg, err.into())
    }
    pub fn object_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::ObjectExpected, err.into())
    }
    pub fn string_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::StringExpected, err.into())
    }
    pub fn name_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::NameExpected, err.into())
    }
    pub fn function_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::FunctionExpected, err.into())
    }
    pub fn number_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::NumberExpected, err.into())
    }
    pub fn boolean_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::BooleanExpected, err.into())
    }
    pub fn array_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::ArrayExpected, err.into())
    }
    pub fn generic_failure(err: impl Into<Error>) -> Self {
        Self::new(Status::GenericFailure, err.into())
    }
    pub fn pending_exception(err: impl Into<Error>) -> Self {
        Self::new(Status::PendingException, err.into())
    }
    pub fn cancelled(err: impl Into<Error>) -> Self {
        Self::new(Status::Cancelled, err.into())
    }
    pub fn escape_called_twice(err: impl Into<Error>) -> Self {
        Self::new(Status::EscapeCalledTwice, err.into())
    }
    pub fn handle_scope_mismatch(err: impl Into<Error>) -> Self {
        Self::new(Status::HandleScopeMismatch, err.into())
    }
    pub fn callback_scope_mismatch(err: impl Into<Error>) -> Self {
        Self::new(Status::CallbackScopeMismatch, err.into())
    }
    pub fn queue_full(err: impl Into<Error>) -> Self {
        Self::new(Status::QueueFull, err.into())
    }
    pub fn closing(err: impl Into<Error>) -> Self {
        Self::new(Status::Closing, err.into())
    }
    pub fn bigint_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::BigintExpected, err.into())
    }
    pub fn date_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::DateExpected, err.into())
    }
    pub fn array_buffer_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::ArrayBufferExpected, err.into())
    }
    pub fn detachable_arraybuffer_expected(err: impl Into<Error>) -> Self {
        Self::new(Status::DetachableArraybufferExpected, err.into())
    }
    pub fn would_deadlock(err: impl Into<Error>) -> Self {
        Self::new(Status::WouldDeadlock, err.into())
    }
    pub fn no_external_buffers_allowed(err: impl Into<Error>) -> Self {
        Self::new(Status::NoExternalBuffersAllowed, err.into())
    }
    pub fn unknown(err: impl Into<Error>) -> Self {
        Self::new(Status::Unknown, err.into())
    }

    pub fn message(&self) -> String {
        if let Some(anyhow_err) = self.err.downcast_ref::<anyhow::Error>() {
            format!("{:#}", anyhow_err)
        } else {
            format!("{}", self.err)
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }
}

impl Display for JsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status: {}. {}", self.status, self.err)
    }
}

impl std::error::Error for JsErr {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.err.source()
    }
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.err.source()
    }
}
