extern crate js_err;
extern crate napi;

use js_err::{JsErr, Status};

pub trait ToNapi<T> {
    fn into_napi(self) -> T;
}

impl ToNapi<napi::Status> for js_err::Status {
    fn into_napi(self) -> napi::Status {
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

impl ToNapi<napi::Error> for js_err::JsErr {
    fn into_napi(self) -> napi::Error {
        napi::Error::new(self.status().into_napi(), self.message())
    }
}

impl<T> ToNapi<napi::Result<T>> for Result<T, JsErr> {
    fn into_napi(self) -> napi::Result<T> {
        self.map_err(|err| err.into_napi())
    }
}

pub struct NapiJsErr(JsErr);
impl From<NapiJsErr> for napi::Error {
    fn from(val: NapiJsErr) -> Self {
        val.0.into_napi()
    }
}
