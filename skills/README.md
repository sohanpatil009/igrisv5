# skills/ — JARVIS skill library

Versioned procedural knowledge for IGRIS v5. Each skill is a JSON manifest
validated by `igris-skills` (`name` kebab-case, non-empty tools/steps,
permissions in L0..L4, non-empty validation, explicit `cautions`).

**Security protocol, always on:**
- L0–L1 read-only and draft-only skills run freely.
- L2 skills show their plan first (dry-run where it matters).
- L3–L4 skills STOP for explicit user approval (`requires_approval()` +
  `approval_gated()` in the registry). No silent deploys, restarts, or deletes.
- Secrets never leave the device; backups stay local; sends are drafts until approved.

## Library (18 skills)

| Skill | Level | Tools | What it does |
|---|---|---|---|
| `morning-brief` | L0–L1 | memory_search | Standup brief: done/doing/blocked |
| `summarize-thread` | L0–L1 | memory_search | 5-bullet summary + verdict |
| `web-research` | L0–L1 | browser | Sourced findings with confidence |
| `system-status` | L0 | terminal | CPU/RAM/disk/battery/network brief |
| `translate-text` | L0–L1 | memory_search | Privacy-routed translation |
| `explain-error` | L0–L1 | memory_search, browser | Safest-fix-first diagnosis |
| `meeting-prep` | L0–L1 | memory_search | Attendees, decisions, agenda |
| `set-reminder` | L1 | messaging | Drafts only, approval to schedule |
| `file-organizer` | L1–L2 | filesystem | Dry-run first, moves never deletes |
| `backup-notes` | L1 | filesystem | Local timestamped memory export |
| `network-diagnostics` | L1 | terminal | Local/DNS/upstream verdict |
| `code-review` | L1 | terminal, filesystem | Read-only diff review, no commits |
| `lockdown-check` | L1 | memory_search | Audit: gated skills, sessions, jobs |
| `open-application` | L2 | terminal | Allowlisted apps, no elevation |
| `run-tests` | L2 | terminal | Workspace tests with timeout |
| `clean-temp` | L2 | filesystem | Scoped temp clean, approval to delete |
| `restart-service` | L3 | terminal | APPROVAL: named local service only |
| `deploy-service` | L3–L4 | terminal, docker | APPROVAL: plan + rollback + health verify |

New skills start enabled only after review; repeated successes surface via
`promotion_candidates()`. Nothing self-installs.
