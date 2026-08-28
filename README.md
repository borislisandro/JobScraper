# JobScraper

Windows 11 local-first Tauri 2 desktop application for manually collecting jobs, matching them to local personas, reviewing them, and tracking applications. It makes no startup network calls, has no telemetry or background scheduler, and never submits applications.

## Run locally

Use Node 24 LTS and pnpm:

```powershell
pnpm install
pnpm check
pnpm build
pnpm tauri dev
```

Before a release build, package the project-owned Node sidecar (not a system Node installation):

```powershell
.\scripts\package-sidecar.ps1 -NodePath C:\path\to\node.exe
```

Rust stores all application state under `%LOCALAPPDATA%\JobScraper`, using SQLite WAL, migrations, foreign keys, a 5-second busy timeout, and FTS5. Browser work is isolated in `sidecar/worker.mjs` and uses versioned JSONL over stdin/stdout. It only starts after a Test or Scrape action and is terminated by cancellation/exit.

The current model/adapter boundary expects a bundled offline `bge-small-en-v1.5` 384-dimensional ONNX artifact. No remote model loading is permitted.

## Verification matrix

| Gate | Status on this host |
| --- | --- |
| Worker protocol, CSS/XPath, feeds, pagination, robots, rate limits, adapter request builders | Passed with Node 24 fixtures |
| Frontend typecheck, workflow tests, production build | Passed |
| Rust formatting and dependency metadata | Passed |
| Backup archive and restore lifecycle source | Implemented: checksummed ZIP manifest, controlled-document enumeration, validation, staging, pre-pool intent application, safety snapshot, same-volume swap/rollback. Rust tests are present but cannot link on this host. |
| Offline BGE matching source | FastEmbed 6 user-defined local-resource loader, 384-D normalized BLOB cache, 350/50 chunks, resume/job/persona guarded 45/25/20/10 writes, any/all include mode, Tauri stale-run progress/cancellation, and migrated SQLite cache/guard/transaction tests authored but link-unexecuted. |
| Reminder/refocus source | One-shot `schtasks.exe /Create /XML` task boundary, scoped UUID task deletion/query, startup reconciliation, reminder-only `--deliver-reminder UUID` local DB/toast path, durable apply attempts and Rust focus events, plus interview 24h/1h reminder CRUD and ghost lifecycle source. Rust lifecycle tests are incomplete and link-unexecuted. |
| Rust compile/tests and Tauri package | Blocked: the isolated Rust toolchain cannot find MSVC `link.exe`; Visual Studio C++ Build Tools plus a Windows SDK are required. |

Restore archives contain only `snapshot/jobscraper.sqlite` and controlled `documents/...` entries listed with their byte size and SHA-256. Restore validates every ZIP entry before extracting under `%LOCALAPPDATA%\JobScraper\restore-staging`, then writes an intent. On the next startup, before a SQLite pool opens, it makes a `backups\pre-restore-*.zip` safety snapshot and atomically swaps the database and documents directory. Sessions, Credential Manager material, models, caches, logs, and temporary data are neither archived nor replaced. A failed swap rolls back and leaves its safety snapshot and diagnostic status.

No live careers-site check, Edge session capture, Windows Task Scheduler/toast delivery, installer, or linked Rust test claim is made by this matrix. Scheduler and notification source is present but unverified on a clean Windows packaged build.
