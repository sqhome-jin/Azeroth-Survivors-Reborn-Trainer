//! macOS trainer backend used by the SwiftUI host through a narrow C ABI.
mod backend;
mod diagnostic_log;
mod ffi;
mod hunt;
mod process;
mod settings;
mod stat_fields;
mod stats_ui;
mod update;

pub use backend::Backend;
pub use ffi::*;
pub use update::{check as check_update, version as parse_version};
