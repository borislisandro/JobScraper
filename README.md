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
| Worker protocol, CSS/XPath, feeds, pagination, robots, Retry-After, URL guard, platform HTTP execution | Passed with Node 24 fixtures, including local Workday/Eightfold/iCIMS/Jibe/Phenom listing, pagination/cursor, cookie, and detail requests |
| Frontend typecheck, workflow tests, production build | Passed |
| Rust formatting and dependency metadata | Passed |
| Backup archive and restore lifecycle source | Implemented: checksummed ZIP manifest, controlled-document enumeration, validation, staging, pre-pool intent application, safety snapshot, same-volume swap/rollback. Rust tests are present but cannot link on this host. |
| Offline BGE matching source | FastEmbed 6 user-defined local-resource loader, 384-D normalized BLOB cache, 350/50 chunks, resume/job/persona guarded 45/25/20/10 writes, any/all include mode, Tauri stale-run progress/cancellation, and migrated SQLite cache/guard/transaction tests authored but link-unexecuted. |
| Reminder/refocus source | One-shot `schtasks.exe /Create /XML` task boundary, scoped UUID task deletion/query, startup reconciliation, reminder-only `--deliver-reminder UUID` local DB/toast path, durable apply attempts and Rust focus events, plus interview 24h/1h reminder CRUD and ghost lifecycle source. Rust lifecycle tests are incomplete and link-unexecuted. |
| Source orchestration/security source | Stable dated 18-active-plus-reference starter pack plus conservative legacy reconciliation; manual run-ID Scrape All cancellation registry (two domains, one stream/domain, cooperative JSONL then tree-kill grace); frontend Resume/Cancel controls; DNS/IP/redirect and Playwright document/frame/popup guard; capped Retry-After; 1.5–3.0 second jitter; random-nonce authenticated sessions. Versioned platform request contracts now execute guarded direct listing/detail HTTP for Workday, Eightfold, iCIMS, TalentBrew/Jibe, and Phenom; declared fallback emits a warning. Node fixtures pass. Rust state tests authored but link-unexecuted. |
| Sources and resume/persona UI source | Manifest-driven structured source fields, browser-capable headed Capture Session with Resume & save/Cancel and no cookie display, probe/test preview table (normalized listing fields, warnings, completion, diagnostics only on expand), local PDF/DOCX import, image-only manual paste, immutable corrected resume versions, resume selection, and create/edit/archive persona controls covering titles, skills, any/all includes, excludes, location, work mode, salary, seniority, unknown policy, resume, and threshold. Saving an edit retains its ID and starts stale rescore. Node workflow tests pass; deterministic PDF text/image-only and DOCX paragraph/table Rust fixtures plus corrected-hash/reference tests are authored but link-unexecuted. |
| Application tracking source | Migration-backed recruiter/contact, source, rejection/withdrawal/acceptance fields; legal stage service with explicit historical override reason; immutable events; derived first response; audited notes; BLOB-backed exact application document snapshots with SHA-256/size/export verification; interview outcome plus existing 24h/1h reminders; Review’s explicit planned-application action; and board detail drawer/timeline. Frontend workflow tests pass; Rust migration/transition tests are authored but link-unexecuted. |
| Analytics source | Typed date/persona/source/company filter; application-created cohort window; immutable-event response/interview/offer/accepted conversion facts; stage, daily trend, source/company, outcomes, match-score, review, and freshness/availability tables; response-time supporting samples; explicit denominator and small-sample flags. Frontend dashboard semantics tests and migrated SQLite boundary fixture are authored; Rust runtime tests remain link-unexecuted. |
| Deduplication source | Canonical HTTP(S) URL normalization, source/external, requisition, and stable fingerprint exact matching during persistence; high-threshold title/location fuzzy suggestions only. Reviewed merge records full conflict row snapshots plus moved relationship IDs. Conflict controls now transactionally keep canonical, replace it with alias, or retain both as immutable history while preserving one current unique row; unmerge reconstructs resolved rows or safely refuses changed ownership. UI exposes merged history, conflict evidence, decisions, and errors. Frontend workflow tests pass; migrated SQLite merge/conflict tests are authored but link-unexecuted. |
| Export and data management source | Relational JSON exports carry version, UTC export time, stable IDs, and relationship closure; filter roots use UTC half-open `[start, end)` dates plus persona/source/company conditions, then include required parent and dependent rows only. RFC4180 CRLF CSV exports use identical closure for jobs, applications, events, interviews, and source/company outcomes. Native save dialog selects output; existing files require overwrite confirmation. Purge previews have ten-minute token/hash binding plus typed `PURGE`, audit records, protected application/review history, controlled-session path checks, and post-commit cleanup reporting. Rust migration/export/purge tests are authored but link-unexecuted; frontend workflow tests pass. |
| Rust compile/tests and Tauri package | Blocked: the isolated Rust toolchain cannot find MSVC `link.exe`; Visual Studio C++ Build Tools plus a Windows SDK are required. |

Restore archives contain only `snapshot/jobscraper.sqlite` and controlled `documents/...` entries listed with their byte size and SHA-256. Restore validates every ZIP entry before extracting under `%LOCALAPPDATA%\JobScraper\restore-staging`, then writes an intent. On the next startup, before a SQLite pool opens, it makes a `backups\pre-restore-*.zip` safety snapshot and atomically swaps the database and documents directory. Sessions, Credential Manager material, models, caches, logs, and temporary data are neither archived nor replaced. A failed swap rolls back and leaves its safety snapshot and diagnostic status.

No live careers-site check, Edge session capture, Windows Task Scheduler/toast delivery, installer, or linked Rust test claim is made by this matrix. Scheduler and notification source is present but unverified on a clean Windows packaged build.
