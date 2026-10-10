# WP<id>: <title>

| Field | Value |
| --- | --- |
| Wave | <n> |
| Crate(s) | `a3s_xxx` (`crates/<group>/a3s_xxx`) |
| Agent | rust-implementer / egui-implementer |
| Java commit | <short sha of java_version/Arma3Sync HEAD> |
| Depends on (merged WPs) | <ids> |
| Wire-touching | yes / no |
| Live tests allowed | none / WOG FTP `.a3s/*` + files < 5 MB |
| Estimate | <Java lines in scope> Java / <expected Rust lines> Rust |

## Owned paths

- `crates/<group>/a3s_xxx/**`
- (explicit additive exceptions, e.g. `crates/net/a3s_transport/src/upload.rs`)

## Scope

3–8 bullets: what this WP delivers.

## Java sources

Paths relative to `java_version/Arma3Sync/src/main/java/fr/soe/a3s/`.

| File | Lines | Role |
| --- | --- | --- |
| `service/Example.java` | 1-99 | ... |

## Behaviour contract

Numbered, testable statements, each with a citation.

- B1. ... (`service/Example.java:42-57`)
- B2. ...

### Java quirks to preserve (wire-visible)

### Java quirks that may be dropped (local-only; needs an approved deviation)

### Errors, cancellation, progress

What is retried and how often; which errors abort vs. are collected; progress events emitted.

### Concurrency

Thread / connection counts and their limits in Java.

## Wire-compat notes

- Classes read/written: FQCN, serialVersionUID, fields in wire order, collection classes,
  shared instances.
- Byte identity required for: ... — semantic compatibility only for: ...
- Fixtures used: `tests/fixtures/...`; fixtures still needed from the JDK harness: ...

## Public API sketch

```rust
// key types and functions
```

Upstream APIs consumed: `a3s_transport::Transport::fetch`, ...

## Workspace deps needed

| Crate | Version | License | Why |
| --- | --- | --- | --- |

## Acceptance tests

- A1 (unit) ... — covers B1, B2
- A2 (golden) ...
- A3 (local server) ...
- A4 (live, `#[ignore = "network"]`) ...
- Ported Java tests: `src/test/java/...` → A<k>

## Out of scope

- ... (owned by WP<id>)

## Open questions

- Q1 ... Proposed default: ... Decision: <filled by orchestrator/user>

## Approved deviations

- none
