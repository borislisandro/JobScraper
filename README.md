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

Before a release build, prepare the project-owned Node sidecar and pinned model
(the installed application needs neither system Node nor network model access):

```powershell
.\scripts\package-sidecar.ps1 `
  -NodePath C:\path\to\node.exe `
  -PnpmPath C:\path\to\pnpm.mjs
.\scripts\fetch-bge-model.ps1
.\scripts\prepare-release.ps1 -PreflightOnly
pnpm tauri build
```

Rust stores all application state under `%LOCALAPPDATA%\JobScraper`, using SQLite WAL, migrations, foreign keys, a 5-second busy timeout, and FTS5. Browser work is isolated in `sidecar/worker.mjs` and uses versioned JSONL over stdin/stdout. It only starts after a Test or Scrape action and is terminated by cancellation/exit.

The current model/adapter boundary expects a bundled offline `bge-small-en-v1.5` 384-dimensional ONNX artifact. No remote model loading is permitted.

## Verification matrix

| Gate | Status on this host |
| --- | --- |
| Worker protocol, CSS/XPath, feeds, pagination, robots, Retry-After, URL guard, platform HTTP execution | Passed with Node 24 fixtures, including local Workday/Eightfold/iCIMS/Jibe/Phenom listing, pagination/cursor, cookie, and detail requests |
| Frontend typecheck, workflow tests, production build | Passed |
| Rust formatting, metadata, compile, and tests | Passed: `cargo fmt --check`, metadata, `cargo check`, and all 43 Rust library tests. |
| Backup archive and restore lifecycle | Checksummed ZIP manifest, controlled-document enumeration, validation, staging, pre-pool intent application, safety snapshot, same-volume swap/rollback, and fault-injection tests pass. |
| Offline BGE matching | FastEmbed 6 uses only pinned local resources; 384-D normalized BLOB cache, 350/50 chunks, guarded 45/25/20/10 writes, any/all filters, explanations, and stale-run cancellation are implemented. Both source-tree and installed-resource BGE inference smoke tests return 384 dimensions without runtime downloads. |
| Reminder/refocus | One-shot Task Scheduler XML, scoped UUID reconciliation, reminder-only local DB/toast path, durable apply attempts, Rust focus events, interviews, and ghost lifecycle are implemented; pure lifecycle tests pass. Live toast and scheduled delivery while closed still require manual observation. |
| Source orchestration/security | Stable starter pack, manual two-domain Scrape All, cooperative cancellation/tree cleanup, session capture, DNS/IP/redirect and Playwright navigation guards, Retry-After, and jitter are implemented. Node fixtures pass. Installed Node 24 plus system Edge completed `started`, `progress`, `job`, `completed` against a local fixture and left no new Edge process. |
| Sources and resume/persona UI | Structured source forms, capture-session controls, normalized preview table, PDF/DOCX import and correction, immutable resume versions, and full create/edit/archive persona controls are implemented. Frontend workflow and Rust document fixture tests pass. |
| Application tracking | Recruiter/contact/outcome fields, legal stage service, immutable events, derived response, audited notes, exact document snapshots, interviews/reminders, review actions, and board details are implemented. Frontend and Rust migration/transition tests pass. |
| Analytics | Filtered event-derived funnel, response metrics, trends, source/company outcomes, score/review/freshness tables, explicit denominators, and small-sample flags are implemented. Frontend semantics and migrated SQLite tests pass. |
| Deduplication | Exact/fingerprint matching, conservative fuzzy review, lossless conflict snapshots, explicit conflict decisions, merge/unmerge, history UI, and audit evidence are implemented. Frontend and Rust transaction tests pass. |
| Export and data management | Relationship-closed filtered JSON; RFC4180 jobs/applications/events/interviews/source/company CSV; overwrite-safe save dialog; and token/hash/expiry-bound purge preview are implemented. Frontend and Rust tests pass. |
| Tauri package | Current-user NSIS build passes. Silent clean install places binaries under `%LOCALAPPDATA%\Programs\JobScraper`, preserves `%LOCALAPPDATA%\JobScraper` data across reinstall/uninstall, and includes physical production Node modules plus pinned offline model resources. |

Restore archives contain only `snapshot/jobscraper.sqlite` and controlled `documents/...` entries listed with their byte size and SHA-256. Restore validates every ZIP entry before extracting under `%LOCALAPPDATA%\JobScraper\restore-staging`, then writes an intent. On the next startup, before a SQLite pool opens, it makes a `backups\pre-restore-*.zip` safety snapshot and atomically swaps the database and documents directory. Sessions, Credential Manager material, models, caches, logs, and temporary data are neither archived nor replaced. A failed swap rolls back and leaves its safety snapshot and diagnostic status.

No live careers-site, authenticated session/CAPTCHA, or native reminder-toast claim is made by this matrix. A clean packaged startup also revealed outbound connections made by the Evergreen WebView2 runtime itself; the application frontend and Rust startup path issue no HTTP requests, but strict process-level zero-startup-network acceptance remains unresolved.
