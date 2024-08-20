use anyhow::Error;
use std::fmt::Display;

// equivalent to napi::Error, but declared separately so
// it can be used in tested modules
//
// Test modules can't reference napi::Error directly, since
// that would lead to a reference to `napi_delete_reference`,
// which only exists when the library is linked with node.
#[derive(Debug)]
pub struct JsErr {
    status: napi::Status,
    err: Error,
}

impl JsErr {
    pub fn new(status: napi::Status, err: Error) -> Self {
        Self { status, err }
    }
    pub fn ok(err: Error) -> Self {
        Self {
            status: napi::Status::Ok,
            err: err.into(),
        }
    }
    pub fn invalid_arg<P: Into<Error>>(err: P) -> Self {
        Self {
            status: napi::Status::InvalidArg,
            err: err.into(),
        }
    }
    pub fn object_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::ObjectExpected,
            err: err.into(),
        }
    }
    pub fn string_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::StringExpected,
            err: err.into(),
        }
    }
    pub fn name_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::NameExpected,
            err: err.into(),
        }
    }
    pub fn function_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::FunctionExpected,
            err: err.into(),
        }
    }
    pub fn number_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::NumberExpected,
            err: err.into(),
        }
    }
    pub fn boolean_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::BooleanExpected,
            err: err.into(),
        }
    }
    pub fn array_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::ArrayExpected,
            err: err.into(),
        }
    }
    pub fn generic_failure<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::GenericFailure,
            err: err.into(),
        }
    }
    pub fn pending_exception<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::PendingException,
            err: err.into(),
        }
    }
    pub fn cancelled<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::Cancelled,
            err: err.into(),
        }
    }
    pub fn escape_called_twice<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::EscapeCalledTwice,
            err: err.into(),
        }
    }
    pub fn handle_scope_mismatch<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::HandleScopeMismatch,
            err: err.into(),
        }
    }
    pub fn callback_scope_mismatch<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::CallbackScopeMismatch,
            err: err.into(),
        }
    }
    pub fn queue_full<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::QueueFull,
            err: err.into(),
        }
    }
    pub fn closing<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::Closing,
            err: err.into(),
        }
    }
    pub fn bigint_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::BigintExpected,
            err: err.into(),
        }
    }
    pub fn date_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::DateExpected,
            err: err.into(),
        }
    }
    pub fn array_buffer_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::ArrayBufferExpected,
            err: err.into(),
        }
    }
    pub fn detachable_arraybuffer_expected<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::DetachableArraybufferExpected,
            err: err.into(),
        }
    }
    pub fn would_deadlock<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::WouldDeadlock,
            err: err.into(),
        }
    }
    pub fn no_external_buffers_allowed<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::NoExternalBuffersAllowed,
            err: err.into(),
        }
    }
    pub fn unknown<P: Into<anyhow::Error>>(err: P) -> Self {
        Self {
            status: napi::Status::Unknown,
            err: err.into(),
        }
    }

    pub fn message(&self) -> String {
        return format!("{}", self.err);
    }
    pub fn to_napi(self) -> napi::Error {
        napi::Error::new(self.status, format!("{}", self.err))
    }
}

impl Display for JsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status: {}. {}", self.status, self.err)
    }
}

impl Into<napi::Error> for JsErr {
    fn into(self) -> napi::Error {
        napi::Error::new(self.status, self.err)
    }
}
