# Plan: background scraping with a "new jobs" Windows notification

## Goal

The user wants to stop checking manually. Target flow:

1. Windows starts (or a few hours pass).
2. JobScraper runs headless and scrapes every enabled source.
3. When the batch finishes, the app works out which listings are genuinely new.
4. One Windows toast is shown summarising them. Nothing else pops up.

## What already exists (do not rebuild)

- **Toast delivery.** `tauri-plugin-notification` is registered in `src-tauri/src/lib.rs:46` and already used from Rust at `lib.rs:69`. Rust-side `app.notification()` calls are not gated by `src-tauri/capabilities/default.json` (that file gates frontend IPC), so no capability change is needed.
- **A headless CLI mode.** `--deliver-reminder <uuid>` (`lib.rs:26-38`, `lib.rs:59-90`) is a working precedent: the process opens the database, shows one toast, writes back to the DB, and calls `app.handle().exit(0)` without ever handing control to the frontend. Mirror this shape exactly.
- **A Task Scheduler boundary.** `src-tauri/src/notifications.rs` shells out to `schtasks.exe` directly (never through a shell), builds task XML, escapes values, and scopes deletion to task names it owns. Reuse it.
- **Batch scraping.** `scrape_all_inner` (`src-tauri/src/sidecar.rs:1493`) runs all enabled sources across lanes and is already reused by the UI button and by `start_automatic_sync` (`sidecar.rs:1730`).
- **The data needed to define "new".** `jobs.created_at` is set on insert and is *not* in the `ON CONFLICT(id) DO UPDATE` set list (`src-tauri/src/db.rs:997`), so it survives re-scrapes and is a reliable first-seen timestamp.

## What is missing

- No autostart or scheduled trigger; the scrape only begins because React calls `startAutomaticSync()` on mount (`src/main.tsx:167`), so the window must be open.
- Nothing computes new-since-a-timestamp. `scrape_runs` tracks `discovered_count` and `skipped_count`, neither of which is "new".
- The end of a batch only writes an `app_logs` row (`sidecar.rs:1696`). No toast.

---

## Hazards to design around

These are the non-obvious parts. Getting them wrong produces either a visible window at logon, a double scrape, or a cancelled run.

### H1. The window is visible by default

`src-tauri/tauri.conf.json` declares the main window with no `"visible"` key, so it defaults to visible, and Tauri creates windows *before* the `setup` closure runs. A background sync would flash — then keep showing — a full app window.

**Fix:** add `"visible": false` to the window object in `tauri.conf.json`, and call `window.show()` explicitly in `setup` for normal mode only. This also removes the window flash that `--deliver-reminder` causes today.

### H2. The frontend would start a second scrape

Even with a hidden window the webview still boots, React mounts, and `startAutomaticSync()` fires — a second full batch inside the background process.

**Fix:** add `headless: bool` to `AppState` (`lib.rs:20-25`). `start_automatic_sync` returns `Ok(false)` immediately when it is set.

### H3. Opening the GUI cancels a running background sync

`recover_interrupted_runs` (`db.rs:603`) runs on every startup and unconditionally deletes `job_occurrences` for, and cancels, every run with `status='running'`. If the user opens the GUI while the background sync is mid-batch, that sweep destroys the in-flight run's sightings.

**Fix:** a cross-process heartbeat (see below). `recover_interrupted_runs` skips the sweep entirely while the heartbeat is fresh.

### H4. Two processes scraping at once

Benign for SQLite (WAL handles concurrent writers), but it doubles outbound requests and wastes the incremental-scrape work. The heartbeat covers this too.

### H5. `is_managed_task_path` will refuse the sync task

`notifications.rs:151` only accepts `\JobScraper\JobScraper-Reminder-<uuid>`. `cancel()` and `exists()` reject anything else, so the sync task cannot be removed through the existing helpers without a second, equally narrow guard.

**Fix:** add `is_managed_sync_task(value) -> bool` matching exactly `\JobScraper\JobScraper-Sync` (after `/`→`\` normalisation), and widen the guard in `cancel`/`exists` to `is_managed_task_path(x) || is_managed_sync_task(x)`. Do not loosen the guards to a prefix match.

### H6. `current_exe()` in a dev build

`std::env::current_exe()` in a debug build points at `target/debug/JobScraper.exe`, which uses the `JobScraper-dev` data directory (`lib.rs:52-56`). A task registered from a dev build will scrape the dev database. Acceptable for testing; note it in the UI copy or gate registration behind `cfg!(not(debug_assertions))`.

---

## Implementation

### Step 1 — Heartbeat helper (`src-tauri/src/db.rs`)

One `settings` row, no migration (the `settings` table already exists; see `db.rs:472`).

```rust
pub const SYNC_HEARTBEAT: &str = "sync.heartbeatAt";
const HEARTBEAT_FRESH_SECS: i64 = 300;

/// True while some process is mid-batch. Stale after five minutes, so a crashed
/// background sync unblocks the GUI on its own without a cleanup path.
pub async fn sync_in_progress(pool: &SqlitePool) -> bool { ... }
pub async fn write_sync_heartbeat(pool: &SqlitePool) { ... }
pub async fn clear_sync_heartbeat(pool: &SqlitePool) { ... }
```

Read the stored RFC3339 value, parse with `chrono::DateTime::parse_from_rfc3339`, compare against `Utc::now()`. A missing or unparseable value means "not in progress".

Then in `recover_interrupted_runs` (`db.rs:603`), return `Ok(0)` early when `sync_in_progress` is true. Add a comment saying why — a future reader will otherwise "fix" it back.

### Step 2 — New-jobs query (`src-tauri/src/db.rs`)

```rust
pub struct NewJob { pub title: String, pub company: String }

/// Listings first stored after `since`. `created_at` is insert-only in the jobs
/// upsert, so a re-scrape of an existing listing does not resurface it here.
pub async fn new_jobs_since(pool: &SqlitePool, since: &str, limit: i64)
    -> ApiResult<(i64, Vec<NewJob>)>
```

```sql
SELECT count(*) FROM jobs
 WHERE created_at >= ?1 AND availability != 'closed';

SELECT title, company FROM jobs
 WHERE created_at >= ?1 AND availability != 'closed'
 ORDER BY created_at DESC LIMIT ?2;
```

`created_at` is written by `db.rs`'s `now()` = `Utc::now().to_rfc3339()`, a fixed `+00:00` offset, so string comparison is a valid time comparison. Do not mix in `chrono::Local`.

No filter for the scrape title filter is needed — `title_passes_scrape_filter` already ran at scrape time, so nothing that fails it was ever stored.

### Step 3 — Notification text (`src-tauri/src/sidecar.rs` or a small `alerts` module)

Keep the string building in a pure function so it is testable without a toast:

```rust
/// None when nothing is new — the batch should stay silent rather than
/// notify "0 new jobs" every few hours.
pub fn new_jobs_summary(count: i64, samples: &[NewJob]) -> Option<(String, String)>
```

- Title: `"1 new job"` / `"{count} new jobs"`.
- Body: up to three `"{title} — {company}"` lines joined with `\n`, then `"and {count - shown} more"` when there is a remainder.
- `count <= 0` returns `None`.

Then the delivery wrapper:

```rust
pub async fn notify_new_jobs(app: &tauri::AppHandle, pool: &SqlitePool, since: &str)
```

which calls `new_jobs_since(pool, since, 3)`, builds the summary, and on `Some` calls `app.notification().builder().title(t).body(b).show()`. Log the outcome through `db::log(pool, "info", None, None, "new_jobs_alert", ...)` so the Diagnostics log shows what was announced.

Do not add a click handler. The existing reminder path does not use one, and handling activation in a process that exits immediately is not reliable.

### Step 4 — Wire both callers (`src-tauri/src/sidecar.rs`)

Do **not** put this inside `scrape_all_inner` — that also serves the manual "Rebuild all listings" button, where a toast is noise and the UI already shows the result.

In `start_automatic_sync` (`sidecar.rs:1730`), and in the new `--sync` path:

```rust
if state.headless || db::sync_in_progress(&state.db.pool).await { return Ok(false); }
let since = chrono::Utc::now().to_rfc3339();   // capture BEFORE the batch
db::write_sync_heartbeat(&state.db.pool).await;
// spawn a task that re-writes the heartbeat every 60s until the batch returns
let result = scrape_all_inner(app.clone(), state.clone(), Some("automatic-startup".into()), Vec::new(), false).await;
db::clear_sync_heartbeat(&state.db.pool).await;
notify_new_jobs(&app, &state.db.pool, &since).await;
```

The heartbeat must be written *before* the batch starts, so a GUI launched one second later already sees it and skips its `recover_interrupted_runs` sweep.

`chrono` is already a dependency; add the import to `sidecar.rs` if absent.

### Step 5 — `--sync` CLI mode (`src-tauri/src/lib.rs`)

Mirror `parse_deliver_reminder_args` exactly:

```rust
fn parse_sync_argument(args: &[String]) -> Result<bool, String>
```

`--sync` present with no other non-exe argument returns `true`; `--sync` with extra arguments is an error, matching how the reminder parser refuses anything but one UUID.

In `setup`, after the existing `--deliver-reminder` branch:

- `headless = parse_sync_argument(&args)?`
- Build `AppState` as normal but with `headless` set.
- When **not** headless, `app.get_webview_window("main").map(|w| w.show())` (needed because of H1).
- When headless, skip `window.show()`, and after `startup::spawn` completes, run the sync-and-notify sequence from Step 4, then `app.handle().exit(0)`.

The headless path still needs the startup work (migrations, starter pack) — reuse `startup::spawn`, or extract `startup::start` and await it directly, which is cleaner headless since there is no splash to feed.

Skip `db::reconcile_reminders_pool` in headless mode: it shells out to `schtasks.exe` once per pending reminder and adds nothing to a scrape.

### Step 6 — The scheduled task (`src-tauri/src/notifications.rs`)

```rust
pub const SYNC_TASK: &str = "\\JobScraper\\JobScraper-Sync";
pub fn sync_task_xml(executable: &str) -> Result<String, String>
```

Same validation as `task_xml` (reject empty paths and embedded newlines, derive `WorkingDirectory` from the parent, `xml_escape` every interpolated value). Differences from the reminder XML:

- Two triggers: a `<LogonTrigger>` and a `<CalendarTrigger>` with `<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay>` plus `<Repetition><Interval>PT4H</Interval><Duration>P1D</Duration></Repetition>`.
- `<Arguments>--sync</Arguments>`.
- Settings: `<StartWhenAvailable>true</StartWhenAvailable>` (so a machine that was off still checks when it wakes), `<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>`, `<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>`, `<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>`, `<ExecutionTimeLimit>PT2H</ExecutionTimeLimit>` (a wedged batch must not linger).
- Keep `InteractiveToken` / `LeastPrivilege` — a toast needs the interactive session, and this must not need elevation.

Create and delete it through the existing `WindowsTaskScheduler::run` helper, with the H5 guard widened.

### Step 7 — User-facing toggle (`src-tauri/src/db.rs` commands + `src/main.tsx`)

**Register nothing automatically.** Creating a Windows scheduled task is persistent system configuration; it must be an explicit opt-in, and the app must be able to undo it.

Two commands, registered in the `invoke_handler` list in `lib.rs`:

- `background_sync_status() -> { enabled: bool }` — asks the scheduler whether `SYNC_TASK` exists.
- `set_background_sync(enabled: bool)` — creates the task from `current_exe()`, or deletes it. Store the intent in `settings` under `sync.background` as well, so the UI can explain a task that Windows removed behind the app's back.

Frontend: a checkbox in the Diagnostics panel next to the purge control, labelled something like *"Check for new jobs in the background"*, with one line of hint text — that Windows will run JobScraper hidden at sign-in and every 4 hours, and show a single notification when new listings appear.

Add task removal to `src-tauri/windows/installer-hooks.nsh` so uninstalling does not leave an orphan task pointing at a deleted executable.

---

## Tests

Follow the existing patterns — `mod reminder_cli_tests` in `lib.rs` for the argument parser, the in-memory pool helpers in `db.rs` for queries.

| Test | Asserts |
|---|---|
| `sync_cli_accepts_only_the_bare_flag` | `--sync` alone parses true; `--sync extra` errors; absent flag is false. Mirrors `reminder_cli_accepts_only_exact_uuid_argument`. |
| `new_jobs_since_ignores_rescraped_and_closed_rows` | Insert one row before the cutoff, one after, one after but `availability='closed'`. Count is 1 and the sample is the right title. |
| `new_jobs_summary_stays_silent_at_zero` | `0` returns `None`; `1` gives singular wording; `7` with 3 samples ends in `"and 4 more"`. |
| `sync_task_xml_scopes_and_escapes` | Contains `--sync`, `LogonTrigger`, `PT4H`, `ExecutionTimeLimit`, and escapes `&` in the executable path. |
| `only_the_exact_sync_task_path_is_managed` | `is_managed_sync_task` accepts `\JobScraper\JobScraper-Sync`, rejects `\JobScraper\JobScraper-Sync2`, `\JobScraper\`, and `\Microsoft\Windows\Update`. |
| `a_fresh_heartbeat_blocks_the_interrupted_run_sweep` | Write a heartbeat, insert a `running` run, call `recover_interrupted_runs`, assert the run and its `job_occurrences` survive. Then age the heartbeat past 5 minutes and assert the sweep cancels it. |

Existing suites must stay green: `pnpm check`, `cargo test`, and the sidecar tests.

## Manual verification

1. Enable the toggle, confirm `schtasks /Query /TN "\JobScraper\JobScraper-Sync"` lists it.
2. `schtasks /Run /TN "\JobScraper\JobScraper-Sync"` with the GUI closed — no window should appear, `Get-Process JobScraper` should show a process that exits on its own, and a toast should arrive if anything new was stored.
3. Run it again immediately — the second run finds nothing new and must stay silent.
4. Start it, then open the GUI mid-batch — the run must not be cancelled and the GUI must not launch a second batch.
5. Disable the toggle, confirm the task is gone.

## Explicitly out of scope

- No system tray and no resident process. A scheduled task covers "runs while I am not looking" with no idle process and far less code. Revisit only if the user wants sub-hour checks or a tray-driven "check now".
- No per-source or keyword-specific alerting — the scrape-time title filter already decides what is stored, so everything new is by definition something the user asked to keep.
- No notification click-through to the app.

## One thing to verify early

The toast is shown by a process that exits moments later. The existing `--deliver-reminder` path does exactly this and is shipped, so the pattern is presumed sound — but confirm the toast actually renders and persists in the Action Center before building the rest on it. If it does not, hold the process open for a few seconds after `show()` before `exit(0)`.
