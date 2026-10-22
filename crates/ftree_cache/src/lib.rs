// So we can parameterize the ContextDataCaches on the filename of the context file, constantly
#![allow(incomplete_features)]
#![feature(adt_const_params)]
#![feature(unsized_const_params)]
#![cfg_attr(windows, feature(windows_by_handle))]

pub mod context_data;
pub mod inputs;

#[cfg(test)]
mod context_data_tests;
