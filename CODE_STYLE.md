# Code style

Follows the spirit of [rerun's code style](https://github.com/rerun-io/rerun/blob/main/CODE_STYLE.md).

* Each crate starts with a `//!` doc comment and has a `README.md` with a one-line description.
* Crate names are `a3s_<name>`, grouped under `crates/<group>/`.
* Library crates return typed errors (`thiserror`); binaries may use `anyhow`.
* No `unwrap()` in library code paths that can be reached with bad input; use `expect("reason")`
  only for true invariants.
* Name threads (`std::thread::Builder`), use `parking_lot` locks.
* Anything that touches the Java wire format must cite the Java source it mirrors
  (e.g. `// Mirrors dao/repository/RepositoryBuildProcessor.java:468`) and be covered by a golden
  fixture test.
* Names that are part of the Java wire format keep their original (misspelled) form, e.g.
  `configration`, `protocole`, `racine`, `hidddenFolderPaths`.
