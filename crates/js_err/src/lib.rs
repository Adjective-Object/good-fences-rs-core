use anyhow::Error;
use std::{fmt::Display};

#[derive(Debug)]
pub enum Status {
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
        write!(f, "{}", match &self {
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
        })?;
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
        Self { status, err }
    }
    pub fn ok(err: Error) -> Self {
        Self {
            status: Status::Ok,
            err: err.into(),
        }
    }
    pub fn invalid_arg<P: Into<Error>>(err: P) -> Self {
        Self {
            status: Status::InvalidArg,
            err: err.into(),
        }
    }
    pub fn object_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::ObjectExpected,
            err: err.into(),
        }
    }
    pub fn string_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::StringExpected,
            err: err.into(),
        }
    }
    pub fn name_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::NameExpected,
            err: err.into(),
        }
    }
    pub fn function_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::FunctionExpected,
            err: err.into(),
        }
    }
    pub fn number_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::NumberExpected,
            err: err.into(),
        }
    }
    pub fn boolean_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::BooleanExpected,
            err: err.into(),
        }
    }
    pub fn array_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::ArrayExpected,
            err: err.into(),
        }
    }
    pub fn generic_failure<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::GenericFailure,
            err: err.into(),
        }
    }
    pub fn pending_exception<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::PendingException,
            err: err.into(),
        }
    }
    pub fn cancelled<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::Cancelled,
            err: err.into(),
        }
    }
    pub fn escape_called_twice<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::EscapeCalledTwice,
            err: err.into(),
        }
    }
    pub fn handle_scope_mismatch<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::HandleScopeMismatch,
            err: err.into(),
        }
    }
    pub fn callback_scope_mismatch<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::CallbackScopeMismatch,
            err: err.into(),
        }
    }
    pub fn queue_full<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::QueueFull,
            err: err.into(),
        }
    }
    pub fn closing<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::Closing,
            err: err.into(),
        }
    }
    pub fn bigint_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::BigintExpected,
            err: err.into(),
        }
    }
    pub fn date_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::DateExpected,
            err: err.into(),
        }
    }
    pub fn array_buffer_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::ArrayBufferExpected,
            err: err.into(),
        }
    }
    pub fn detachable_arraybuffer_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::DetachableArraybufferExpected,
            err: err.into(),
        }
    }
    pub fn would_deadlock<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::WouldDeadlock,
            err: err.into(),
        }
    }
    pub fn no_external_buffers_allowed<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::NoExternalBuffersAllowed,
            err: err.into(),
        }
    }
    pub fn unknown<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: Status::Unknown,
            err: err.into(),
        }
    }

    pub fn message(&self) -> String {
        return format!("{}", self.err);
    }

    #[cfg(feature = "napi")]
    pub fn to_napi(self) -> napi::Error {
        self.into()
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

#[cfg(feature = "napi")]
impl Into<napi::Error> for JsErr {
    fn into(self) -> napi::Error {
        napi::Error::new(self.status.into(), self.err)
    }
}


#[cfg(feature = "napi")]
impl Into<napi::Status> for Status {
    fn into(self) -> napi::Status {
        match self {
            Status::Ok => napi::Status::Ok,
            Status::InvalidArg => napi::Status::InvalidArg,
            Status::ObjectExpected => napi::Status::ObjectExpected,
            Status::StringExpected => napi::Status::StringExpected,
            Status::NameExpected => napi::Status::NameExpected,
            Status::FunctionExpected => napi::Status::FunctionExpected,
            Status::NumberExpected => napi::Status::NumberExpected,
            Status::BooleanExpected => napi::Status::BooleanExpected,
            Status::ArrayExpected => napi::Status::ArrayExpected,
            Status::GenericFailure => napi::Status::GenericFailure,
            Status::PendingException => napi::Status::PendingException,
            Status::Cancelled => napi::Status::Cancelled,
            Status::EscapeCalledTwice => napi::Status::EscapeCalledTwice,
            Status::HandleScopeMismatch => napi::Status::HandleScopeMismatch,
            Status::CallbackScopeMismatch => napi::Status::CallbackScopeMismatch,
            Status::QueueFull => napi::Status::QueueFull,
            Status::Closing => napi::Status::Closing,
            Status::BigintExpected => napi::Status::BigintExpected,
            Status::DateExpected => napi::Status::DateExpected,
            Status::ArrayBufferExpected => napi::Status::ArrayBufferExpected,
            Status::DetachableArraybufferExpected => napi::Status::DetachableArraybufferExpected,
            Status::WouldDeadlock => napi::Status::WouldDeadlock,
            Status::NoExternalBuffersAllowed => napi::Status::NoExternalBuffersAllowed,
            Status::Unknown => napi::Status::Unknown,
        }
    }
}
