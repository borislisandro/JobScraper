# Handoff: company expansion

Written 2026-09-02 at the end of a Claude Code session, for whoever picks this up next.

## Context

JobScraper is a local Tauri app that reads employers' job boards through a Node sidecar worker and
stores the results in SQLite. A "source" is one employer's board plus an adapter id and a config.
`sidecar/company-catalog.json` is the shipped picker of ready-made sources; `src-tauri/src/db.rs`
seeds a 21-source starter pack from it.

There is a backlog of 269 candidate employers in `docs/company-support-tracker.json`. That file is
the machine-readable source of truth for status; `docs/company-expansion-plan.md` is a generated
view of it and must never be hand-edited (`node scripts/update-company-plan.mjs` regenerates it).

The user is a **digital verification engineer** working in HDL, RTL and ASIC. Two filters follow
from that and both are already applied to the tracker:

- `inScope: false` on 51 rows outside ASIC / digital design / verification / embedded / mixed-signal
  / hardware work (SaaS vendors, cybersecurity software, civil engineering consultancies, chemicals,
  mechanical CAD/PLM). They keep their sequence numbers so the filter is reversible.
- `focus: true` on 14 rows that are the *extreme priority*: employers who actually hire RTL and
  verification engineers and that the catalog does not already cover.

## Goal

Work the 14 focus employers first, then the wider in-scope backlog in priority order. For each one:
discover its real board, configure or implement an adapter, prove it live, and only then admit it to
the catalog. `docs/company-onboarding-runbook.md` is the procedure — read it before starting.

The focus list, in tracker order:

| Seq | Company | Status |
| ---: | --- | --- |
| 15 | Siemens (Siemens EDA — Questa, Veloce) | blocked |
| 94 | Synaptics | queued |
| 95 | Cirrus Logic | queued |
| 96 | Renesas | queued |
| 98 | Nordic Semiconductor | queued |
| 103 | Rambus | queued |
| 104 | CEVA | queued |
| 105 | Imagination Technologies | queued |
| 106 | XMOS | queued |
| 108 | SiPearl | queued |
| 109 | Axelera AI | queued |
| 110 | Etched | queued |
| 136 | Arista Networks | queued |
| 137 | Ciena | queued |

Note that the catalog **already** ships Arm, AMD, NVIDIA, Intel, Qualcomm, Broadcom, Apple, MediaTek,
Marvell, Infineon, STMicroelectronics, NXP, Texas Instruments, Analog Devices, Microchip, Lattice,
Silicon Labs, GlobalFoundries, Micron, SiFive, Astera Labs, Tenstorrent, Lightmatter and SambaNova,
plus Synopsys and Cadence added this session. Do not re-add them.

## Current state

In scope: 218 of 269. **18 supported · 9 blocked · 4 discovering · 1 validating · 186 queued.**

Supported (all with a passing full proof in `docs/company-proofs/`): ASML, ASM International,
Tokyo Electron, Kulicke & Soffa, Onto Innovation, Synopsys, Cadence, Lockheed Martin, Northrop
Grumman, L3Harris, Anduril, Shield AI, AeroVironment, TEKEVER, Rocket Lab, Relativity Space, Stoke
Space, Sierra Space. (PTC is also in the catalog but is marked out of scope — it was proven before
the filter was applied.)

Blocked, each with the observed reason in its tracker `notes`: Applied Materials, Teradyne,
Advantest, DISCO, Veeco, Siemens, RTX, General Dynamics, Leonardo.

Still open: BESI, SCREEN, MathWorks and Blue Origin are `discovering`; BAE Systems is `validating`
(see the open question below).

## The workflow

```bash
# 1. Find the real board. Confirms hosted tokens by calling the platform API for a live job count.
node scripts/detect-board.mjs "Rambus" https://www.rambus.com/careers/

# 2. Write docs/company-candidates/<tracker-id>.json (see the runbook for the shape).

# 3. Prove it live: full traversal, deferred descriptions, warm re-check.
node scripts/verify-company-catalog.mjs --full \
  --candidate docs/company-candidates/<id>.json --report docs/company-proofs/<id>.json

# 4a. Admit a passing proof to the catalog and tracker.
node scripts/admit-company.mjs <id> docs/company-proofs/<id>.json \
  --country US --tags semiconductor,ip --careers <url> --notes "..."

# 4b. Or record a non-admission outcome.
node scripts/mark-company.mjs <id> blocked --notes "what you observed" --next "..."

# 5. Regenerate the plan and run the gates.
node scripts/update-company-plan.mjs
npm test
```

`admit-company.mjs` refuses anything that is not a passing `level: "full"` proof, and copies the
board URL and config out of the proof itself, so the catalog entry cannot drift from its evidence.
`sidecar/starter-pack.contract.test.mjs` enforces that for every supported row.

## Hard-won lessons — read these before trusting any discovery result

1. **"Blocked" is usually wrong.** Every blocked verdict produced by a subagent and then re-checked
   turned out to be a readable board. Anduril was reported as a custom Lambda backend and is plain
   Greenhouse with 2202 openings. AeroVironment was reported as misconfigured and is Workday
   `avav`/`AVAV` with 351. BAE Systems and Thales were reported as API-blocked and both ship the
   `phApp.ddo` payload the existing Cisco reader parses. Always re-verify before recording blocked.

2. **A board that answers is not necessarily the right company.** Two proofs passed against boards
   belonging to other companies: Lever token `blue` (10 data roles in Costa Rica and Eastern Europe)
   is not Blue Origin, and Ashby token `sierra` is Sierra AI, not Sierra Space. The stored `company`
   field is filled from the source name, so it looks correct either way — judge by the job titles and
   locations. Both came from guessing a company's first word as a token; `detect-board.mjs` no longer
   does that. Also watch for "talent community" boards: ASM International's has 2 rows, its real
   Greenhouse board (`asm`) has 444.

3. **Reconcile against the employer's own published count** before accepting a board.

## What changed this session

Only the files below were touched by this work. The tree also contains unrelated pre-existing
changes (`src-tauri/notifications.rs`, `db.rs`, `styles.css`, `presets.ts`, and others) that predate
this session — leave them alone. **Nothing has been committed.**

### Bug fixes in shipped code

- `sidecar/adapters.mjs` — **Workday pagination never terminated on some tenants.** PTC's Workday
  answers an out-of-range offset with page one again, so "a full page came back" was not proof the
  board continued: a 180-job board read 500 pages and 10,000 rows. Paging now ends on the published
  total, carried forward because that tenant reports `total: 0` on later pages. Affects all 20+
  Workday sources. Test in `adapters.test.mjs`.
- `sidecar/worker.mjs` — **a feed read claimed to be a complete board.** TEKEVER's careers page
  counts 123 openings; its RSS feed carries the most recent 100. The generic `rss` adapter reported
  reaching the end of the feed as a finished board, which would have reconciled the missing 23 to
  closed — a direct violation of the project rule that capped reads never authorize closing jobs. A
  feed now cannot certify a whole board unless the source sets `feedIsWholeBoard`. Test in
  `platform.execution.test.mjs`.
- `sidecar/worker.mjs` — `toLocation` did not know schema.org's `addressLocality` / `addressRegion` /
  `addressCountry`, so any board publishing a JobPosting graph stored no location at all.

### Adapters

- **`radancy` reader** (Radancy / TalentBrew), serving `synopsys` and `l3harris`. Reads the paged
  search page and reconciles against the page's own `data-total-job-results` and `data-total-pages`,
  so reaching the last page is not by itself treated as having read every counted vacancy. Cards are
  identified by the results container and the job id on the link, because tenants theme their own
  class names. Arm turned out to be the same platform, so its detail reader is now shared
  (`talentBrewDetail`), and the enrichment loop Arm and u-blox each had a copy of is one function
  (`enrichListings`).
- **Phenom family**: the Cisco-specific reader became `phenom`, now also serving `bae-systems` and
  `thales` as config entries against the same proven code.

### Policy change requested by the user

**robots.txt enforcement is off everywhere.** This matches what `install_starter_pack` already did
(it seeded every catalog source with `robots_override = 1`); only the verification harness was
enforcing it, which meant proofs were certifying a stricter run than the app actually performs.
Changed: `scripts/worker-proof.mjs` no longer refuses an override, `verify-company-catalog.mjs` sets
it, the catalog picker in `src/main.tsx` sets it, all candidate files carry it, and the runbook says
so. The URL guard (SSRF), per-origin pacing and `allowPrivateNetwork: false` are all unchanged and
still required.

### New tooling

- `scripts/detect-board.mjs` — resolves a careers URL to its actual platform.
- `scripts/admit-company.mjs`, `scripts/mark-company.mjs` — the two ways a company leaves the queue.
- `docs/company-onboarding-runbook.md` — the per-company procedure, written for a subagent.
- `scripts/update-company-plan.mjs` — now renders the focus list and the out-of-scope appendix.

### Test changes worth knowing about

- The Workday fix removes one request per board, so `platform.execution.test.mjs` asserts exactly one
  listing request for the Workday fixture rather than "at least two".
- `starter-pack.contract.test.mjs` no longer asserts `robotsOverride === false`; it asserts
  `allowPrivateNetwork !== true` (an omitted flag is off, which is what the worker tests), and it
  skips `requestFor` for generic adapters (`json`, `rss`, `static-css`, …) that fetch their own base
  URL instead of building a platform request.

Gates run green at handoff: 77 sidecar tests, 44 vitest tests, `tsc -b` clean. Rust tests were not
run this session.

## Open questions for the next session

1. **BAE Systems (`validating`).** Its Phenom board reaches the published total of 1855, but only
   1440 distinct rows survive: the tenant lists a requisition once per location and those collapse to
   one stored job. The traversal is complete; the proof gate counts the collapsed duplicates as rows
   that went missing (`payload.discovered !== jobs.length`). Apple's board in the catalog behaves the
   same way. Decide whether the gate should accept identity-deduplicated rows before admitting BAE
   and Thales. Neither is on the focus list, so this is not urgent.
2. **Applied Materials (`blocked`).** Its Eightfold index serves a few promoted rows twice, and not
   deterministically — one raw traversal read all 1943 distinct rows cleanly, two worker proofs each
   ended 2–3 short. Retry, or read the board in slices whose pages carry no promoted rows.
3. **Avature** (Siemens, focus list) publishes only "999+" as its size and ignores every paging
   parameter tried. Finding how that build pages would unlock Siemens and several other employers.
4. **Teamtailor** covers other queued EU employers. TEKEVER is read through the generic `json`
   adapter against Teamtailor's JSON Feed (`/jobs.json`, `itemsPath: items`, `page`/`per_page`), so a
   dedicated adapter may not be needed — try that config first.
