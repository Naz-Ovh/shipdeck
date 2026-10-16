---
name: manual-detail-level
description: Manual pages must be detailed but token-efficient - full sentences with why and what to do; neither bloated nor compressed fragments
metadata:
  type: feedback
---

The manual's detail level sits between two versions the user rejected:
- ~145 KB of details: "bloated" (box art, wide wrapped tables, the same facts on several pages);
- ~57 KB: "too compact" (`a | b | c` fragments, facts without the reason or the fix).
The accepted version is ~105 KB: full sentences and short paragraphs, the reason behind a behaviour, what to do when it fails, short examples, each fact once. The index stays as short as possible: one `key: brief` line per topic, a few words each.

**Why:** an LLM reads it into its context, so tokens matter, but it must still understand the behaviour well enough to act and troubleshoot.

**How to apply:** when writing or editing a page, prefer one clear sentence over a fragment list, and never repeat a fact another page or `configs.yaml` already holds; link by key instead. See [[manual-audience]].
