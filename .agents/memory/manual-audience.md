---
name: manual-audience
description: shipdeck man is user docs for people driving shipdeck through an LLM; no development or code-layout docs
metadata:
  type: project
---

`shipdeck man` (text in `bin/shipdeck/manual/`) is for people using shipdeck and the LLM helping them: setting it up, deploying, configuring deploy.yaml, troubleshooting, understanding what each step does. It is not development documentation: no project layout, module names, code internals or contributing rules.

Development docs come later, in `docs/code/` and `docs/features/` (said 2026-10-17); don't put them in the manual meanwhile.

**Why:** the user removed the `development` topic and said the manual holds "only what an LLM needs to work on a project" that uses shipdeck.

**How to apply:** when adding or editing manual pages, write for a shipdeck user; link to properties by path (`service.env`) rather than describing code. See [[manual-detail-level]].
