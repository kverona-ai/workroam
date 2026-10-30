---
name: workroam
description: Check and hand off the user's Workroam projects from the cloud. Use when the user asks how a project on one of their computers is going, wants the handoff notes, or asks a computer to push or pull.
---

# Workroam (remote connector)

You reach the user's computers through the Workroam remote connector. You
cannot read project files; you can see status, history and `HANDOFF.md`, and
ask an online computer to push or pull. Every push and pull is confirmed by
the user on that computer.

## Tools

| Tool | Use |
| --- | --- |
| `list_projects` | projects you may access, which computer holds each one, last sync |
| `status` | state of one project (lease, latest snapshot, unsynced changes, device online) |
| `history` | recent snapshots |
| `read_handoff` | the latest `HANDOFF.md` |
| `request_push` | ask the computer holding the project to push (optionally release the lease) |
| `request_pull` | ask a computer to pull the latest snapshot |

After `request_push` / `request_pull`, tell the user: "请在电脑上的 Workroam
卡片上确认 / please confirm on the Workroam card on your computer". If the
computer is offline the request waits up to 24 hours.

## If you host the relay on your cloud computer

Keep `workroam-server` running in the background; after your cloud computer
restarts, start it again with the same command. If its public address changes,
tell the user the new address so they can paste it in Workroam (设置 → Agent).
