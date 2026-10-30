---
name: workroam
description: Sync this project between computers with Workroam. Use when the user says they are leaving or switching machines ("我要下班了", "push my work", "同步上去"), when they arrive on another machine ("同步下来", "pull the latest"), or asks where the work stands.
---

# Workroam: hand off this project between computers

Workroam keeps this project folder in sync across the user's computers through
their own encrypted storage. Only one computer writes at a time (the *lease*).
Every sync needs the user's confirmation on a Workroam card.

## When the user is leaving ("我要下班了" / "I'm heading home" / "push")

1. Update `.handoff/HANDOFF.md` first. Keep the template headings and write:
   current goal, what is done, what is in progress, the next steps, open
   questions and anything the next agent must watch out for. Put the time, this
   machine and your agent name in the header.
2. Run:

   ```bash
   workroam push --release --json
   ```

3. While it runs, tell the user: "请在 Workroam 卡片上确认 / please confirm on the
   Workroam card". The command waits up to 10 minutes.
4. Read the final JSON line and report the result (snapshot id, files, time).

## When the user arrives ("同步下来" / "pull the latest")

1. Run `workroam pull --json` and ask the user to confirm on the card.
2. Read `.handoff/HANDOFF.md` and the machine block at the end of `AGENTS.md`
   (it says which repositories can be pushed from this computer).
3. Summarise where the work stands and continue from "下一步 / Next steps".

## Checking state

- `workroam status --json`: lease holder, latest snapshot, unsynced changes,
  new snapshots on the server, which repositories this machine can push.
- `workroam history --json`: recent snapshots.
- `workroam handoff --json`: structured summary of HANDOFF.md.

## Results and exit codes

Every `--json` result has `code`, equal to the exit code:

| code | meaning | what to do |
| --- | --- | --- |
| 0 | success | report the result |
| 10 | the user denied | stop; do not retry |
| 11 | no confirmation in time (treated as "later") | tell the user it is waiting; retry only if asked |
| 12 | another computer holds the lease | tell the user who holds it; `workroam pull --wait 30m` waits, `--takeover` forces (ask first) |
| 13 | cross-platform problem (case conflict, illegal Windows name) | rename the listed files, then push again |
| 14 | conflicts to resolve | open the listed `*.conflict-*` files, merge them, delete the copies, push again |
| 15 | a git rebase/merge is in progress | finish or abort it first |
| 16 | read-only device | this computer cannot push |
| 17 | the project is archived | ask the user before `workroam archive restore` |
| 20 | network or storage error | retry later; the next push resumes |

## Rules while working

- While `.workroam/SYNCING` exists, Workroam is writing files: do not edit the
  project until it disappears.
- Never edit the block between `<!-- workroam:env:begin -->` and
  `<!-- workroam:env:end -->` in `AGENTS.md`; it is generated per machine.
- Files such as `.mcp.json`, `.codex/config.toml` or `.claude/skills` are
  generated from `.workroam/mcp.toml` and `.agents/skills/`. Edit the sources.
- Only commit locally in repositories this machine cannot push to; Workroam
  carries the commits to the machine that can.
