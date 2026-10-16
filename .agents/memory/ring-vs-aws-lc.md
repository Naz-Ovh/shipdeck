---
name: ring-vs-aws-lc
description: In russh, ring and aws-lc-rs only back the SSH session ciphers (AES-GCM, ChaCha20-Poly1305); key exchange and keys use RustCrypto either way
metadata:
  type: reference
---

russh 0.64 requires one crypto backend feature, `ring` or `aws-lc-rs` (default). Checked in its source: only the session cipher code is gated on them (`cipher/gcm.rs`, `cipher/chacha20poly1305.rs`). Key exchange, host keys and login keys (ed25519, RSA, ECDSA) come from RustCrypto crates (ssh-key, ed25519-dalek, p256, rsa) with either backend. So switching loses no algorithm a server could need.

- ring: compact Rust library with BoringSSL assembly; quick to build; ~2.2 MB smaller in shipdeck.
- aws-lc-rs: AWS's wrapper over AWS-LC (a BoringSSL fork); slow C build; adds a FIPS-certified option and post-quantum key exchange, which is why the ecosystem (rustls) moved to it. shipdeck needs neither.

Both use assembly for AES-GCM; SSH throughput is the same, and uploads are network-bound anyway. Used by [[binary-size]].
