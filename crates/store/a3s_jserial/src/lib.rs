//! Reader and writer for the Java Object Serialization Stream Protocol.
//!
//! Arma3Sync stores all repository metadata (`.a3s/sync`, `.a3s/serverinfo`, …) as the output of
//! `java.io.ObjectOutputStream`. This crate implements that stream format generically:
//!
//! * [`Graph`] is an arena holding every handle-bearing item of a stream (class descriptors,
//!   objects, strings, enum constants, arrays). Items refer to each other by [`NodeId`], which
//!   makes shared references and cycles (e.g. a tree node's `parent`) explicit.
//! * [`read()`] parses a stream into a [`Graph`]; a node's [`NodeId`] equals its wire handle offset.
//! * [`write()`] serializes a [`Graph`] back. Reading and re-writing any supported stream
//!   reproduces it byte for byte.
//! * [`jdk`] knows the `writeObject` layouts of the JDK classes Arma3Sync uses
//!   (`ArrayList`, `HashMap`, `HashSet`, `Date`, `Boolean`, …), both for decoding and for
//!   building new graphs that Java can read.
//!
//! See the [specification](https://docs.oracle.com/en/java/javase/21/docs/specs/serialization/protocol.html).

mod constants;
mod error;
pub mod jdk;
mod model;
mod mutf8;
mod reader;
mod writer;

pub use constants::{flags, tc};
pub use error::{Error, Result};
pub use model::{
    Array, ArrayData, ClassData, ClassDesc, Content, EnumConst, FieldDesc, FieldType, Graph, Node,
    NodeId, Object, Value,
};
pub use reader::{
    JavaFilterLimits, JavaFilterStats, Limits, java_filter_stats, read, read_with_limits,
};
pub use writer::write;
