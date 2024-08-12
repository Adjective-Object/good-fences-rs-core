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
    message: String,
}

impl JsErr {
    pub fn new(status: napi::Status, message: String) -> Self {
        Self { status, message }
    }
    pub fn ok(message: String) -> Self {
        Self {
            status: napi::Status::Ok,
            message: message.to_string(),
        }
    }
    pub fn invalid_arg<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::InvalidArg,
            message: message.to_string(),
        }
    }
    pub fn object_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::ObjectExpected,
            message: message.to_string(),
        }
    }
    pub fn string_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::StringExpected,
            message: message.to_string(),
        }
    }
    pub fn name_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::NameExpected,
            message: message.to_string(),
        }
    }
    pub fn function_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::FunctionExpected,
            message: message.to_string(),
        }
    }
    pub fn number_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::NumberExpected,
            message: message.to_string(),
        }
    }
    pub fn boolean_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::BooleanExpected,
            message: message.to_string(),
        }
    }
    pub fn array_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::ArrayExpected,
            message: message.to_string(),
        }
    }
    pub fn generic_failure<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::GenericFailure,
            message: message.to_string(),
        }
    }
    pub fn pending_exception<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::PendingException,
            message: message.to_string(),
        }
    }
    pub fn cancelled<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::Cancelled,
            message: message.to_string(),
        }
    }
    pub fn escape_called_twice<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::EscapeCalledTwice,
            message: message.to_string(),
        }
    }
    pub fn handle_scope_mismatch<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::HandleScopeMismatch,
            message: message.to_string(),
        }
    }
    pub fn callback_scope_mismatch<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::CallbackScopeMismatch,
            message: message.to_string(),
        }
    }
    pub fn queue_full<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::QueueFull,
            message: message.to_string(),
        }
    }
    pub fn closing<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::Closing,
            message: message.to_string(),
        }
    }
    pub fn bigint_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::BigintExpected,
            message: message.to_string(),
        }
    }
    pub fn date_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::DateExpected,
            message: message.to_string(),
        }
    }
    pub fn array_buffer_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::ArrayBufferExpected,
            message: message.to_string(),
        }
    }
    pub fn detachable_arraybuffer_expected<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::DetachableArraybufferExpected,
            message: message.to_string(),
        }
    }
    pub fn would_deadlock<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::WouldDeadlock,
            message: message.to_string(),
        }
    }
    pub fn no_external_buffers_allowed<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::NoExternalBuffersAllowed,
            message: message.to_string(),
        }
    }
    pub fn unknown<P: ToString>(message: P) -> Self {
        Self {
            status: napi::Status::Unknown,
            message: message.to_string(),
        }
    }

    pub fn message<'a>(self: &'a Self) -> &'a str {
        &self.message
    }
    pub fn to_napi(self) -> napi::Error {
        napi::Error::new(self.status, self.message)
    }
}

impl AsRef<str> for JsErr {
    fn as_ref(&self) -> &str {
        &self.message
    }
}

impl Display for JsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status: {}. {}", self.status, self.message)
    }
}

impl Into<napi::Error> for JsErr {
    fn into(self) -> napi::Error {
        napi::Error::new(self.status, self.message)
    }
}
