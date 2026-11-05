# .agents

Agent-only notes: where the project is, what is next, and how to work on it.
Explanations for people live in [/docs](../docs) and the [README](../README.md).

**The plan** is [docs/project-progress.html](../docs/project-progress.html), the owner's official tracker:
every step in phases, each with a status and a permanent id (`p1b1`). When a step changes state, edit its row
in the page's `PLAN` data and the "Up next" box, and bump `UPDATED`; the comment at the top of the file has the
rules. Write the page for the owner in very easy English: short sentences, common words, a full stop at the end
of every sentence. Agent-only detail (test logs, file paths, open threads) goes in [handoff.md](handoff.md).
Deliver one step at a time and don't jump ahead unasked; a step stays **built** until the owner has tried it.
**The spec** is [docs/visual/Plan/spec.html](../docs/visual/Plan/spec.html): the agreed design of what comes next,
as a canvas. Keep it true when a decision changes. Keep the `docs/` root clean: visual pages go under
`docs/visual/`, feature plans under [docs/feature/](../docs/feature/), code guides under [docs/code/](../docs/code/);
[../AGENTS.md](../AGENTS.md) says which page to read for which task.

**Start here, in this order:** [../AGENTS.md](../AGENTS.md) (rules) → [handoff.md](handoff.md) →
[the plan](../docs/project-progress.html) → [memory/](memory/MEMORY.md).

| file / dir | brief |
| --- | --- |
| [handoff.md](handoff.md) | Where the last session stopped, what the owner is trying, what to do on resume. Overwrite it at the end of every session. |
| [memory/](memory/MEMORY.md) | Decisions and facts the owner gave, one file each, with why and how to apply. |
