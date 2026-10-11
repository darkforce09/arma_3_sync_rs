# a3s_dev_tools

Part of the Arma3Sync Rust rewrite (see [`ARCHITECTURE.md`](../../../ARCHITECTURE.md)).

Developer tooling: fixture regeneration, code checks and release helpers.

Wraps the Java fixture harness ([`tools/java_fixture_harness`](../../../tools/java_fixture_harness/README.md)),
which runs the real Arma3Sync Java code (JDK 25, Unix only):

| Command | Purpose |
| --- | --- |
| `regen-fixtures [--out <dir>] [--only <parts>]` | Generate the golden fixtures into a directory (default: a kept temporary one) |
| `regen-fixtures --write-fixtures` | Generate everything and install it into `tests/fixtures` (orchestrator only) |
| `regen-fixtures --check` | Generate everything and compare with `tests/fixtures`; exit 1 on differences |
| `java-verify [--dump-dir <d>] [--semantic] <paths>...` | Read Rust-produced files with Arma3Sync's own Java readers |
| `java-dump [--semantic] <file>` | Canonical JSON dump of a Java object graph |
| `java-roundtrip <in> <out>` | Java read plus upload-style re-serialization |

```sh
scripts/gate.sh cargo run -p a3s_dev_tools -- regen-fixtures --out /tmp/fixtures
```

The library API (`Harness`, `fixtures`) can be used as a dev-dependency to cross-check Rust
output in tests. JDK-backed tests are `#[ignore]`d:

```sh
scripts/gate.sh cargo test -p a3s_dev_tools -- --ignored --test-threads=1
```
