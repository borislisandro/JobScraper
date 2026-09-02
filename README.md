# JobScraper

Windows 11 local-first Tauri 2 desktop application for manually collecting jobs, filtering them, and tracking applications. It makes no startup network calls, has no telemetry or background scheduler, and never submits applications.

## The four pages

- **Jobs** — the home page. Tick the sources you want to look at; those sources scope both the list and what **Update Jobs** re-reads. Ticking is a view, not a setting — nothing is switched off by it, and which sources you follow at all lives on the Sources page. Openings their board has not listed in two complete reads are hidden behind **Include closed**; one missed read is badged instead. The first 200 results load immediately, with more available on demand. Update Jobs reads listing rows only; a job's detail page and full description are downloaded when that job is expanded. A run that fails or is cancelled keeps every listing it had already read, and **Full refresh** still leaves detail pages lazy. Filter by words in the title or stored listing content, sort by posting date, expand any row for the full listing, and press **Save** (adds it to Applications) or **Apply** (saves it and opens the posting in your browser). Saved and applied jobs are badged in the list.
- **Sources** — add a careers page by URL, select or deselect it, test it, and edit its detected settings.
- **Applications** — the board of everything you saved or applied to, with notes, documents, interviews and reminders.
- **Diagnostics** — local health information and the recent activity log.

Analytics, backup and export exist in the backend but have no UI in this build. Personas, relevance matching and resume import were removed: nothing could reach them, and they carried a bundled ONNX model, a FastEmbed dependency and roughly 1,300 lines of Rust with them.

## Run locally

Use Node 24 and pnpm 11. Developer mode installs the locked dependencies,
prepares the bundled sidecar, and starts Tauri without creating artifacts:

```powershell
pnpm app:dev
```

Create the current-user NSIS installer, the folder-based portable ZIP, or both:

```powershell
pnpm package:installer
pnpm package:portable
pnpm package:all
```

The commands accept `-NodePath` and `-PnpmPath` after `--`; otherwise they use
`JOBSCRAPER_NODE_RUNTIME` and `JOBSCRAPER_PNPM`, then `PATH`. Release builds
require Windows x64, Rust MSVC, Node 24, and pnpm 11. They run all release gates
and write versioned files plus `SHA256SUMS.txt` under `artifacts/<version>/`.
The portable ZIP needs an installed WebView2 Runtime and keeps its data under
`%LOCALAPPDATA%\JobScraper`, just like the installed edition.

Measure where a scrape's time goes. The benchmark drives the same worker the app drives, opens
the development database read-only, and never writes jobs, run history, or source state:

```powershell
node scripts/benchmark-scrape.mjs --fixtures   # deterministic, no network
node scripts/benchmark-scrape.mjs              # plus one check per enabled source and warm updates
```

Diagnostics shows the same metrics per run under "Scrape performance": the latest total, the
slowest measured step, per-source medians and p95, and the worker/request/database breakdowns.

Rust stores all application state under `%LOCALAPPDATA%\JobScraper`, using SQLite WAL, migrations, foreign keys, and a 5-second busy timeout. Job search is literal substring matching, not FTS5: the token index was dropped in migration 0017 because it could not match `C++` or `Verification/Validation`. Browser work is isolated in `sidecar/worker.mjs` and uses versioned JSONL over stdin/stdout. It only starts after a Test or Scrape action and is terminated by cancellation/exit.

## Verification matrix

| Gate | Status on this host |
| --- | --- |
| Worker protocol, CSS/XPath, feeds, pagination, robots, Retry-After, URL guard, platform HTTP execution | Passed with Node 24 fixtures, including local Workday/Eightfold/iCIMS/Jibe/Phenom listing, pagination/cursor, cookie, and detail requests |
| Frontend typecheck, workflow tests, production build | Passed |
| Scrape performance instrumentation | Every completed, failed or cancelled run records worker buckets, request kinds, app critical path, and database work in `app_logs` and the terminal run event; Diagnostics ranks them and `scripts/benchmark-scrape.mjs` reproduces them. Node, frontend, and Rust metric tests pass. |
| Rust formatting, metadata, compile, and tests | Passed: `cargo fmt --check`, `cargo check` with no warnings, and 64 Rust library tests. |
| Backup archive and restore lifecycle (no UI in this build; restore status still shows in Diagnostics) | Checksummed ZIP manifest, controlled-document enumeration, validation, staging, pre-pool intent application, safety snapshot, same-volume swap/rollback, and fault-injection tests pass. |
| Reminder/refocus | One-shot Task Scheduler XML, scoped UUID reconciliation, reminder-only local DB/toast path, durable apply attempts, Rust focus events, interviews, and ghost lifecycle are implemented; pure lifecycle tests pass. Live toast and scheduled delivery while closed still require manual observation. |
| Source orchestration/security | Stable starter pack, five cross-domain lanes with one serial stream per domain, cheap preflight checks, on-demand description enrichment, cooperative cancellation/tree cleanup, session capture, DNS/IP/redirect and Playwright navigation guards, Retry-After, and request pacing are implemented. Node fixtures pass. |
| Partial reads and completeness | Jobs are published as they are accepted, so a board that fails or is cancelled on page 480 of 500 persists the 479 pages already read. `complete` requires reaching an extent the board itself declared: an unreadable page count is reported as unfinished rather than collapsed to one page, and a run that discovers nothing is never complete. Only a complete run may close a job, so a broken selector no longer reconciles a whole board to closed. |
| Change checks | A source is skipped only when its own vendor total matches the last complete run's, and never more than three times running before it is read in full anyway — equal totals do not prove nothing changed, since one opening closing while another opens leaves the count identical. A failed check never skips a source; the full read has the retries behind it. |
| Jobs page | Source selection scopes the list and Update Jobs alike without changing which sources are switched on; empty selection lists nothing. Closed openings are out of the list unless asked for, and `possibly_closed` ones are badged. Results are paged 200 at a time and text input is debounced. Title and full-text filters require every typed word, ignore case and treat punctuation as text. Posting-date sort puts undated listings last. Save/Apply are one idempotent application per job. Rust and frontend tests pass. |
| Sources UI | Structured source forms, capture-session controls, and a normalized preview table are implemented. Frontend workflow tests pass. |
| Application tracking | Recruiter/contact/outcome fields, legal stage service, immutable events, derived response, audited notes, exact document snapshots, interviews/reminders, and board details are implemented; stages are named the same everywhere, with `planned` shown as Saved. Frontend and Rust migration/transition tests pass. |
| Analytics | Implemented in the backend and covered by migrated SQLite tests; no UI in this build. |
| Deduplication | Identity only: source+external id, canonical URL, requisition id, and a same-source title/company/location fingerprint. Similar-but-distinct openings are never merged and nothing proposes a merge, so the fuzzy review and merged-history UI are gone; the merge/unmerge commands and their audit tables remain. Rust transaction tests pass. |
| Export and data management | Implemented in the backend and covered by Rust tests; no UI in this build. |
| Tauri package | The release orchestrator builds a current-user NSIS installer and a folder-based portable ZIP, verifies their sidecars and SHA-256 hashes, and preserves `%LOCALAPPDATA%\JobScraper` data across upgrades and uninstall. Silent clean installer acceptance and portable launch/move acceptance remain manual checks. |

Restore archives contain only `snapshot/jobscraper.sqlite` and controlled `documents/...` entries listed with their byte size and SHA-256. Restore validates every ZIP entry before extracting under `%LOCALAPPDATA%\JobScraper\restore-staging`, then writes an intent. On the next startup, before a SQLite pool opens, it makes a `backups\pre-restore-*.zip` safety snapshot and atomically swaps the database and documents directory. Sessions, Credential Manager material, models, caches, logs, and temporary data are neither archived nor replaced. A failed swap rolls back and leaves its safety snapshot and diagnostic status.

No live careers-site, authenticated session/CAPTCHA, or native reminder-toast claim is made by this matrix. A clean packaged startup also revealed outbound connections made by the Evergreen WebView2 runtime itself; the application frontend and Rust startup path issue no HTTP requests, but strict process-level zero-startup-network acceptance remains unresolved.
