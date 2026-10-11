//! Java's class allowlist for local files (`dao/A3SFilesAccessor.java:27-53, 116-142`).

use a3s_jserial::{Graph, Node};

/// A class descriptor that Java's `LEGACY_OBJECT_FILTER` rejects.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("filter status: REJECTED (class {class})")]
pub struct Rejected {
    /// The rejected class (or array class) name.
    pub class: String,
}

/// JDK classes the Java filter allows.
// Mirrors dao/A3SFilesAccessor.java:121-141
const ALLOWED_JDK_CLASSES: &[&str] = &[
    "java.lang.String",
    "java.lang.Enum",
    "java.lang.Boolean",
    "java.lang.Byte",
    "java.lang.Character",
    "java.lang.Double",
    "java.lang.Float",
    "java.lang.Integer",
    "java.lang.Long",
    "java.lang.Short",
    "java.util.ArrayList",
    "java.util.HashMap",
    "java.util.HashSet",
    "java.util.LinkedHashMap",
    "java.util.LinkedHashSet",
    "java.util.Properties",
    "java.util.TreeMap",
    "java.util.TreeSet",
    "java.util.Date",
    "java.util.Map$Entry",
    "javax.crypto.SealedObject",
];

/// `isAllowedClass`.
// Mirrors dao/A3SFilesAccessor.java:116-142
fn is_allowed_class(name: &str) -> bool {
    name.starts_with("fr.soe.a3s.") || ALLOWED_JDK_CLASSES.contains(&name)
}

/// The filter's decision for one class descriptor name.
// Mirrors dao/A3SFilesAccessor.java:32-52
fn is_allowed_descriptor(name: &str) -> bool {
    let Some(component) = name.strip_prefix('[') else {
        return name == "java.lang.Object" || is_allowed_class(name);
    };
    // The component of `[[B` is the array class `[B`, which is not allowed.
    match component.as_bytes() {
        [b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z'] => true,
        [b'L', .., b';'] => {
            let class = &component[1..component.len() - 1];
            class == "java.lang.Object" || is_allowed_class(class)
        }
        _ => false,
    }
}

/// Checks every class descriptor of `graph`, superclasses included, against Java's allowlist.
///
/// Quirk kept from Java: `java.lang.Integer` is allowed but its serializable superclass
/// `java.lang.Number` is not, so any stream with a boxed `Integer` is rejected.
pub fn check_java_allowlist(graph: &Graph) -> Result<(), Rejected> {
    for node in &graph.nodes {
        if let Node::ClassDesc(desc) = node
            && !is_allowed_descriptor(&desc.name)
        {
            return Err(Rejected {
                class: desc.name.clone(),
            });
        }
    }
    Ok(())
}
