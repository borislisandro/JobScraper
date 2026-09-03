# Engineering company expansion plan

Updated: 2026-09-02. **218 in-scope employers** of 269 candidates. 30 supported · 10 blocked · 4 discovering · 1 validating · 173 queued.

## Extreme priority: the focus list

14 employers, worked before anything else. These are the ones that hire digital design and
verification engineers — RTL, testbenches, silicon bring-up — and that the catalog does not already
cover. The catalog already ships Arm, AMD, NVIDIA, Intel, Qualcomm, Broadcom, Apple, MediaTek,
Marvell, Infineon, STMicroelectronics, NXP, Texas Instruments, Analog Devices, Microchip, Lattice,
Silicon Labs, GlobalFoundries, Micron, SiFive, Astera Labs, Tenstorrent, Lightmatter, SambaNova,
Synopsys and Cadence, so none of them appear below.

| Order | Company | Category | Status | Why it is on this list |
| ---: | --- | --- | --- | --- |
| 15 | Siemens | EDA and engineering software | **supported** | Siemens EDA (Questa, Veloce) builds the verification tools themselves. |
| 94 | Synaptics | Semiconductors | **blocked** | Touch, audio and edge-AI SoCs. |
| 95 | Cirrus Logic | Semiconductors | **supported** | Mixed-signal SoCs with substantial digital design and verification teams. |
| 96 | Renesas | Semiconductors | **supported** | MCU and SoC families; RTL design and verification across several sites. |
| 98 | Nordic Semiconductor | Semiconductors | **supported** | Wireless SoCs designed in-house, with a dedicated verification org. |
| 103 | Rambus | Semiconductors | **supported** | Memory and security IP; RTL design and verification is the core discipline. |
| 104 | CEVA | Semiconductors | **supported** | DSP and connectivity IP houses hire almost exclusively RTL and verification engineers. |
| 105 | Imagination Technologies | Semiconductors | **supported** | GPU and AI IP; large verification organisation. |
| 106 | XMOS | Semiconductors | **blocked** | xcore RISC-V silicon designed in-house. |
| 108 | SiPearl | Semiconductors | **supported** | European HPC CPU; a from-scratch ASIC programme. |
| 109 | Axelera AI | Semiconductors | **supported** | AI inference accelerator; RTL and verification for its own silicon. |
| 110 | Etched | Semiconductors | **supported** | Transformer ASIC startup; small team, all silicon. |
| 136 | Arista Networks | Networking | **supported** | Designs its own switch silicon alongside the OS. |
| 137 | Ciena | Networking | **supported** | Coherent optical DSP ASICs. |

## Objective and working order

Work the 14 focus employers first: they are the ones hiring digital design and verification engineers (HDL, RTL, ASIC) and are not in the catalog yet. Everything after them is the wider ASIC/embedded/hardware backlog, in priority order, with a complete live worker proof before each catalog admission. Rows outside that discipline stay recorded as out of scope rather than deleted.

This is the backlog from the requested missing-company list, filtered to employers that do ASIC, digital design, digital verification, embedded software, mixed-signal or hardware engineering. A row is a candidate employer or brand, not a promise that its board is currently readable. The 51 rows outside that discipline are kept at the bottom of this file rather than deleted. The machine-readable source of status is [company-support-tracker.json](company-support-tracker.json). Regenerate this view with `node scripts/update-company-plan.mjs`.

| Priority | Scope | In scope | Completion condition |
| --- | --- | ---: | --- |
| P1 | Semiconductor equipment, EDA and engineering software | 16 of 21 | Process rows 1–17 in order |
| P2 | Defense, Aviation, Space | 49 of 49 | Process rows 22–70 in order |
| P3 | Industrial automation, Test and measurement | 20 of 20 | Process rows 71–90 in order |
| P4 | Semiconductors, Hardware and electronics, Photonics, Networking, Automotive, Robotics | 78 of 78 | Process rows 91–168 in order |
| P5 | Software and cloud, Cybersecurity, Quantum, Energy, Medical devices | 33 of 60 | Process rows 169–227 in order |
| P6 | Engineering services, Rail and heavy machinery | 22 of 41 | Process rows 229–269 in order |

Work on one company at a time within each batch. Batches run in parallel as subagents following [company-onboarding-runbook.md](company-onboarding-runbook.md); each owns only its own candidate and proof files. Shared adapters, the catalog and the tracker have one integration owner, who reviews evidence and admits validated entries with `node scripts/admit-company.mjs`. Later phases stay queued until the current one is assessed.

Reuse a proven platform adapter when its live response matches; otherwise implement the smallest company adapter at the existing guarded request seam. Shared-family work is justified when the observed contract serves multiple queued employers. Do not guess tenants, tokens, endpoints, or ownership from a similar company name.

## Status definitions

- **queued**: not examined in this expansion; only the candidate name is known.
- **discovering**: identifying the official board, regional scope, robots policy, and actual API or HTML contract.
- **implementing**: configuration or adapter changes are in progress; the company is not supported yet.
- **validating**: offline regression checks and live worker proof are in progress.
- **supported**: the catalog entry is present and all acceptance gates below have saved evidence.
- **blocked**: a concrete policy, access, missing data, or technical obstacle remains. Record the observed failure and next step, then continue to the next queued row. Revisit after the current priority group.
- **covered_by_parent**: an existing parent board demonstrably includes the brand's jobs; link its source and proof. A corporate acquisition alone is not coverage evidence.

## Per-company implementation and acceptance

1. **Discover and identify.** Follow the employer's official careers page to its actual board. Record both the official page and selected board, relevant regions, and any separate subsidiary boards. Confirm company identity against employer-owned content.
2. **Check access.** Robots enforcement is off by choice for this tool, matching what `install_starter_pack` already seeds, so a `Disallow` does not stop a read. Private-network protection, per-origin pacing and the URL guard stay on, and no authentication bypass or guessed private endpoint is acceptable. Record what the host's robots policy says anyway: it usually names the endpoint that serves the vacancies.
3. **Configure or implement.** Tune only settings justified by observed responses: page size, paging limits, detail strategy, and pacing. Add a custom adapter when required, including registry, detection/editor support, normalization and packaging integration as applicable.
4. **Add regression proof.** Exercise the observed listing/detail shape, pagination termination, stable identity, malformed/partial reads, and any new branch that could falsely mark a board complete. Keep company-specific fixtures compact and free of unnecessary job content.
5. **Full cold read.** Run the real worker's `scrape_source` protocol over the live board with no known hashes or title filters. Require non-empty normalized jobs, valid HTTP(S) application links, unique identities and listing hashes, and `complete: true`. Reconcile a published exact total with discovered rows. A first-page preview is insufficient.
6. **Description and incremental proof.** Verify real description content, fetching deferred details for representative first/middle/last jobs where available. Run a warm change check using the cold read's hashes and record freshness/count behavior. A moving board is investigated rather than mislabeled as an adapter failure.
7. **Publish locally.** Only after live proof succeeds, add the ready-to-use catalog configuration with a stable ID and verification reference. Leave starter membership unchanged. Prove catalog request construction and run affected sidecar, Rust and frontend gates.
8. **Update status.** Save the compact proof report (source/config, timestamp, worker code hashes, requests, counts, completion, samples, details, and warm check), set the row to supported, regenerate this plan, then proceed to the next sequence number.

## Safety and completion rules

Partial, failed, capped, malformed, or empty reads never authorize closing stored jobs. Preserve current per-origin pacing, cancellation, SSRF defenses, and starter enable/delete choices. A browser-rendered fallback must preserve the same access policy and must prove its pagination before it is accepted.

Do not add 269 disabled guesses to the app. Pending and blocked work stays in this tracker; the picker contains only proven configurations. A blocked company remains unfinished until access changes or a verified public alternative is implemented. The overall objective is complete only when every row is supported or demonstrably covered by a parent board; unresolved blocked rows must remain visible.

Existing 133 supported employers retain their earlier verification level. The stronger full-read gate applies to new entries added by this expansion. Live proof certifies the recorded run, not permanent availability of a third-party board.

## Reproducing acceptance

Before catalog admission, save the candidate as a worker source JSON, then run `node scripts/verify-company-catalog.mjs --full --candidate path/to/source.json --report docs/company-proofs/company.json`. The command exits unsuccessfully on incomplete traversal, duplicate identities, missing descriptions, count mismatch or unstable warm results. Inspect the saved samples and warnings before admission.

After admission, rerun by exact catalog name: `node scripts/verify-company-catalog.mjs --full "ASML" --report docs/company-proofs/asml.json`. Without `--full`, the command is only a first-page preview and does not satisfy acceptance. The catalog contract test requires supported expansion rows to have full evidence matching the shipped configuration.

## Company-by-company queue

| Order | Priority | Company | Category | Status | Adapter | Evidence | Next step / notes |
| ---: | --- | --- | --- | --- | --- | --- | --- |
| 1 | P1 | ASML | Semiconductor equipment | **supported** | asml | [proof](company-proofs/asml.json) | Complete; revalidate if ASML changes its board. Global ASML Sitecore search: all 591 unique vacancies, six pages, full inline descriptions; warm check found zero fresh rows. Custom adapter rejects duplicate pages and changing totals. |
| 2 | P1 | Applied Materials | Semiconductor equipment | **blocked** | eightfold | [proof](company-proofs/applied-materials-initial.json) | Retry the full proof later, or read the board in filtered slices whose pages do not carry promoted rows. Eightfold PCSX board, domain appliedmaterials.com, no sort_by (the UI's own default order; 'relevance' is the only other accepted value). The board is 1943 distinct openings and one raw traversal read all 1943 with no repeat, but two full worker proofs each saw a few promoted rows served again on later pages, ending 2-3 rows short. Repeats are not deterministic, so a run cannot be trusted to have seen everything and the read is correctly refused as incomplete. |
| 3 | P1 | ASM International | Semiconductor equipment | **supported** | greenhouse | [proof](company-proofs/asm-international.json) | Complete; revalidate if this employer changes its board. Greenhouse token asm, the board embedded behind asm.com/open-vacancies (444 openings, matching ASM's own published count). The separate talentcommunityasm board is a talent pool, not vacancies, and is not this source. |
| 4 | P1 | BESI | Semiconductor equipment | **discovering** | — | — | Small company adapter: table parser plus the shared enrichListings detail pass. Deferred because it is one employer with 9 openings; the contract above is what it needs. Board is besi.kandidatenportal.eu/Jobs and it is the whole board: besi.com/careers/jobs lists exactly the same 9 openings, all linking there. Server-rendered, no XHR, no paging, and it states its own size ('Derzeit gibt es 9 offene Stellen'). Rows are table markup: tbody tr, title and link in .job-title a, td.location, td.date in DD.MM.YYYY. Descriptions live only on /Job/<id>, so the generic static-css adapter cannot satisfy the description gate. |
| 5 | P1 | Teradyne | Semiconductor equipment | **blocked** | — | — | Read the published sitemap for the job URL set, or find the paging parameter this older theme honours. Radancy/TalentBrew board at jobs.teradyne.com, but an older theme than the one the Synopsys adapter reads: no #search-results data attributes, no search-results-list__list-item cards, and /search-jobs ignores ?p= (page 2 returns page 1 byte-for-byte). Its JSON results endpoint sits under /services/, which its own robots.txt disallows. A sitemap.xml listing every /job/ URL is published and allowed. |
| 6 | P1 | Advantest | Semiconductor equipment | **blocked** | — | — | Revisit if ADP publishes a readable public listing endpoint; do not accept the consent gate automatically. Careers run on ADP myjobs (myjobs.adp.com/advantestcareers). The site is an Angular app behind Akamai bot protection, /robots.txt returns the app shell rather than a policy, and the listing sits behind a privacy-consent gate; the public API refuses without an orgoid header. Europe is a separate site at advantest-career.de. |
| 7 | P1 | Tokyo Electron | Semiconductor equipment | **supported** | workday | [proof](company-proofs/tokyo-electron.json) | Complete; revalidate if this employer changes its board. Workday tenant tel, site TEL-Careers. Full cold read of the global board with descriptions from the Workday detail endpoint; warm check clean. |
| 8 | P1 | SCREEN | Semiconductor equipment | **discovering** | — | — | Find SCREEN Holdings' English careers board before assessing the Japanese one. Reported as screen-recruit-fresh.snar.jp, a Japanese graduate-recruitment platform. Not independently verified, and the global/English board was not located. |
| 9 | P1 | DISCO | Semiconductor equipment | **blocked** | — | — | Recheck for a global careers board; there is nothing to read today. No public vacancy board found on the global or Japanese site; recruitment appears to run through Japanese graduate-hiring channels rather than a listed board. |
| 10 | P1 | Kulicke & Soffa | Semiconductor equipment | **supported** | oracle | [proof](company-proofs/kulicke-soffa.json) | Complete; revalidate if this employer changes its board. Oracle Recruiting Cloud: pod etyy.fa.ap2.oraclecloud.com, site CX_1. Whole board in one page (62 openings), descriptions from the Oracle detail endpoint; warm check clean. |
| 11 | P1 | Onto Innovation | Semiconductor equipment | **supported** | workday | [proof](company-proofs/onto-innovation.json) | Complete; revalidate if this employer changes its board. Workday tenant onto, site ONTO_Careers. Full cold read matched the published total of 186; warm check clean. |
| 12 | P1 | Veeco | Semiconductor equipment | **blocked** | — | — | Same as Teradyne: read the sitemap, or identify this theme's paging parameter. Same older Radancy/TalentBrew theme as Teradyne at careers.veeco.com: 107 jobs declared, 25 rendered, and every paging parameter tried returns page one. No data attributes and no per-card markup the Synopsys reader matches; the JSON endpoint is under the robots-disallowed /services/. sitemap.xml is published and allowed. |
| 13 | P1 | Synopsys | EDA and engineering software | **supported** | synopsys | [proof](company-proofs/synopsys.json) | Complete; revalidate if this employer changes its board. Radancy/TalentBrew board read from the paged search page (its robots.txt disallows the /search-jobs/ JSON endpoint but not the page). 391 openings reconciled against the board's own published count; listing cards carry the posting date, so no detail fetch is needed to date a job. |
| 14 | P1 | Cadence | EDA and engineering software | **supported** | workday | [proof](company-proofs/cadence.json) | Complete; revalidate if this employer changes its board. Workday tenant cadence, site External_Careers. 562 openings, exact total, warm check clean. Some slugs are stale after a requisition is renamed; the requisition id in the URL still matches the stored externalId. |
| 15 | P1 | Siemens | EDA and engineering software | **supported** | siemens | [proof](company-proofs/siemens.json) | Complete; revalidate if this employer changes its board. Siemens Digital Industries Software board (jobs.sw.siemens.com), which includes Siemens EDA and Mendix -- not the capped global Avature portal tried earlier. Reads prod-search-api.jobsyn.org. 753 openings, exact total, warm check clean. |
| 17 | P1 | MathWorks | EDA and engineering software | **discovering** | — | — | Confirm whether the search results are reachable as a plain paged GET before treating this as adapter work. Reported as an in-house careers system on Adobe Experience Manager with form-based search. Not independently verified. |
| 22 | P2 | Lockheed Martin | Defense | **supported** | eightfold | [proof](company-proofs/lockheed-martin.json) | Complete; revalidate if this employer changes its board. Eightfold PCSX at lockheedmartin.eightfold.ai, domain lockheedmartin.com. 3060 openings, exact total reconciled, descriptions from the PCSX detail endpoint; warm check clean. |
| 23 | P2 | Northrop Grumman | Defense | **supported** | eightfold | [proof](company-proofs/northrop-grumman.json) | Complete; revalidate if this employer changes its board. Eightfold PCSX at jobs.northropgrumman.com, domain ngc.com. A plain read stops with auth_required around 1000 rows because Eightfold answers a burst with 403; retryForbidden plus a 900ms delay reads all 3701 openings, exact total, warm check clean. |
| 24 | P2 | RTX | Defense | **blocked** | phenom | — | Retry through the declared Playwright fallback, which would have to prove its own pagination before being accepted. Phenom board at careers.rtx.com. The landing page ships the phApp.ddo search payload the Cisco reader already knows how to parse, but the paged /global/en/search-results path is behind a Cloudflare JS challenge and answers 403 to a plain request, so the board cannot be traversed without a browser. |
| 25 | P2 | BAE Systems | Defense | **validating** | bae-systems | — | Decide whether the proof gate should accept rows deduplicated by identity (Apple's board behaves the same way) before admitting this and Thales. Phenom, and readable: the search page ships the phApp.ddo payload the Cisco reader already parses, so it is now a config entry against that shared reader rather than new code. A full traversal reaches the published total of 1855, but only 1440 distinct rows come out — this tenant lists a requisition once per location, and those collapse to one stored job. The read is complete; the proof gate counts the collapsed rows as missing. |
| 26 | P2 | General Dynamics | Defense | **blocked** | — | — | Decide whether to onboard the business-unit boards individually; there appears to be no single General Dynamics board. The Workday host generaldynamics.wd1.myworkdayjobs.com is not a live tenant: every path on it, robots.txt included, answers HTTP 422, so no site segment can be right. General Dynamics also runs its business units (GDIT, Mission Systems, Electric Boat, NASSCO) as separate employers with their own boards. |
| 27 | P2 | L3Harris | Defense | **supported** | l3harris | [proof](company-proofs/l3harris.json) | Complete; revalidate if this employer changes its board. Radancy/TalentBrew, older theme than Synopsys: bare <li> cards read by job id rather than theme class names. 1924 openings reconciled against the board's own count. This board publishes no posting date anywhere, on the card or the vacancy page, so its listings are stored undated and the age filter cannot narrow them. |
| 28 | P2 | Anduril | Defense | **supported** | greenhouse | [proof](company-proofs/anduril.json) | Complete; revalidate if this employer changes its board. Greenhouse token andurilindustries: 2202 openings in one response. The careers page loads them through its own backend, but the board itself is plain Greenhouse and its token is named in the site's CSP. |
| 29 | P2 | Shield AI | Defense | **supported** | ashby | [proof](company-proofs/shield-ai.json) | Complete; revalidate if this employer changes its board. Ashby token shield-ai. Whole board in one response with inline descriptions (445 openings); warm check clean. |
| 30 | P2 | AeroVironment | Defense | **supported** | workday | [proof](company-proofs/aerovironment.json) | Complete; revalidate if this employer changes its board. Workday tenant avav, site AVAV. 351 openings, exact total, warm check clean. |
| 31 | P2 | Kratos | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 32 | P2 | Saab | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 33 | P2 | Thales | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 34 | P2 | Leonardo | Defense | **blocked** | — | — | Check the Italian-language careers site and Leonardo's national subsidiaries for their own boards. No public vacancy board located from the corporate careers page, and the leonardo.it careers path 404s. Not confirmed against a rendered page. |
| 35 | P2 | Rheinmetall | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 36 | P2 | Hensoldt | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 37 | P2 | Kongsberg | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 38 | P2 | MBDA | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 39 | P2 | QinetiQ | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 40 | P2 | Indra | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 41 | P2 | Naval Group | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 42 | P2 | Babcock | Defense | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 43 | P2 | TEKEVER | Defense | **supported** | json | [proof](company-proofs/tekever.json) | Complete; revalidate if this employer changes its board. Teamtailor, read through the generic json adapter against its JSON Feed at /jobs.json: itemsPath items, paged with page/per_page, descriptions inline as content_html and the place taken from the embedded schema.org JobPosting address. All 123 openings, matching the count the careers page publishes. Its RSS feed at /jobs.rss carries only the most recent 100 and must not be used. |
| 44 | P2 | Airbus | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 45 | P2 | Boeing | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 46 | P2 | GE Aerospace | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 47 | P2 | Safran | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 48 | P2 | Rolls-Royce | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 49 | P2 | MTU Aero Engines | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 50 | P2 | Dassault Aviation | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 51 | P2 | Bombardier | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 52 | P2 | Embraer | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 53 | P2 | CAE | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 54 | P2 | Honeywell | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 55 | P2 | GKN Aerospace | Aviation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 56 | P2 | SpaceX | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 57 | P2 | Blue Origin | Space | **discovering** | — | — | Identify the real board from a rendered page; the site rate-limits plain fetches. Board not identified. blueorigin.com/careers answers 429 to plain requests, and no Greenhouse or Ashby token under blueorigin/blue-origin exists. WARNING: the Lever token 'blue' answers with 10 live jobs and is NOT Blue Origin (data roles in Costa Rica and Eastern Europe) — an earlier pass proved and nearly admitted it. Do not accept that board. |
| 58 | P2 | Rocket Lab | Space | **supported** | greenhouse | [proof](company-proofs/rocket-lab.json) | Complete; revalidate if this employer changes its board. Greenhouse token rocketlab, 445 openings. |
| 59 | P2 | Relativity Space | Space | **supported** | greenhouse | [proof](company-proofs/relativity-space.json) | Complete; revalidate if this employer changes its board. Greenhouse token relativity, 341 openings. |
| 60 | P2 | Stoke Space | Space | **supported** | greenhouse | [proof](company-proofs/stoke-space.json) | Complete; revalidate if this employer changes its board. Greenhouse token stokespacetechnologies, 62 openings. |
| 61 | P2 | Sierra Space | Space | **supported** | workday | [proof](company-proofs/sierra-space.json) | Complete; revalidate if this employer changes its board. Workday tenant sierraspace, site Sierra_Space_External_Career_Site, 164 openings in Louisville CO. NOTE: the Ashby token 'sierra' is Sierra AI, a different company, and was nearly admitted as this one. |
| 62 | P2 | Axiom Space | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 63 | P2 | MDA Space | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 64 | P2 | OHB | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 65 | P2 | Beyond Gravity | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 66 | P2 | Open Cosmos | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 67 | P2 | Planet | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 68 | P2 | ICEYE | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 69 | P2 | Telesat | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 70 | P2 | GMV | Space | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 71 | P3 | ABB | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 72 | P3 | Schneider Electric | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 73 | P3 | Rockwell Automation | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 74 | P3 | Emerson | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 75 | P3 | Beckhoff | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 76 | P3 | Festo | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 77 | P3 | Phoenix Contact | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 78 | P3 | WAGO | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 79 | P3 | Omron | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 80 | P3 | Eaton | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 81 | P3 | TE Connectivity | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 82 | P3 | Molex | Industrial automation | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 83 | P3 | Keysight | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 84 | P3 | Rohde & Schwarz | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 85 | P3 | Tektronix | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 86 | P3 | Anritsu | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 87 | P3 | VIAVI | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 88 | P3 | Hexagon | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 89 | P3 | Teledyne | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 90 | P3 | Fortive | Test and measurement | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 91 | P4 | onsemi | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 92 | P4 | Qorvo | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. A previous pass found no reusable readable configuration. Reassess the official board; implement a custom adapter if needed. |
| 93 | P4 | Skyworks | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. A previous pass found no reusable readable configuration. Reassess the official board; implement a custom adapter if needed. |
| 94 | P4 | Synaptics | Semiconductors | **blocked** | static-css | — | Try the browser adapter, or find whether Talemetry answers a different header/cookie combination that a plain fetch can supply. Talemetry board (careers.synaptics.com) with a Cloudflare challenge on every plain request -- the listing, its JSON path, page 2, job detail pages and robots.txt all return a Just a moment challenge, though the same pages render normally in a real browser. 13 openings observed by hand; no direct transport was found. |
| 95 | P4 | Cirrus Logic | Semiconductors | **supported** | lever | [proof](company-proofs/cirrus-logic.json) | Complete; revalidate if this employer changes its board. Lever token cirrus (jobs.eu.lever.co). 113 openings. |
| 96 | P4 | Renesas | Semiconductors | **supported** | renesas | [proof](company-proofs/renesas.json) | Complete; revalidate if this employer changes its board. SmartRecruiters token RenesasElectronics. 898 openings. |
| 97 | P4 | ROHM | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 98 | P4 | Nordic Semiconductor | Semiconductors | **supported** | json | [proof](company-proofs/nordic-semiconductor.json) | Complete; revalidate if this employer changes its board. Teamtailor JSON Feed, same shape as TEKEVER/SiPearl. 15 openings. |
| 99 | P4 | ams OSRAM | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 100 | P4 | Wolfspeed | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 101 | P4 | Power Integrations | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 102 | P4 | Diodes Incorporated | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 103 | P4 | Rambus | Semiconductors | **supported** | rambus | [proof](company-proofs/rambus.json) | Complete; revalidate if this employer changes its board. iCIMS HTML board (careers-rambus.icims.com), not the generic JSON icims adapter -- this tenant serves HTML search pages, zero-based pr offset via link[rel=next], and descriptions only on the schema.org JobPosting detail page. 64 openings across 4 pages, exact page count, warm check clean. |
| 104 | P4 | CEVA | Semiconductors | **supported** | ceva | [proof](company-proofs/ceva.json) | Complete; revalidate if this employer changes its board. Comeet API (company 76.005) read with details=true so both Description and Requirements sections are joined; the generic json adapter only kept Description and left every listing description empty. 37 openings including the general-application entry, exact total, warm check clean. |
| 105 | P4 | Imagination Technologies | Semiconductors | **supported** | json | [proof](company-proofs/imagination-technologies.json) | Complete; revalidate if this employer changes its board. PageUp job feed at careers.pageuppeople.com/774/cw/en/jobs.json. 14 openings, exact total, warm check clean. |
| 106 | P4 | XMOS | Semiconductors | **blocked** | playwright | — | Revisit with the browser adapter once XMOS lists more than a handful of roles; verifying pagination on a one-job board proves nothing. WordPress board behind a Cloudflare challenge to any plain request: the listing, its wp-json REST path, the admin-ajax pagination endpoint and even robots.txt all return a JS challenge, not content. Only one real vacancy exists right now (Senior AI Compiler Engineer), and the homepage widget that shows it caps display at three posts with pagination disabled, so its data-total cannot be trusted as an exact count either. A browser read is required, and it still cannot certify completeness of a genuinely paginated board. |
| 107 | P4 | Pragmatic Semiconductor | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 108 | P4 | SiPearl | Semiconductors | **supported** | json | [proof](company-proofs/sipearl.json) | Complete; revalidate if this employer changes its board. Teamtailor, read via the generic json adapter against its JSON Feed (same shape as TEKEVER). 19 openings, exact total. |
| 109 | P4 | Axelera AI | Semiconductors | **supported** | ashby | [proof](company-proofs/axelera-ai.json) | Complete; revalidate if this employer changes its board. Ashby token axelera. 9 openings. |
| 110 | P4 | Etched | Semiconductors | **supported** | ashby | [proof](company-proofs/etched.json) | Complete; revalidate if this employer changes its board. Ashby token etched. 108 openings. |
| 111 | P4 | TSMC | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. A previous pass found no reusable readable configuration. Reassess the official board; implement a custom adapter if needed. |
| 112 | P4 | Samsung | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 113 | P4 | UMC | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 114 | P4 | Tower Semiconductor | Semiconductors | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 115 | P4 | HP | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 116 | P4 | HPE | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 117 | P4 | Lenovo | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 118 | P4 | Supermicro | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 119 | P4 | Sony | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 120 | P4 | Panasonic | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 121 | P4 | Logitech | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 122 | P4 | Garmin | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 123 | P4 | Zebra Technologies | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 124 | P4 | Jabil | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 125 | P4 | Flex | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 126 | P4 | Celestica | Hardware and electronics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 127 | P4 | ZEISS | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. A previous pass found no reusable readable configuration. Reassess the official board; implement a custom adapter if needed. |
| 128 | P4 | Coherent | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 129 | P4 | Lumentum | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 130 | P4 | TRUMPF | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 131 | P4 | Jenoptik | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 132 | P4 | Hamamatsu Photonics | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 133 | P4 | IPG Photonics | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 134 | P4 | MKS Instruments | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 135 | P4 | Excelitas | Photonics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 136 | P4 | Arista Networks | Networking | **supported** | arista-networks | [proof](company-proofs/arista-networks.json) | Complete; revalidate if this employer changes its board. SmartRecruiters token AristaNetworks. 234 openings. |
| 137 | P4 | Ciena | Networking | **supported** | workday | [proof](company-proofs/ciena.json) | Complete; revalidate if this employer changes its board. Workday tenant ciena, site Careers. 142 openings. |
| 138 | P4 | F5 | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 139 | P4 | Adtran | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 140 | P4 | Calix | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 141 | P4 | Extreme Networks | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 142 | P4 | Motorola Solutions | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 143 | P4 | Viasat | Networking | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 144 | P4 | Tesla | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 145 | P4 | Rivian | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 146 | P4 | Lucid | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 147 | P4 | Ford | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 148 | P4 | General Motors | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 149 | P4 | BMW | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 150 | P4 | Mercedes-Benz | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 151 | P4 | Volkswagen | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 152 | P4 | Volvo Cars | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 153 | P4 | Bosch | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 154 | P4 | ZF | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 155 | P4 | Magna | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 156 | P4 | Aptiv | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 157 | P4 | Valeo | Automotive | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 158 | P4 | Boston Dynamics | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 159 | P4 | Agility Robotics | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 160 | P4 | Figure AI | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 161 | P4 | 1X | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 162 | P4 | ANYbotics | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 163 | P4 | Sanctuary AI | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 164 | P4 | Symbotic | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 165 | P4 | Locus Robotics | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 166 | P4 | FANUC | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 167 | P4 | KUKA | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 168 | P4 | Yaskawa | Robotics | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 169 | P5 | Microsoft | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 170 | P5 | Amazon | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 171 | P5 | Meta | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 172 | P5 | IBM | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 185 | P5 | Canonical | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 186 | P5 | SUSE | Software and cloud | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 191 | P5 | Fortinet | Cybersecurity | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 199 | P5 | IonQ | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 200 | P5 | Quantinuum | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 201 | P5 | PsiQuantum | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 202 | P5 | Rigetti | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 203 | P5 | D-Wave | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 204 | P5 | IQM | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 205 | P5 | Pasqal | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 206 | P5 | QuEra | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 207 | P5 | Alice & Bob | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 208 | P5 | Riverlane | Quantum | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 209 | P5 | Siemens Energy | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 210 | P5 | GE Vernova | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 211 | P5 | Hitachi Energy | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 212 | P5 | Vestas | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 213 | P5 | Nordex | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 217 | P5 | Westinghouse | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 218 | P5 | BWX Technologies | Energy | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 219 | P5 | Medtronic | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 220 | P5 | Siemens Healthineers | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 221 | P5 | GE HealthCare | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 222 | P5 | Philips | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 223 | P5 | Stryker | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 224 | P5 | Boston Scientific | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 225 | P5 | Intuitive | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 226 | P5 | Abbott | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 227 | P5 | BD | Medical devices | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 229 | P6 | Alten | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 230 | P6 | Capgemini Engineering | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 231 | P6 | Akkodis | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 232 | P6 | Expleo | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 233 | P6 | SEGULA Technologies | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 234 | P6 | EDAG | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 235 | P6 | AVL | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 236 | P6 | FEV | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 237 | P6 | Critical Software | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 238 | P6 | Critical TechWorks | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 239 | P6 | Sener | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 240 | P6 | Bertrandt | Engineering services | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 260 | P6 | Alstom | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 261 | P6 | Stadler | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 262 | P6 | CAF | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 263 | P6 | Hitachi Rail | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 264 | P6 | Wabtec | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 265 | P6 | Knorr-Bremse | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 266 | P6 | Caterpillar | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 267 | P6 | John Deere | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 268 | P6 | CNH Industrial | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |
| 269 | P6 | Liebherr | Rail and heavy machinery | **queued** | — | — | Discover the official careers board and confirm employer identity. |

## Out of scope

Recorded, not planned. These employers sit outside ASIC, digital design and verification, embedded software, mixed-signal and hardware work. They keep their original sequence number, so lifting the filter renumbers nothing.

| Order | Company | Category | Why it is out of scope |
| ---: | --- | --- | --- |
| 16 | Dassault Systèmes | EDA and engineering software | Mechanical CAD/PLM software, not silicon or embedded. |
| 18 | Autodesk | EDA and engineering software | Design software for AEC and manufacturing, not silicon or embedded. |
| 19 | PTC | EDA and engineering software | PLM and IoT application software, not silicon or embedded. |
| 20 | Bentley Systems | EDA and engineering software | Infrastructure design software, not silicon or embedded. |
| 21 | COMSOL | EDA and engineering software | Multiphysics simulation software, not silicon or embedded. |
| 173 | Adobe | Software and cloud | Application software vendor, no hardware organisation. |
| 174 | SAP | Software and cloud | Enterprise software vendor, no hardware organisation. |
| 175 | ServiceNow | Software and cloud | Enterprise SaaS, no hardware organisation. |
| 176 | Workday | Software and cloud | Enterprise SaaS, no hardware organisation. |
| 177 | Snowflake | Software and cloud | Data SaaS, no hardware organisation. |
| 178 | Atlassian | Software and cloud | Developer SaaS, no hardware organisation. |
| 179 | Shopify | Software and cloud | Commerce SaaS, no hardware organisation. |
| 180 | Uber | Software and cloud | Consumer platform, no hardware organisation. |
| 181 | Netflix | Software and cloud | Streaming platform, no hardware organisation. |
| 182 | Dropbox | Software and cloud | Storage SaaS, no hardware organisation. |
| 183 | Zoom | Software and cloud | Communications SaaS, no hardware organisation. |
| 184 | Box | Software and cloud | Storage SaaS, no hardware organisation. |
| 187 | Hugging Face | Software and cloud | ML platform, no hardware organisation. |
| 188 | Docker | Software and cloud | Developer tooling, no hardware organisation. |
| 189 | Palo Alto Networks | Cybersecurity | Security software, no silicon or embedded engineering. |
| 190 | CrowdStrike | Cybersecurity | Security software, no hardware. |
| 192 | Zscaler | Cybersecurity | Security SaaS, no hardware. |
| 193 | SentinelOne | Cybersecurity | Security software, no hardware. |
| 194 | Snyk | Cybersecurity | Security software, no hardware. |
| 195 | Tenable | Cybersecurity | Security software, no hardware. |
| 196 | Rapid7 | Cybersecurity | Security software, no hardware. |
| 197 | Sophos | Cybersecurity | Security software, no hardware. |
| 198 | Rubrik | Cybersecurity | Data protection software, no hardware. |
| 214 | Ørsted | Energy | Energy developer and operator, not equipment engineering. |
| 215 | Vattenfall | Energy | Utility operator, not equipment engineering. |
| 216 | EDF | Energy | Utility operator, not equipment engineering. |
| 228 | Zimmer Biomet | Medical devices | Orthopaedic implants; mechanical, with little electronics or firmware. |
| 241 | Arup | Civil and infrastructure | Civil and structural consultancy. |
| 242 | AECOM | Civil and infrastructure | Civil consultancy. |
| 243 | WSP | Civil and infrastructure | Civil consultancy. |
| 244 | AtkinsRéalis | Civil and infrastructure | Civil consultancy. |
| 245 | Jacobs | Civil and infrastructure | Civil consultancy. |
| 246 | Mott MacDonald | Civil and infrastructure | Civil consultancy. |
| 247 | Stantec | Civil and infrastructure | Civil consultancy. |
| 248 | Arcadis | Civil and infrastructure | Civil consultancy. |
| 249 | Ramboll | Civil and infrastructure | Civil consultancy. |
| 250 | Aecon | Civil and infrastructure | Construction contractor. |
| 251 | BASF | Materials and chemicals | Chemicals. |
| 252 | Dow | Materials and chemicals | Chemicals. |
| 253 | DuPont | Materials and chemicals | Materials and chemicals. |
| 254 | 3M | Materials and chemicals | Diversified materials. |
| 255 | Corning | Materials and chemicals | Glass and optical materials, not electronics design. |
| 256 | Air Liquide | Materials and chemicals | Industrial gases. |
| 257 | Linde | Materials and chemicals | Industrial gases. |
| 258 | Evonik | Materials and chemicals | Specialty chemicals. |
| 259 | Solvay | Materials and chemicals | Specialty chemicals. |
