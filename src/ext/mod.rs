//! Nova extensions: installed bundles, the JavaScript extension host and
//! the online Extension Library.

pub mod config;
pub mod host;
pub mod library;
pub mod manifest;
pub mod process;
pub mod protocol;

pub use host::ExtHost;
pub use manifest::Extension;
pub use protocol::{DocInfo, FromHost, HostEvent, ToHost};

#[cfg(test)]
mod tests;
