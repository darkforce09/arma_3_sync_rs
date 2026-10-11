# a3s_jserial

Reader and writer for the Java Object Serialization Stream Protocol, independent of Arma3Sync's
classes: an object graph model, byte-exact re-encoding, and the JDK's stream limits (depth,
references, size) so Rust never writes a stream Java would refuse.

Boundaries: no Arma3Sync types (those live in `a3s_types`).
