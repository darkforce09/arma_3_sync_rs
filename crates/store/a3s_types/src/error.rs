/// Errors converting between Java object graphs and domain types.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Serialization(#[from] a3s_jserial::Error),

    #[error("unexpected class {found} (expected {expected})")]
    UnexpectedClass {
        expected: &'static str,
        found: String,
    },

    #[error("unknown {kind} constant `{name}`")]
    UnknownEnumConstant { kind: &'static str, name: String },

    #[error("required value `{0}` is null")]
    NullValue(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
