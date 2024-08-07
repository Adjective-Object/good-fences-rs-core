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
        Self{ status: napi::Status::Ok, message }
    }
    pub fn invalid_arg(message: String) -> Self {
        Self{ status: napi::Status::InvalidArg, message }
    }
    pub fn object_expected(message: String) -> Self {
        Self{ status: napi::Status::ObjectExpected, message }
    }
    pub fn string_expected(message: String) -> Self {
        Self{ status: napi::Status::StringExpected, message }
    }
    pub fn name_expected(message: String) -> Self {
        Self{ status: napi::Status::NameExpected, message }
    }
    pub fn function_expected(message: String) -> Self {
        Self{ status: napi::Status::FunctionExpected, message }
    }
    pub fn number_expected(message: String) -> Self {
        Self{ status: napi::Status::NumberExpected, message }
    }
    pub fn boolean_expected(message: String) -> Self {
        Self{ status: napi::Status::BooleanExpected, message }
    }
    pub fn array_expected(message: String) -> Self {
        Self{ status: napi::Status::ArrayExpected, message }
    }
    pub fn generic_failure(message: String) -> Self {
        Self{ status: napi::Status::GenericFailure, message }
    }
    pub fn pending_exception(message: String) -> Self {
        Self{ status: napi::Status::PendingException, message }
    }
    pub fn cancelled(message: String) -> Self {
        Self{ status: napi::Status::Cancelled, message }
    }
    pub fn escape_called_twice(message: String) -> Self {
        Self{ status: napi::Status::EscapeCalledTwice, message }
    }
    pub fn handle_scope_mismatch(message: String) -> Self {
        Self{ status: napi::Status::HandleScopeMismatch, message }
    }
    pub fn callback_scope_mismatch(message: String) -> Self {
        Self{ status: napi::Status::CallbackScopeMismatch, message }
    }
    pub fn queue_full(message: String) -> Self {
        Self{ status: napi::Status::QueueFull, message }
    }
    pub fn closing(message: String) -> Self {
        Self{ status: napi::Status::Closing, message }
    }
    pub fn bigint_expected(message: String) -> Self {
        Self{ status: napi::Status::BigintExpected, message }
    }
    pub fn date_expected(message: String) -> Self {
        Self{ status: napi::Status::DateExpected, message }
    }
    pub fn array_buffer_expected(message: String) -> Self {
        Self{ status: napi::Status::ArrayBufferExpected, message }
    }
    pub fn detachable_arraybuffer_expected(message: String) -> Self {
        Self{ status: napi::Status::DetachableArraybufferExpected, message }
    }
    pub fn would_deadlock(message: String) -> Self {
        Self{ status: napi::Status::WouldDeadlock, message }
    }
    pub fn no_external_buffers_allowed(message: String) -> Self {
        Self{ status: napi::Status::NoExternalBuffersAllowed, message }
    }
    pub fn unknown(message: String) -> Self {
        Self{ status: napi::Status::Unknown, message }
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
