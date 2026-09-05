# Remaining findings: implementation follow-up

This continues the original audit in the same worktree. Changes are not installed or committed. Existing user databases have not been rewritten.

## Implemented

| Finding | Result | Proof |
| --- | --- | --- |
| Credential forwarding | Cookies are selected per destination using domain, path, Secure and expiry. Legacy cookies stay on their original origin. Authorization and custom headers cannot follow unrelated origins or redirects. Session files are read once per worker source. | Same-origin session and two-origin redirect tests; mixed-case headers, expiry and domain-boundary fixtures. |
| Browser private-network access | Browser contexts block service workers and send requests through a local proxy. The proxy connects to the address it checked, including redirect hops Chromium does not expose to page routing. Popups share the policy. IPv4-mapped IPv6 and the full link-local range are covered. | Real Edge tests for fetch, images, popup navigation and redirects; HTTP/CONNECT fixtures. Public HTTPS smoke test returned 200 and the expected heading. |
| Indefinitely stale update checks | On its next update, a source receives a full read when its previous full read is four hours old, its three-skip allowance is exhausted, or configuration/title-filter settings change. Incomplete reads and previews cannot establish a full baseline. Due reads ignore known hashes so changed settings are reapplied. | Rust tests cover age, missing/invalid/future timestamps, configuration/filter changes, and selection of full runs over newer previews. |
| Discarded locations | Store every normalized location. Sort and deduplicate arrays before hashing; reorder-only changes no longer trigger rereads. Object locations contribute actual names to identity hashes. | Seventh-location retention, array reordering and changed object-location tests. Existing rows recover on their next full read; no database backfill was performed. |
| Stale descriptions | Opening a downloadable description refreshes it after seven days; Refresh description requests an immediate read. Cached text remains visible after failed refreshes. Empty detail results fail instead of replacing text. Late requests cannot overwrite another job after navigation. | Database cache-age, explicit-refresh, stale-listing and failed-refresh tests; empty-detail worker regression; two reader interaction tests. |
| Cross-company requisitions | Requisition-only matches require the same normalized company name. Existing job IDs and application relationships remain attached. | Two companies sharing 1234 remain distinct; a changed URL within one company reuses the existing job. |
| Workday URL suffixes | A URL version such as JR2001099-1 no longer overrides the published JR2001099 when a listing bullet explicitly corroborates that ID. Genuine hyphenated IDs are retained; no unconditional suffix stripping was added to the scraper. | Four identity cases and a full query-scoped live proof for JR2001099, including detail and warm check. |
| Release integrity | Release prerequisites and build-input tracking include both new network-policy modules. Worker evidence hashes cover them too. | Packaging configuration inspection and normal build checks. An installer was not built. |

The four-hour limit is evaluated when an update runs; it does not promise fresh data while the application is offline. Description freshness applies to opened jobs with a supported detail URL. It does not pre-download every saved job or add conditional HTTP requests.

## Live validation

Three full provider proofs passed: Intel **600**, SiFive **116**, Axelera AI **10** jobs. Each includes traversal, sampled details and a warm change check. See [followup-full-proofs.json](followup-full-proofs.json). This is representative regression coverage, not a new full audit of all 164 readable companies.

Real Edge opened a public HTTPS page through the guarded proxy: [browser-public-smoke.json](browser-public-smoke.json). Plain ws:// upgrades are currently rejected; wss:// uses the guarded tunnel. Real authenticated sessions and every employer's browser fallback have not been exercised. Direct HTTP still checks DNS separately from the eventual fetch connection; transport pinning for that path remains open.

### NVIDIA: completed requisition comparison

[NVIDIA's corporate careers page](https://www.nvidia.com/en-us/about-nvidia/careers/) links to its [Eightfold careers board](https://jobs.nvidia.com/careers). The configured Workday board yielded **2,695** unique listings; the complete linked feed yielded **2,685** unique requisitions, with an unchanged reported total throughout the read.

The raw IDs matched 2,377 jobs. Another **308** matched after accounting for Workday URL version suffixes, and every one of those pairs had an identical title. A live listing and detail both explicitly published the unsuffixed requisition. This audit normalization found **all 2,685 linked-board jobs**, with **zero linked-board-only jobs** and **ten Workday-only jobs**. All ten extra Workday detail requests returned HTTP 200 with a published requisition. Their exact IDs, titles, URLs and dates are retained in [nvidia-requisition-parity.json](nvidia-requisition-parity.json), together with both identity sets and the raw differences.

This was a paced, checkpointed extraction, not an atomic snapshot. A local test recursion exhausted resources and interrupted collection; it resumed from the saved page and completed. The test defect was fixed and rerun successfully. No employer throttling was observed on the slower completed extraction. The evidence records the sampling window and the interruption.

The identity coverage gap is resolved for this snapshot. The reason the ten extra roles are absent from the linked search feed remains unknown; HTTP 200 does not prove that an application is still accepted. Keep the catalog provider unchanged while monitoring that difference in a subsequent audit. Do not close, merge or migrate stored jobs from aggregate count differences.

## Validation

**276 distinct tests passed:** 53 frontend, 115 scraper/adapter, 108 Rust; one existing Rust test remains ignored. The full frontend/scraper suite passed, followed by a focused 23-test rerun after the final Workday identity fix added one test. Production frontend build, lint (zero warnings/errors), Rust formatting and diff whitespace checks passed.

See [followup-validation.json](followup-validation.json) for the commands, validation scope and final code hashes. Live evidence comes from public endpoints and may change. No healthy-board speedup is claimed; stable location hashes and cached session parsing avoid redundant work, while four-hour forced reads deliberately spend requests to improve freshness.

## Concrete next work

1. Add bounded worker runtime and response sizes, preserving partial results and preventing closure after timeouts. Cover a silent worker, oversized response and cancellation during backoff.
2. Extend duplicate/repeated-page completeness checks to custom JSON, HTML and company-specific readers. Add schema-verified empty-board evidence before allowing empty reads to close jobs.
3. Add a cancellable description queue for selected/saved jobs and description coverage in search. Keep regular updates listing-first. Consider conditional requests only for providers verified to support them.
4. Group requests by effective API origin, then measure before changing concurrency. Pin DNS for direct HTTP connections too.
5. Finish country backfill verification on a controlled database copy; compare country filters and counts before/after without implicitly rewriting the user's database.
6. Fix inaccessible/inert sort controls and verify drawer focus with real keyboard interaction.
7. Benchmark search and pagination at 15k/50k/100k rows. Add coverage/proof dates and a repair workflow before broad catalog changes.

These remain open implementation work. Company-name scoping intentionally avoids fuzzy corporate alias matching. Description refresh without a detail URL, historical identity repair, release installation and native desktop smoke testing remain separate work.
