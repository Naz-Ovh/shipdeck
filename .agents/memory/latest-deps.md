---
name: latest-deps
description: Keep every dependency on its latest version when the upgrade costs nothing, and never ship two copies of a crate shipdeck could share
metadata:
  type: feedback
---

When a newer version of a dependency exists and the upgrade costs nothing (or only a small code change), switch to it. When shipdeck's own dependency is the reason a crate is compiled twice (sha2 0.10 next to russh's 0.11 was), move to the version the others use.

**Why:** the user said (2026-10-17) "it doesn't make sense to use two copy" and "if upgrade doesn't cost anything then switch to latest version always". Duplicates also grow the binary; see [[binary-size]].

**How to apply:** when touching dependencies, run `cargo info <crate>` for the workspace ones (it prints `latest` when behind), `cargo update` for compatible bumps, and `cargo tree -d -e normal` for duplicates. Duplicates that come only from other crates (generic-array, hashbrown, sha3, syn) are theirs to fix; leave them.
