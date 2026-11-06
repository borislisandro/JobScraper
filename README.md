# JobScraper

Local-first Windows desktop app that collects jobs from company career sites, filters them, and tracks your applications.

Not an auto-apply bot: it opens the original vacancy in your browser, records only what you confirm, and never submits an application for you.

## Features

**Collect** — Scrapes every enabled source on launch, or hidden at sign-in and every 4 hours via Task Scheduler, with one Windows notification per run that finds something new. Incremental checks keep updates light; descriptions are fetched only when a job is opened. Results stream into the local database as they arrive, so cancelled or partially failed runs still keep what they found. A vacancy is marked possibly closed after one full run misses it, closed after two.

**Search** — Filter by source, title, listing text, posting age, country, application status, and closed status. Sort by discovery or publish date, page through results, and open a reader view with description, work mode, seniority, salary, and status. Saved and applied jobs survive their source being disabled or deleted.

**Sources** — Add any of 164 prepared companies in one click, or paste a careers URL and let the detector find the ATS, JSON endpoint, feed, or repeating HTML. JavaScript-only pages render through Microsoft Edge. Test a source before trusting it, configure CSS/XPath/JSON/feed/ATS/Playwright adapters by hand, capture a browser session for sites needing login, and filter titles during scraping so unwanted roles are never stored.

**Applications** — Board with Saved, Applied, Screening, Interviewing, Offer, Accepted, Rejected, and Withdrawn stages. Stores recruiter details, notes with timestamps, an event timeline, and immutable document snapshots up to 20 MB. Schedules interview reminders (24 h and 1 h before) and ghosting reminders.

**Diagnostics** — Local health view for database, sources, listings, and sidecar; last 200 activity entries; scrape medians, p95, slowest phases, retries, and failures, exportable as JSON.

## Supported sites

240 companies ship with verified settings in [`sidecar/company-catalog.json`](sidecar/company-catalog.json). A new install starts with **no sources**: on the Sources tab, pick a **profile** and the app sets itself up for that kind of job search, or use **Browse companies** to add any of the 240 yourself.

### Profiles

A profile is a named set of catalog companies. Choosing one from the drop-down at the top of the Sources tab:

- adds the profile's companies, switches them on, and reads the ones that have never been read;
- when you switch from one profile to another, switches off the companies the first one turned on (their stored jobs stay) and leaves alone anything you switched on or added yourself, including companies added through Browse companies;
- is remembered, and choosing the same profile again only adds what is new;
- "No profile" forgets the choice and changes no source.

Browse companies keeps working under any profile, so a data-science profile can still follow Arm or Apple.

| Profile | Companies | Covers |
| --- | ---: | --- |
| Semiconductor | 48 | Chipmakers, EDA and chip-IP employers across the US, Europe and the UK, for design verification, DFT, RTL and ASIC roles |
| Data science | 51 | Data and AI employers and Portuguese job boards that hire in Portugal |

**Semiconductor:** AMD, Analog Devices, Apple, Arago, Arm, Arteris, Axiomise, Baya Systems, Broadcom, Cadence, Cirrus Logic, Cisco, Codasip, Dolphin Semiconductor, Ericsson, Fractile, GlobalFoundries, Google, Graphcore, Imagination Technologies, Infineon, Intel, Marvell, MediaTek, Microchip, Micron, Monolithic Power Systems, Nordic Semiconductor, NVIDIA, NXP, Olix, Openchip, Pragmatic Semiconductor, Prophesee, Qualcomm, Racyics, Rambus, Raspberry Pi, Renesas, Riverlane, Semidynamics, Siemens, SiPearl, SK hynix, STMicroelectronics, Synopsys, Tenstorrent, u-blox.

**Data science:** Accenture (Portugal), act digital, Air Apps, Airbus (Portugal), Axians Portugal, BearingPoint Portugal, Bitsight (Portugal), Bosch (Portugal), Celfocus, Critical Software, DataCamp, DBServices Portugal, Devoteam, Emma Sleep, Farfetch (Ashby), Farfetch (Lever), Feedzai, Finom, Flix, GoCardless, Hiire, IG&H Portugal, iLoF, Indicium AI, Inetum, Intermedia, IQVIA (Portugal), ITJobs.pt - AI, Landing.jobs - Artificial Intelligence, Landing.jobs - Data, Landing.jobs - Machine Learning, Loka, Mercedes-Benz.io, Metyis, MobiLab Solutions, Natixis in Portugal, OLX, PagerDuty, Santander (Portugal), SIXT (Portugal), Smartex.ai, Superhuman, Sword Health, Talkdesk, TEKEVER, Thorit, TransPerfect (Unbabel), Ubiwhere, Veeva Systems, Volkswagen Digital Solutions, Zego.

Adding a profile means giving each company a `profiles` entry in the catalog and listing the profile at the top of `company-catalog.json`; `scripts/set-profile.mjs` edits the membership and bumps the catalog version.

The rest of the catalog is mostly Greenhouse (76), Ashby (40), Workday (35), Eightfold (8), Lever (13) and Oracle Recruiting (5) boards.

### Adapter coverage beyond the catalog

Workday · Eightfold · Greenhouse · Ashby · Lever · Workable · BambooHR · Personio · SmartRecruiters · Oracle Recruiting · iCIMS · TalentBrew/Jibe/Radancy · Phenom · RSS/Atom feeds · public JSON APIs (auto-discovered) · server-rendered HTML (inferred CSS selectors, or manual CSS/XPath) · JavaScript-rendered HTML via Playwright + Edge · authenticated boards via encrypted session capture.

Adding a URL is verified, not optimistic: automatic setup saves an adapter only after a probe returns recognizable listing rows. Unsupported pages are reported rather than silently accepted.

"Supported" means the repo has a verified configuration and fixture coverage. Career sites change without notice — check Diagnostics and the source test first when one breaks.

## Stack

Tauri 2 + Rust (persistence, workflow, scheduling, OS integration) · SQLx/SQLite · React 19 + TypeScript + TanStack Query + Vite · bundled Node.js 24 sidecar for all web parsing, talking to Rust over versioned JSON Lines.

```text
React UI  ──Tauri commands/events──▶  Rust service ──JSONL──▶  Node sidecar
                                      SQLite, workflow,        ATS adapters,
                                      reminders, scheduling    HTML/JSON/RSS,
                                                               Playwright/Edge
```

Rust owns persistence and rules; the Node worker owns untrusted web parsing and browser automation. A worker starts only when needed and is killed on cancel or exit.

## Privacy

No account, no cloud, no telemetry. Data lives in `%LOCALAPPDATA%\JobScraper` (dev builds use `JobScraper-dev`). The UI's CSP blocks arbitrary requests; all network access is isolated in the sidecar, which blocks private/loopback/reserved targets, paces requests per origin, honors `Retry-After` and `robots.txt` (overridable per source), and encrypts captured browser sessions with AES-256-GCM keyed through the Windows credential store.

## Install and build

Running a packaged build needs Windows 11 x64 and the Edge WebView2 runtime (normally already installed).

Building needs Node.js 24, pnpm 11, Rust (`x86_64-pc-windows-msvc`), MSVC C++ Build Tools, and Microsoft Edge.

```powershell
pnpm app:dev
pnpm check; pnpm lint; pnpm test; pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
pnpm package:all   # or package:installer / package:portable
```

Packages land in `artifacts/<version>/` with `SHA256SUMS.txt`. Keep the portable ZIP as one folder — `JobScraper.exe` loads the adjacent `sidecar` directory.

### Startup permissions

JobScraper runs as your normal user and uses least-privileged scheduled tasks. Do **not** enable "Run this program as an administrator" — Windows cannot show a UAC prompt during a scheduled launch. If an older admin-created task rejects changes, close the app and run `scripts/repair-startup.ps1` from an elevated PowerShell; it backs up task definitions under `%LOCALAPPDATA%\JobScraper\startup-repair-*` and clears only this executable's `RUNASADMIN` flag.

## Limitations

- Windows only. No cloud sync or multi-device account.
- Never fills or submits application forms.
- Authenticated sites may need manual login or CAPTCHA completion before session capture.
- Some boards omit dates, locations, salaries, or descriptions; missing values stay missing rather than being invented.
- Google Careers pagination is limited by its robots policy unless overridden.
- Backup/restore, analytics, CSV export, and duplicate resolution exist in the Rust backend but have no UI yet.

## More

- `node scripts/verify-company-catalog.mjs` — live per-company checks (opt-in, no jobs stored).
- `node scripts/benchmark-scrape.mjs --fixtures` — offline scrape benchmarks.
