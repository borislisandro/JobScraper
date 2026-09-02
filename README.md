# JobScraper

JobScraper is a local-first Windows desktop application for collecting jobs from company career sites, filtering the results, and managing the complete application process.

It is not an auto-apply bot. JobScraper opens the original vacancy in your browser, records only what you confirm, and never submits an application on your behalf.

## What JobScraper does

### Collect and maintain job listings

- Reads every enabled source automatically when the application opens.
- Can run hidden at sign-in and every four hours through Windows Task Scheduler.
- Shows one Windows notification when a background run finds new listings.
- Uses incremental checks to avoid unnecessary full reads while periodically forcing a complete refresh.
- Streams accepted listings into the local database while a source is still being read, so useful work survives cancellation or a later page failure.
- Downloads full descriptions only when a job is opened. This keeps regular updates faster and lighter.
- Marks a vacancy as possibly closed after one complete read misses it and closed after two complete misses. Partial or failed reads never close jobs.
- Supports cancellation, retries, partial results, per-source status, and full re-scrubbing.

### Search and review jobs

- Filter by source, title, any stored listing text, posting age, country, application status, and closed status.
- Select multiple countries and optionally include jobs whose country cannot be determined.
- Sort by discovery time or published date.
- Page through large result sets in batches of 200.
- Open a focused job reader with description, work mode, seniority, salary, source, and status.
- Save a vacancy for later or open the original posting to apply.
- Keep saved and applied jobs even when their source is switched off or removed.

### Manage sources

- Add a source by pasting its careers-page URL.
- Detect known ATS platforms, public JSON endpoints, feeds, repeated HTML listings, and supported employer-specific boards.
- Render JavaScript-only pages through Microsoft Edge when a direct HTTP read is insufficient.
- Test a source before relying on it and inspect normalized preview rows.
- Enable, disable, edit, or delete sources independently.
- Configure CSS, XPath, JSON, feed, ATS, and Playwright adapters manually when needed.
- Capture an authenticated browser session for sites that require login or CAPTCHA completion.
- Apply an optional title filter during scraping so unwanted roles are never stored.
- Inspect warnings, failures, redirects, pagination status, and live worker progress in the run log.

### Track applications

- Move applications through Saved, Applied, Screening, Interviewing, Offer, Accepted, Rejected, and Withdrawn stages.
- Drag cards between legal workflow stages on the application board.
- Confirm whether an application was actually submitted after JobScraper opens the posting.
- Store recruiter details, source attribution, rejection information, and withdrawal reasons.
- Add editable notes with timestamps and an event timeline.
- Attach immutable local document snapshots up to 20 MB and export them later.
- Schedule interviews and Windows reminders for 24 hours and 1 hour before each interview.
- Configure ghosting reminders when an application receives no response.

### Diagnose and operate the app

- View database, source, listing, sidecar, and restore health locally.
- Inspect the last 200 activity entries.
- Compare scrape medians, p95 timings, slowest phases, request types, retries, and failures.
- Copy raw diagnostic and performance reports as JSON.
- Start JobScraper automatically at sign-in.
- Close the window to the system tray and reopen the existing single instance from the tray or shortcut.
- Delete stored listings while preserving sources, settings, and jobs linked to applications.

## Currently supported websites

The bundled starter pack contains 19 live-configured employer sources. The installer asks whether to enable them; they remain editable from the Sources page either way.

| Employer | Careers site | Adapter |
| --- | --- | --- |
| Microchip | `wd5.myworkdaysite.com` | Workday |
| Analog Devices | `analogdevices.wd1.myworkdayjobs.com` | Workday |
| Broadcom | `broadcom.wd1.myworkdayjobs.com` | Workday |
| Intel | `intel.wd1.myworkdayjobs.com` | Workday |
| Marvell | `marvell.wd1.myworkdayjobs.com` | Workday |
| NVIDIA | `nvidia.wd5.myworkdayjobs.com` | Workday with large-board facet splitting |
| Micron | `micron.wd1.myworkdayjobs.com` | Workday |
| NXP | `nxp.wd3.myworkdayjobs.com` | Workday |
| STMicroelectronics | `stmicroelectronics.eightfold.ai` | Eightfold legacy API |
| GlobalFoundries | `careers.gf.com` | Eightfold PCSX API |
| Qualcomm | `careers.qualcomm.com` | Eightfold PCSX API |
| Apple | `jobs.apple.com` | Apple search API |
| Arm | `careers.arm.com` | Dedicated employer adapter |
| AMD | `careers.amd.com` | Dedicated employer adapter |
| Cisco | `careers.cisco.com` | Dedicated employer adapter |
| Google | `google.com/about/careers` | Dedicated employer adapter |
| MediaTek | `careers.mediatek.com` | Dedicated employer adapter |
| u-blox | `u-blox.com/en/job-openings` | Algolia listing API plus vacancy details |
| SK hynix | `talent.skhynix.com` | Combined SK Careers and SK hynix America Greenhouse feeds |

The starter pack also contains a reference-only Marvell careers URL. Reference sources are never scraped.

Career sites change without notice. "Supported" means the repository contains a dedicated or verified configuration and automated fixture coverage; it does not guarantee that a third-party site will never change, rate-limit, block, or require authentication.

## Reusable adapter coverage

JobScraper is not limited to the starter pack. The source detector and advanced source editor support:

| Source type | Current support |
| --- | --- |
| Workday | Direct CXS JSON API, pagination, details, large-board splitting, and optional browser fallback |
| Eightfold | Current PCSX and legacy APIs, offset or cursor pagination, details, and saved sessions |
| iCIMS | Storefront detection, paged JSON configuration, and details |
| TalentBrew / Jibe / Radancy | Configurable paged listing API and details |
| Phenom | Configurable paged listing API and details |
| RSS and Atom | Direct feed parsing |
| Public JSON APIs | Automatic endpoint discovery on many JavaScript sites, inferred job arrays, configurable field mapping, and pagination |
| Server-rendered HTML | Automatic CSS selector inference plus manual CSS or XPath configuration |
| JavaScript-rendered HTML | Headless or headed Microsoft Edge through Playwright Core |
| Authenticated boards | Per-source browser-session capture and encrypted session reuse |

Adding a URL is intentionally optimistic but verified: automatic setup saves a working adapter only after a probe produces recognizable listing rows. Unsupported pages are reported instead of being silently accepted as an empty source.

## Technology stack

### Desktop and backend

- [Tauri 2](https://tauri.app/) for the native Windows shell, IPC boundary, tray icon, single-instance behavior, dialogs, notifications, and packaging.
- Rust 2021 for startup, source orchestration, persistence, application workflows, reminders, backup foundations, and operating-system integration.
- Tokio for asynchronous worker and process management.
- SQLx with SQLite for local persistence, migrations, foreign keys, WAL mode, and transactional updates.
- Windows Task Scheduler for sign-in launch, four-hour background synchronization, interview reminders, and ghosting reminders.

### Frontend

- React 19 and TypeScript.
- TanStack Query for server-state caching and refresh behavior.
- dnd-kit for the application workflow board.
- Vite for development and production builds.
- Vitest, jsdom, and ESLint for frontend verification.

### Scraping sidecar

- A bundled Node.js 24 runtime communicates with Rust over versioned JSON Lines on stdin/stdout.
- Native `fetch` handles guarded HTTP requests, redirects, pacing, retries, and `Retry-After`.
- Cheerio parses HTML.
- `fast-xml-parser` parses RSS and Atom feeds.
- `xpath` and `@xmldom/xmldom` support XPath sources.
- Playwright Core drives the installed Microsoft Edge browser for rendering and session capture.

Release packages include the Node runtime and sidecar dependencies. Users do not need Node.js installed.

## Architecture

```text
React + TypeScript UI
        |
        | Tauri commands and events
        v
Rust application service
  |-- SQLite database and migrations
  |-- application workflow and reminders
  |-- background scheduling and notifications
  |-- source orchestration and diagnostics
        |
        | versioned JSONL protocol
        v
Bundled Node.js worker
  |-- direct ATS and employer adapters
  |-- HTML, JSON, RSS, and XPath parsers
  `-- Microsoft Edge / Playwright fallback
```

Rust owns persistence and workflow rules. The Node worker owns untrusted web parsing and browser automation. A scrape worker starts only when a test, update, detail request, session capture, or scheduled synchronization needs it, and is terminated on cancellation or application exit.

## Local data, privacy, and security

- Release data is stored under `%LOCALAPPDATA%\JobScraper`.
- Development data is isolated under `%LOCALAPPDATA%\JobScraper-dev`.
- There is no account, cloud database, analytics SDK, or application telemetry.
- The application frontend is prevented by its CSP from making arbitrary internet requests. Network scraping is isolated in the sidecar.
- URLs, redirects, DNS results, and browser navigations are checked to block private, loopback, reserved, and non-HTTP targets unless local-network access is explicitly enabled for a source.
- Requests are paced per origin and honor capped `Retry-After` delays.
- `robots.txt` policy is inspected and any override is stored per source.
- Captured browser storage is encrypted with AES-256-GCM. Its key is stored through the Windows credential store, and session values are never displayed in the UI.
- Attached application documents remain local.
- JobScraper application code makes no startup HTTP request. The embedded Microsoft WebView2 runtime can perform Microsoft-controlled runtime traffic outside JobScraper's code path.

## Requirements

### Running a packaged build

- Windows 11 x64.
- Microsoft Edge WebView2 Runtime. Current Windows installations normally include it.

### Development and release builds

- Windows x64.
- Node.js 24.
- pnpm 11.
- Rust with the `x86_64-pc-windows-msvc` host.
- Microsoft C++ Build Tools required by the Rust MSVC toolchain.
- Microsoft Edge for rendered-page and session-capture support.

## Development

The development launcher validates tool versions, installs the locked workspace dependencies, prepares the isolated sidecar, and starts Tauri:

```powershell
pnpm app:dev
```

Useful verification commands:

```powershell
pnpm check
pnpm lint
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
```

Benchmark the same worker protocol used by the application without writing jobs or source state:

```powershell
node scripts/benchmark-scrape.mjs --fixtures
node scripts/benchmark-scrape.mjs
```

The first command uses deterministic local fixtures. The second also checks enabled sources and exercises warm incremental updates against the development database in read-only mode.

## Packaging

Build a current-user NSIS installer, a portable ZIP, or both:

```powershell
pnpm package:installer
pnpm package:portable
pnpm package:all
```

The release pipeline runs TypeScript, frontend, sidecar, Rust formatting, and Rust test gates before packaging. Versioned outputs and `SHA256SUMS.txt` are written to `artifacts/<version>/`.

The portable ZIP must be kept as one folder because `JobScraper.exe` loads the adjacent `sidecar` directory. Portable and installed builds both keep user data under `%LOCALAPPDATA%\JobScraper`; moving or upgrading the executable does not move the database.

## Current limitations

- Windows is the only supported desktop platform.
- JobScraper never fills or submits application forms.
- There is no cloud sync or multi-device account.
- Authenticated sites can require manual login or CAPTCHA completion before a session can be captured.
- Some boards expose incomplete dates, locations, salaries, or descriptions; JobScraper preserves missing values rather than inventing them.
- Google Careers pagination is restricted by its published robots policy unless the user enables an override.
- Backup/restore archives, analytics, CSV/data export, and duplicate-resolution services exist in the Rust backend but do not yet have complete user-facing workflows.
- Site support depends on third-party HTML and API contracts. Diagnostics and source tests are the first place to inspect after a careers-site change.

## Roadmap

The repository does not promise a fixed list of future employers. A site is added to the supported list only after its listing contract is understood, implemented, and covered by fixtures. Planned direction is:

- Expand the verified starter pack with more semiconductor, hardware, and technology employers.
- Add dedicated adapters for important boards that generic HTML or endpoint detection cannot read reliably.
- Broaden ATS tenant coverage and improve JavaScript-only, authenticated, and multi-board source handling.
- Expose the existing backup/restore, analytics, CSV/data export, and duplicate-review backend services in the desktop UI.
- Add more granular alert controls after the current single-summary background notification workflow is proven in regular use.
- Continue improving adapter diagnostics, change detection, pagination completeness, and recovery from third-party site changes.

Until a company appears in the current-support table, treat it as discoverable rather than guaranteed: paste its careers URL into **Sources > Add source**, let JobScraper probe it, and use the generated preview to verify the result.
