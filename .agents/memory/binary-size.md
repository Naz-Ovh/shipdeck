---
name: binary-size
description: Release binary is 13.3 MB; russh on ring plus fat LTO and codegen-units=1 measured 8.0 MB; proposed, not yet applied
metadata:
  type: project
---

Measured 2026-10-17 (x86_64 Linux release builds, `strip = true` already on):

| Build | Size |
|---|---|
| as is: russh default (aws-lc-rs), `lto = "thin"` | 13.3 MB |
| russh `default-features = false, features = ["ring", "rsa", "flate2"]` | 11.1 MB |
| plus `[profile.release] lto = "fat"`, `codegen-units = 1` | 8.0 MB |

The `man` manual is ~1% of the binary; it was 13 MB before it too. Every test passed with `ring`. `[profile.dist]` sets its own `lto = "thin"`, so it must change too or dist builds stay big.

Status: proposed to the user, not applied. Before releasing with it, run one real `shipdeck releases` against a server: the tests never open an SSH connection.

Rejected:
- `panic = "abort"` (would drop ~1.4 MB of unwind tables): a panic in a background task would kill the run instead of being reported as an error.
- nightly-only flags such as `-Zlocation-detail=none`: not the stable, documented way.
- compressing the manual: not worth it at ~125 KB.

What remains in 8 MB is code shipdeck uses (engine, std, russh, ssh-key, zstd, clap, tokio, TUI). See [[ring-vs-aws-lc]].
