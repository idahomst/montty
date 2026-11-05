//! Helpers shared by the integration tests (`mod common;`).
#![allow(dead_code)]

#[path = "../../src/testutil.rs"]
mod testutil;

pub use testutil::TempDir;
