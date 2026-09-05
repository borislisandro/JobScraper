# Project audit and implementation plan — 5 September 2026

## Outcome

The application has a useful foundation: a local Tauri/SQLite backend, 164 readable catalog entries, streamed persistence, bounded source concurrency, lazy descriptions, and a conservative two-miss closure rule. The most urgent problems are trustworthy coverage and incremental freshness, rather than a lack of adapters.

This audit implemented focused scraping, performance, parsing, and usability fixes. It also identified remaining work below. **A successful preview is not proof of complete coverage**, and agreement with one ATS's count is not proof that another company-published feed contains the same vacancies.

### Scope and evidence

- Reviewed the React filtering/reader/source flows, Rust orchestration and persistence, Node transport/pagination/normalization, migrations, verification scripts, tests, CI, and catalog contracts.
- Ran live previews for **all 164 readable entries** in the 165-entry catalog; the remaining entry is a reference bookmark. All previews returned recognizable rows. These checks inspect at most one listing page and ten emitted examples per source, not every vacancy.
- Ran full cold traversal, unique-identity checks, first/middle/last description samples, and a warm check for **ten engineering employers**, before and after fixes. All ten post-fix proofs passed against their configured providers.
- Inspected the employer-facing Synopsys search page, NVIDIA's corporate careers links and newer public feed, and other public careers pages. Synopsys independently displayed **418 results**, matching its scrape.
- Inspected local databases **read-only**. Development data contains **14,556 jobs**, **5,962 pending descriptions**, **1,508 undated jobs**, and **8 without a resolved country**. The installed-data database inspected has an older schema, 19 sources and no stored jobs. Neither database was migrated or changed during this audit.
- Ran application tests, build, lint, and seven-run local performance benchmarks. No installed desktop end-to-end run or release packaging was performed.

Evidence files: [catalog previews](catalog-previews.json), [full proofs before](full-proofs-before.json), [full proofs after](full-proofs-after.json), [NVIDIA's official feed](nvidia-official-board.json), [benchmarks](benchmarks.json), and [database aggregates](database-summary.json).

Stored database counts must not be divided by today's published count and called scraper recall: the database includes historical state, source settings, filters and different observation times. This audit does not establish exact stored-job recall.

## Live reconciliation

| Employer | Configured provider | Unique jobs emitted | Provider total | Post-fix full proof |
| --- | --- | ---: | ---: | --- |
| NVIDIA | Workday, category slices | 2,695 | 2,695 from categories | Passed |
| Qualcomm | Eightfold | 1,957 | 1,957 | Passed |
| AMD | Employer adapter | 1,169 | 1,169 | Passed |
| Renesas | Employer adapter | 899 | 899 | Passed |
| Texas Instruments | Oracle | 649 | 649 | Passed |
| Intel | Workday | 600 | 600 | Passed |
| ASML | Sitecore search | 596 | 596 | Passed |
| Synopsys | Radancy | 418 | 418 | Passed |
| SiFive | Workday | 116 | 116 | Passed |
| Axelera AI | Ashby | 10 | 10 | Passed |

These are time-stamped observations, not permanent company counts. Full proof includes deferred descriptions where required and a warm check reporting no unseen listings. Description sampling covers three positions, not every posting's semantic accuracy. The final additional guard against category sums below the unfiltered total was verified with a deterministic fixture after these live proofs; each proof retains its recorded worker hash.

### NVIDIA: confirmed scraper bug and an unresolved publisher discrepancy

Before the fix, Workday's unfiltered response said `total: 2000`, while its job-category counts summed to 2,695 and the worker emitted 2,695 unique jobs. The worker copied 2,000 into `discovered`, called it an exact board total, and reported `complete: true`. Full proof correctly rejected that inconsistent result. It also made a corrected full-read baseline incompatible with an unchanged first-page check.

The fix counts unique sightings across slices, includes known and filtered jobs, checks slice counts, preserves other facet filters, and compares against category totals. A short or repeating slice cannot authorize closure. The warm check derives the same category total without traversing the slices.

However, [NVIDIA's corporate careers page](https://www.nvidia.com/en-us/about-nvidia/careers/) now links to [jobs.nvidia.com](https://jobs.nvidia.com/careers), an Eightfold board. Its [public search API](https://jobs.nvidia.com/api/pcsx/search?domain=nvidia.com&start=0&query=&location=) reported **2,685**, ten fewer than Workday. A deeper identity comparison hit HTTP 429; a later single-page observation still reported 2,685. Indexing delay, publication policy, or feed differences are possible explanations, not established causes. **The ten-job discrepancy remains unresolved.** Do not automatically migrate the source or close ten jobs based on aggregate counts.

[Synopsys's public search page](https://careers.synopsys.com/search-jobs) displayed 418 results and real verification/RTL roles, matching the full scrape. Its 28-page traversal took about 62 seconds in the final run; Qualcomm took about 124 seconds and NVIDIA about 118 seconds. These are single live observations and include provider latency and normal pacing.

## Implemented fixes

| Priority | Problem and evidence | Change | Validation |
| --- | --- | --- | --- |
| P0 | Platform short pages could end traversal before the published total; duplicate pages could satisfy the raw count. `sidecar/worker.mjs:455` | Require count consistency and valid unique identities; report partial reads and stop pages that add no identities. | Short, repeated and drifting Workday fixtures; existing Eightfold regressions; ten live full proofs. |
| P0 | Workday splitting substituted a published number for observed coverage. `sidecar/worker.mjs:536` | Union actual identities before filtering, reconcile slice/category counts, retain other source facets, use consistent warm-check totals, reject categories that omit counted jobs. | Missing/overlapping slices, known/filtered rows, capped warm count and omitted-category fixtures; NVIDIA live proof. |
| P1 | Permanent 404/410/422 responses were retried twice with backoff. `sidecar/worker.mjs:187` | Stop retries for permanent HTTP client errors; retain timeout, throttle and server-error handling. | Fixture verifies one request for permanent errors and recovery after a 503. |
| P1 | Out-of-range HTML numeric entities could throw and terminate a scrape; invalid ISO dates were stored verbatim. `sidecar/worker.mjs:19,44` | Replace invalid Unicode scalar references; validate calendar dates and reject overflowing relative dates. | Malformed entity, leap-day and invalid-date fixtures. |
| P2 | "Recent" used `updated_at`, so refreshing old jobs reordered discovery results. `src-tauri/src/db.rs:2259` | Sort by discovery time with a legacy creation-time fallback; add an index for that expression. | Rust regression inserts an old job refreshed after a newer discovery and checks stable order. |
| P2 | Search wording concealed missing description coverage behind a tooltip. `src/main.tsx:319` | Rename the field to "Stored listing text" and make the description-loading limitation visible. | Frontend tests, typecheck and production build. The label change is not a solution to missing descriptions. |
| P2 | Benchmark parsed arbitrary pipe chunks as complete JSON lines and ignored malformed chunks; proof runtime differed from the app's header limit. | Reuse the line-framed worker driver, use the app's 64 KB header limit, capture proof hashes before execution, distinguish partial outcomes, add failure benchmarks. | Benchmarks and worker execution tests. |
| P2 | A performance test assumed at least 60 ms of sleep even when response time consumed the pacing interval. | Assert that measured pacing is attributed to request types instead of imposing a false minimum sleep. | Baseline failure reproduced; final suite passed. |
| P3 | README advertised 134 companies and described a robots policy that the verifier did not use. | Correct readable catalog count to 164 and document the actual saved override policy. | Catalog inventory and verifier inspection. |

No new runtime dependencies were added. Changes are in this worktree; delivering them to the installed application still requires a release build and installation. Migration 0021 adds an index and does not rewrite job records.

## Performance results

Paired failure tests used the original `HEAD` worker and the changed worker on the same local fixtures, seven runs per version, with artificial pacing disabled. Measurements below are median **worker** elapsed times, not desktop startup or full application update time.

| Scenario | Before requests | After requests | Before median | After median |
| --- | ---: | ---: | ---: | ---: |
| Permanent HTTP 404 | 3 | 1 | 1,585 ms | 40 ms |
| Endpoint repeats one vacancy toward a 500-row total | 500 | 2 | 6,968 ms | 49 ms |

The repeating endpoint previously reported a complete read; it now reports partial coverage. The gains come from avoiding useless work, not increasing traffic to employers.

Healthy-path fixtures stayed in the same general range: 100 jobs with descriptions took 1,480 ms before and 1,468 ms in the final run; listings-only took 114 ms before and 97 ms after; a first-page check took 62 ms before and 53 ms after. Environment noise can explain these small differences. **No broad healthy-board speedup is claimed.**

For further performance work, use measured provider latency, pacing, detail requests, and database writes separately. Existing source lanes, batching and lazy descriptions should be retained. Increasing global concurrency without identifying the actual API origin can worsen throttling.

## Remaining findings and concrete next work

P1 means the next correctness/security tranche; P2 is subsequent usability/performance work; P3 is maintenance. This table records the original findings. The follow-up implements request boundaries, browser guards, freshness, location retention, description refresh and identity fixes. See [follow-up results](followup.md) for current status, evidence and remaining acceptance work. Effort estimates below describe the original plan.

| Priority / effort | Gap and evidence | Implementation and acceptance condition |
| --- | --- | --- |
| P1 / 1–2 days | **Authenticated headers are not destination-scoped.** `requestHeaders` flattens saved cookies; `get` reuses them on derived URLs and redirects (`worker.mjs:183,388`). Code-confirmed mechanism; no real-session leak was tested or observed. | Match cookies by domain, path, secure flag and expiry per destination; scope explicit sensitive headers to their authorized origin. Add two-origin fixtures proving no cross-origin cookie/Authorization forwarding and preserving authorized same-origin sessions. |
| P1 / 1–2 days | **Browser private-network guard covers navigation, not all requests.** `installNavigationGuard` immediately permits non-navigation resources (`worker.mjs:567`). DNS checking and actual connection resolution also remain separate. | Guard XHR/fetch and other network requests at browser-context level; cover new pages/popups and service workers. Investigate transport-level DNS pinning separately. Tests must reject private fetches and redirected requests without preventing ordinary public ATS assets. |
| P1 / 1–2 days, provider-dependent | **Official NVIDIA feed parity: comparison completed in follow-up.** Workday 2,695 versus the linked Eightfold feed's 2,685. | Completed: all 2,685 linked-board roles matched after corroborated URL-suffix normalization; ten additional Workday roles remain a provider difference. See [follow-up evidence](followup.md). Keep the provider unchanged until any migration preserves saved applications and identities. |
| P1 / 1–2 days | **First-page count equality cannot prove no changes.** `classify_check` and `MAX_UNCHANGED_SKIPS` (`sidecar.rs:1278–1310`) permit three skipped reads. A replacement deeper in relevance-sorted results can leave the total unchanged. | Verify newest-first order per provider; otherwise impose a wall-clock maximum full-read age. Reset the skip budget after scrape-filter/source changes. Test same-count replacement on page two and filter widening. At a four-hour schedule, current behavior can defer discovery until roughly the fourth check; more frequent manual runs shorten that bound. |
| P1 / 1–2 days | **More than six locations are discarded before country resolution.** `toLocation` truncates persisted text (`worker.mjs:30–34`); the backend resolves countries from that text. | Preserve the full normalized location set for filtering/hashing; truncate only display text. Test a country appearing only in location seven and reordering the same locations. Backfill through a controlled refresh, with before/after country counts. |
| P1 / 1–2 days | **Cached descriptions can stay stale.** Stable title/location hashes skip the listing, and `job_description` returns immediately for status `complete` (`sidecar.rs:1227`). A description-only change is invisible on ordinary warm reads. | Add bounded description freshness for opened/saved jobs, conditional requests where supported, and an explicit refresh action. Test changed qualifications with unchanged title/location; preserve useful old text when refresh fails. |
| P1 / 0.5–1 day | **Requisition IDs are matched globally in persistence.** The `requisition_id=?` branch lacks company/source scope (`db.rs:1130`). No collision was observed in stored data; the ordinary worker currently does not emit a separate `requisitionId` field. | Scope requisition matching to the same employer/board identity before expanding emission. Test two companies sharing `1234` and the same company's duplicate URLs. Preserve application links; do not bulk-merge historical rows from numeric similarity. |
| P2 / 2–3 days | **Pending descriptions limit skill search.** 5,962 of 14,556 stored rows are pending; substring search only sees available text (`db.rs:2229`). | Add a user-controlled, bounded "download descriptions for these sources/jobs" queue with progress, cancellation and retry. Start with saved jobs and selected engineering sources. Acceptance: a skill appearing only in the body becomes searchable after enrichment; normal updates remain listing-first. |
| P2 / 1 day | **Verified empty boards never close old vacancies.** The final empty-read guard always converts a zero read to partial (`worker.mjs:740`). This is intentionally safe but can leave stale openings indefinitely. | Introduce adapter-specific trusted-empty evidence: a validated schema and explicit exact zero, confirmed on separate reads. Only that path may apply the existing two-miss rule. A parser failure or challenge page must never qualify. |
| P2 / 1–2 days | **Custom JSON/CSS traversal and company-specific readers do not share all platform safeguards.** The current patch protects platform readers; generic/page-specific completeness still depends on each reader's own contract. | Extend repeated-page/identity checks at the common result boundary; distinguish jobs observed from raw rows. Add capped/repeating fixtures for custom JSON, HTML, Apple multi-location results and provider pagination changes. Never convert an unknown total to an exact one using a preview length. |
| P2 / 1–2 days | **Worker lifetime and memory are not fully bounded.** Rust awaits stdout lines without an overall run deadline (`sidecar.rs:585`); Node reads whole bodies and keeps result arrays. | Add cancellation-aware inactivity/run budgets, response-size limits and bounded diagnostic retention. Test a worker that stays alive without a terminal event, an oversized response and cancellation during backoff; retain already-persisted jobs and avoid closure. |
| P2 / 1 day | **Concurrency is grouped by saved board hostname, not necessarily the actual API host.** `domain_groups` (`sidecar.rs:1378`) can miss a shared ATS behind different vanity hosts. | Derive a scheduling key from the effective listing origin; share throttle/backoff state across that origin. Test two vanity boards on one API and two independent hosts. Do not raise the lane cap until throttling and p95 improve under the same workload. |
| P2 / 1–2 days | **Accessibility and sort discoverability gaps.** The active sorting control is inside `aria-hidden="true"` (`main.tsx:376`); Role/Company buttons have no handlers. Several drawers use `dialog open` or an aside rather than native modal behavior. | Expose sorting controls to assistive technology, label the current order, remove inert button styling or implement explicit sorts. Verify keyboard entry, Escape, focus containment and focus restoration for each drawer with real keyboard interaction. |
| P2 / 1–2 days | **Search/paging scalability needs measurement beyond current data.** `jobs_page_query` uses substring scans, COUNT and OFFSET; the old unused FTS index was intentionally removed. | Benchmark 15k/50k/100k rows with common filters and page depths. Adopt cursor pagination or a suitable search index only if measured latency warrants it; preserve literal C++/RTL matching and exact filter semantics. |
| P3 / 0.5–1 day | **Catalog confidence is stronger than its evidence.** Most sources received only previews in this audit; historical proof and README/handoff state can drift. | Generate supported counts from the catalog, expose proof level/date, and rotate full checks by adapter family and engineering priority. Validate official corporate-link provenance as well as ATS response shape. |

### Useful additions, in order

1. **Source coverage panel:** display provider total, unique observed jobs, stored/filtered jobs, full-read time, last error and proof level separately. A user should see why 2,695 published jobs can legitimately produce fewer stored matches.
2. **Description coverage and refresh:** show how much of a selected search scope has searchable descriptions, then offer the bounded enrichment queue above. This directly helps UVM/SystemVerilog/RTL searches.
3. **Source repair workflow:** show a changed company careers link or adapter failure, test the proposed replacement, and preview identity preservation before saving it. Do not silently replace NVIDIA's provider.
4. **Engineering search presets:** build on existing saved views, with editable verification/RTL/FPGA/ASIC terms and country choices. Explain title-only scrape filters versus searches over stored descriptions; do not introduce opaque relevance scoring before coverage is reliable.

### Delivery sequence

1. **Completed in this worktree:** closure/completeness protections, repeated-page and HTTP failure performance, input parsing, stable discovery ordering, visible search limitation, benchmark/proof reliability and documentation.
2. **Correctness follow-up:** request boundaries, browser guards, incremental freshness, location retention, description refresh and employer-scoped requisitions are implemented. NVIDIA parity and remaining work are tracked in [followup.md](followup.md).
3. **Usability tranche:** coverage panel, selected enrichment, source repair and keyboard/focus fixes.
4. **Measured scale work:** gather baselines at larger databases and real update runs; tune the demonstrated bottleneck. Keep public-site live tests opt-in so third-party outages do not make normal CI nondeterministic.

## Validation and reproduction

Initial audit checks (superseded by [follow-up validation](followup.md)): **51 frontend tests, 107 scraper/adapter tests, 105 Rust tests passed; one existing Rust test ignored.** Production frontend build, TypeScript checking and formatting passed. Lint has zero errors and one existing unused-variable warning at `src/main.tsx:183`. An existing pacing-test failure was reproduced before the fix and corrected as described above.

Run from the repository root:

```text
pnpm test
pnpm build
pnpm lint
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --lib
node scripts/benchmark-scrape.mjs --fixtures --json
node scripts/verify-company-catalog.mjs --report previews.json
node scripts/verify-company-catalog.mjs --full Intel NVIDIA Qualcomm AMD ASML "Texas Instruments" "Axelera AI" Synopsys SiFive Renesas --report full-proofs.json
```

The last two commands contact public providers with the verifier's normal pacing and URL guards and never store jobs. Counts can change between runs. Reproducing before/after failure measurements requires the original worker in a separate temporary location or checkout, not reverting the current worktree. The shipped benchmark now retains both failure scenarios for future comparisons.

The worktree is ready for review. Release installation, a native desktop smoke test, and the remaining implementation tranches above are separate work; this audit does not claim all project risks have been removed.
