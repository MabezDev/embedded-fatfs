//! Block device adapters

#![cfg_attr(not(test), no_std)]

// MUST be the first module listed
mod fmt;

mod stream_slice;
mod buf_stream;

pub use stream_slice::{StreamSlice, StreamSliceError};
pub use buf_stream::{BufStream, BufStreamError};
