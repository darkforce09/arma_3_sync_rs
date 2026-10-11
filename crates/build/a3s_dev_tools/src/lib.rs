//! Developer tooling around the Java fixture harness: fixture regeneration and Java cross-checks.
//!
//! The golden fixtures in `tests/fixtures/` are produced by the Java implementation through the
//! Java fixture harness (`tools/java_fixture_harness`). This crate wraps it:
//!
//! * [`java_harness`]: build the harness, generate fixtures, and cross-check files written by the
//!   Rust implementation with Arma3Sync's own Java readers ([`Harness::verify`],
//!   [`Harness::dump`], [`Harness::roundtrip`]). Usable as a dev-dependency by any crate.
//! * [`fixtures`]: regenerate, install into `tests/fixtures`, and compare fixture trees.
//!
//! The `a3s_dev_tools` binary exposes fixture regeneration (`regen-fixtures`) and Java
//! verification (`java-verify`). The harness needs JDK 25 (`scripts/java_env.sh`) and runs on Unix only.

pub mod fixtures;
pub mod java_harness;

pub use fixtures::{GENERATED_ENTRIES, TreeDiff};
pub use java_harness::{DumpMode, Harness, HarnessError, Part, VerifyLine, VerifyReport};
