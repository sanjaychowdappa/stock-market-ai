# Session Transcript

## `session_transcript_redacted.jsonl`

The complete raw conversation log of this project's development (~24.7 MB,
~8,700 lines, 2026-05 → 2026-07). One JSON object per line: user messages,
assistant responses, tool calls and their full output.

**Purpose:** continuity. If the machine is reset or a new session starts with no
memory, this preserves the full reasoning history — not just what was built, but
what was tried, what failed, and why each decision was made.

### ⚠️ Credentials are redacted

Alpaca API keys appeared in the original log (in `.env` reads and tool output).
Every occurrence has been replaced with `<REDACTED_CREDENTIAL>` — **20 in total**
— and the file was verified to contain no real key values before committing.

If you ever regenerate this file from a raw transcript, **redact before
committing.** This repository is public.

### How to use it

For a quick catch-up, prefer the curated docs — they are far more readable:

| Read this | For |
|---|---|
| `../RESUME_HERE.md` | How to restart and what is running |
| `../TIMELINE.md` | Dated history and per-day P&L |
| `../DECISIONS.md` | Settled verdicts and the reasoning behind them |

Use this transcript only when you need the full detail behind a specific
decision. To search it:

```powershell
Select-String -Path docs\session_transcript_redacted.jsonl -Pattern "kill criterion"
```

```bash
grep -o '.\{0,200\}kill criterion.\{0,200\}' docs/session_transcript_redacted.jsonl
```

### Caveat

This is an append-only record of a development session, not documentation. It
contains dead ends, corrected mistakes, and superseded conclusions. **Where it
disagrees with `DECISIONS.md`, `DECISIONS.md` wins** — it reflects the final
state of what was actually established.
