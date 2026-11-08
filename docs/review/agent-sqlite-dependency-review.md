# Agent SQLite dependency review

Date: 2026-11-07. Scope: `rusqlite` 0.41.1 with minimal features.
This is dependency research. No project dependency was added.

## Evidence

Read the cached published manifests for `rusqlite` 0.41.1 and
`libsqlite3-sys` 0.39.1. Checked the versioned upstream README through Context7.
Resolved two isolated projects under `/tmp/shipdeck-rusqlite-review-qqkxzryw`.
Used a copy of Shipdeck's current lockfile to preserve compatible existing
versions. Ran offline Cargo dependency trees and metadata for Linux x86_64.
Compared the results with the workspace lockfile and the agent's current tree.

## Minimal configurations

```toml
# Requires the server's compatible SQLite library.
rusqlite = { version = "0.41.1", default-features = false }

# Compiles SQLite into the executable; no server SQLite package is needed.
rusqlite = { version = "0.41.1", default-features = false, features = ["bundled"] }
```

These are alternatives, not two dependencies to install together.
Both support ordinary queries, parameter binding, and transactions.
Defaults are disabled to omit the prepared-statement cache and WASM backend.
The optional Rust WAL hook feature is not needed to issue SQLite WAL pragmas.

## Resolved bundled tree

```text
rusqlite 0.41.1
├── bitflags 2.13.2
├── fallible-iterator 0.3.0
├── fallible-streaming-iterator 0.1.9
├── libsqlite3-sys 0.39.1
│   └── build dependencies
│       ├── cc 1.8.0
│       │   ├── find-msvc-tools 0.1.14
│       │   └── shlex 2.0.1
│       ├── pkg-config 0.3.34
│       └── vcpkg 0.2.15
└── smallvec 1.16.3
```

Eleven packages including rusqlite: six normal packages and five build helpers.
Without bundled SQLite, cc and its two helpers disappear from this isolated
tree: eight packages including rusqlite. `pkg-config` and `vcpkg` remain enabled
through libsqlite3-sys defaults, even though rusqlite defaults are disabled.

| Package | In workspace lockfile? | In current agent dependency tree? |
| --- | --- | --- |
| rusqlite | No. | No. |
| libsqlite3-sys | No. | No. |
| fallible-iterator | No. | No. |
| fallible-streaming-iterator | No. | No. |
| vcpkg | No. | No. |
| bitflags | Yes, same version. | No. |
| smallvec | Yes, same version. | Yes. |
| cc | Yes, same version. | No. |
| find-msvc-tools | Yes, same version. | No. |
| shlex | Yes, same version. | No. |
| pkg-config | Yes, same version. | No. |

Both choices add five package names to the workspace: four normal packages
including rusqlite, plus the vcpkg build helper. Bundling adds no further new
package names because the C compiler helpers are already present elsewhere.
Existing elsewhere does not mean those helpers already build for the agent.
Build helpers are not runtime libraries shipped beside the agent.

## Findings and limits

- No hashlink, new hashbrown, bindgen, SQLCipher, OpenSSL, async database
  runtime, or ORM is selected by the minimal bundled configuration.
- Bundling still adds SQLite's C code and native compilation. Package count
  does not measure executable size, memory, or build time.
- Non-bundled builds rely on a compatible server library. The default binding
  baseline in libsqlite3-sys is SQLite 3.45.3. Verify target compatibility
  before choosing dynamic linking, especially for older servers and musl.
- The local machine has neither sqlite3 on PATH nor SQLite pkg-config data.
  No non-bundled compile or runtime check was performed.
- This review did not compile a size comparison or audit vulnerabilities.
  Version 0.41.1 was checked directly; no claim is made that it is the newest
  release available online.

## Recommendation

Use default-features = false with only bundled, subject to dependency approval.
It is a small dependency tree and keeps the agent independent of server SQLite
versions. Measure the release executable size before accepting a size claim.
The owner has not yet approved installation of this dependency.

## libsql comparison

The owner also asked about the Rust libsql library. Checked cached published
libsql 0.10.0 manifests and resolved an isolated project with
`default-features = false, features = ["core"]`. This enables local embedded
access without remote, replication, sync, or TLS features.

For Linux x86_64, normal/build dependency traversal gives 69 unique
package-version pairs including libsql, compared with 11 for minimal bundled
rusqlite. Compared with Shipdeck's lockfile, libsql adds 24 package-version
pairs (including extra versions of existing package names); rusqlite adds five.
These are dependency counts, not executable-size measurements.

Even core-only libsql includes libsql-sys, libsql-ffi, bindgen, clang-sys,
cmake, tracing, async-trait, and zerocopy 0.7.35. Futures, parking_lot, and many
other dependencies already exist in the workspace. libsql itself is absent
from the project lockfile. Evidence is in the same temporary review directory,
under libsql-core/{Cargo.toml,Cargo.lock,tree.txt,metadata.json}.

libsql offers an async-shaped API and a shared interface for local and remote
databases. It is not just a different Rust wrapper for unchanged upstream
SQLite: core links libSQL's own native implementation through libsql-ffi.
Those capabilities may suit applications using remote databases or replicas.
For this agent's local event store, minimal rusqlite remains the simpler
dependency choice. No libsql dependency was added or compiled in this review.

## Source links

- [Versioned rusqlite README](https://github.com/rusqlite/rusqlite/blob/v0.41.1/README.md).
- [Published rusqlite manifest](https://docs.rs/crate/rusqlite/0.41.1/source/Cargo.toml).
- [Published libsqlite3-sys manifest](https://docs.rs/crate/libsqlite3-sys/0.39.1/source/Cargo.toml).
- [Published libsql manifest](https://docs.rs/crate/libsql/0.10.0/source/Cargo.toml).
- [libsql Rust local builder](https://github.com/tursodatabase/libsql/blob/main/libsql/src/database/builder.rs).
