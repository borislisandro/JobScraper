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

190 companies ship with verified settings in [`sidecar/company-catalog.json`](sidecar/company-catalog.json) — browse and enable them from the Sources page. Fresh installs seed only the semiconductor starter list below (49 entries), and every one of them starts switched off until you enable it or accept the installer's opt-in.

**Original starters**

| Employer | Careers site | Adapter |
| --- | --- | --- |
| Microchip | `wd5.myworkdaysite.com` | Workday |
| Analog Devices | `analogdevices.wd1.myworkdayjobs.com` | Workday |
| Broadcom | `broadcom.wd1.myworkdayjobs.com` | Workday |
| Intel | `intel.wd1.myworkdayjobs.com` | Workday |
| Marvell | `marvell.wd1.myworkdayjobs.com` | Workday |
| NVIDIA | `nvidia.wd5.myworkdayjobs.com` | Workday (facet splitting) |
| Micron | `micron.wd1.myworkdayjobs.com` | Workday |
| NXP | `nxp.wd3.myworkdayjobs.com` | Workday |
| STMicroelectronics | `stmicroelectronics.eightfold.ai` | Eightfold legacy API |
| GlobalFoundries | `careers.gf.com` | Eightfold PCSX API |
| Qualcomm | `careers.qualcomm.com` | Eightfold PCSX API |
| Ericsson | `jobs.ericsson.com` | Eightfold PCSX API |
| Apple | `jobs.apple.com` | Apple search API |
| Arm | `careers.arm.com` | Employer-specific |
| AMD | `careers.amd.com` | Employer-specific |
| Cisco | `careers.cisco.com` | Employer-specific |
| Google | `google.com/about/careers` | Employer-specific |
| MediaTek | `careers.mediatek.com` | Employer-specific |
| ASML | `asml.com/en/careers/find-your-job` | Sitecore search API |
| u-blox | `u-blox.com/en/job-openings` | Algolia API |
| SK hynix | `talent.skhynix.com` | SK Careers + Greenhouse |

**Added to the default list for design-verification, DFT and RTL roles** (chosen from live counts of those titles in Portugal, Spain, France, the Netherlands, Ireland, Austria, Germany, the UK and Switzerland)

| Employer | Careers site | Adapter |
| --- | --- | --- |
| Synopsys | `careers.synopsys.com` | Employer-specific |
| Infineon | `jobs.infineon.com` | Eightfold |
| Graphcore | `job-boards.greenhouse.io` | Greenhouse |
| Cirrus Logic | `jobs.eu.lever.co` | Lever |
| Tenstorrent | `job-boards.greenhouse.io` | Greenhouse |
| Siemens | `jobs.sw.siemens.com` | Employer-specific |
| Monolithic Power Systems | `monolithicpower.wd12.myworkdayjobs.com` | Workday |
| Rambus | `careers-rambus.icims.com` | Employer-specific |
| Imagination Technologies | `careers.pageuppeople.com` | Public JSON API |
| Renesas | `jobs.renesas.com` | Employer-specific |
| Nordic Semiconductor | `careers.nordicsemi.com` | Teamtailor JSON feed |
| SiPearl | `careers.sipearl.com` | Teamtailor JSON feed |
| Cadence | `cadence.wd1.myworkdayjobs.com` | Workday |
| Semidynamics | `careers.semidynamics.com` | Teamtailor JSON feed |
| Fractile | `jobs.ashbyhq.com` | Ashby |
| Arago | `jobs.ashbyhq.com` | Ashby |
| Baya Systems | `job-boards.greenhouse.io` | Greenhouse |
| Riverlane | `job-boards.greenhouse.io` | Greenhouse |
| Olix | `jobs.ashbyhq.com` | Ashby |
| Pragmatic Semiconductor | `talent.pragmaticsemi.com` | Teamtailor JSON feed |
| Dolphin Semiconductor | `careers.dolphin-semiconductor.com` | Teamtailor JSON feed |
| Prophesee | `careers.prophesee.ai` | Teamtailor JSON feed |
| Openchip | `openchip.factorialhr.com` | Employer-specific |
| Codasip | `codasip.bamboohr.com` | BambooHR |
| Raspberry Pi | `apply.workable.com` | Workable |
| Axiomise | `axiomise.jobs.personio.de` | Personio |
| Racyics | `racyics.de` | Employer-specific |
| Arteris | `jobs.jobvite.com` | Employer-specific |

Also in the catalog, not in the default list because their boards are mostly non-chip roles: IonQ, Teledyne, Philips, Bosch, CERN, plus CSEM, Elmos, Kalray, Quantum Motion, SEMRON and Universal Quantum. Employers whose careers sites cannot be read yet (onsemi, Melexis, imec, Rohde & Schwarz, ams OSRAM and others) are recorded as blocked in [`docs/company-support-tracker.json`](docs/company-support-tracker.json) with what was found.

The rest of the catalog is mostly Greenhouse (68), Ashby (35), Workday (30), Eightfold (8), Lever (6), and Oracle Recruiting (5) boards — Infineon, Texas Instruments, KLA, Lam Research, Renesas, Siemens, Synopsys, Cerebras, Tenstorrent and more.

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
