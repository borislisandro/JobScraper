# Company onboarding runbook

The procedure for taking one employer from "only the name is known" to a saved live proof. It is
written for a worker agent handling a batch of companies. Read it once, then work one company at a
time: a batch is a unit of assignment, not a unit of work.

## What you own and what you must not touch

You own exactly two files per company, both named after the company's tracker `id`:

- `docs/company-candidates/<id>.json` — the worker source you propose.
- `docs/company-proofs/<id>.json` — the evidence the verifier writes.

Do **not** edit any of these; they have a single integration owner and concurrent edits corrupt them:
`sidecar/company-catalog.json`, `sidecar/adapters.mjs`, `sidecar/company-adapters.mjs`,
`sidecar/worker.mjs`, `docs/company-support-tracker.json`, anything under `src/` or `src-tauri/`.
If a company needs an adapter change, you report it; you do not make it.

## 1. Find the real board

Start from the employer's own careers page and follow it to the system that actually serves the
vacancies. Confirm the board belongs to that employer — a similar name is not evidence.

Fetch pages with `node -e` and plain `fetch`, which is fast and shows you redirects and payloads:

```bash
node -e "fetch('https://www.example.com/careers',{redirect:'manual'}).then(async r=>console.log(r.status,r.headers.get('location'),(await r.text()).slice(0,3000)))"
```

**Start with the detector.** It follows the redirects, matches the platform signatures below, and
then confirms any hosted token by calling that platform's API and reporting a live job count:

```bash
node scripts/detect-board.mjs "AeroVironment" https://www.avinc.com/careers
```

A `LIVE` line is a board that answered with real rows. A `seen` line is only a signature in the
markup — still a lead worth chasing, not an answer. If it prints nothing, read the page yourself.

Recognising the platform from the URL the careers page leads to:

| What you see | Adapter | Settings to extract |
| --- | --- | --- |
| `https://<tenant>.wd<N>.myworkdayjobs.com/<site>` | `workday` | `tenant`, `site` from the path |
| `https://wd<N>.myworkdaysite.com/en-US/recruiting/<tenant>/<site>` | `workday` | same two, from the path |
| `boards.greenhouse.io/<token>`, `job-boards.greenhouse.io/<token>` | `greenhouse` | `token` |
| `jobs.ashbyhq.com/<token>` | `ashby` | `token` |
| `jobs.lever.co/<token>` | `lever` | `token` |
| an Eightfold careers host (`/api/pcsx/search` answers) | `eightfold` | `domain` = the employer's own domain, not the careers host |
| `*.fa.<region>.oraclecloud.com`, `/hcmUI/CandidateExperience` | `oracle` | `apiHost` (the Oracle pod), `siteNumber` |
| `careers-<co>.icims.com` | `icims` | usually just the base URL |

Anything else — SmartRecruiters, SuccessFactors, Taleo, Workable, Recruitee, Personio, Teamtailor,
Jobvite, an in-house board — has no adapter yet. Do not force it into one. See section 5.

## 2. Prove the endpoint answers before you write a candidate

One probe, so you find out now rather than after a ten-minute traversal. Workday, for example:

```bash
node -e "fetch('https://<tenant>.wd1.myworkdayjobs.com/wday/cxs/<tenant>/<site>/jobs',{method:'POST',headers:{accept:'application/json','content-type':'application/json'},body:JSON.stringify({appliedFacets:{},limit:20,offset:0,searchText:''})}).then(r=>r.json()).then(d=>console.log(d.total,d.jobPostings?.length,d.jobPostings?.[0]?.title))"
```

Robots enforcement is off for this tool: every source runs with `robotsOverride: true`, matching what
the app itself seeds. So a `Disallow` is not a reason to stop — read the endpoint that actually serves
the vacancies. Still note in your report what the host's `robots.txt` says, because it often points
at the real API (a disallowed `/services/` or `/search-jobs/` path is usually exactly where the JSON
lives). What has not changed: keep the normal pacing, never set `allowPrivateNetwork`, and never guess
a private or authenticated endpoint.

## 3. Write the candidate

`docs/company-candidates/<id>.json`, using the tracker's exact `name`:

```json
{
  "id": "<tracker id>",
  "name": "<exact tracker name>",
  "baseUrl": "https://<the human-readable board>",
  "adapterId": "workday",
  "kind": "active",
  "robotsOverride": true,
  "allowPrivateNetwork": false,
  "configJson": {
    "tenant": "…",
    "site": "…",
    "maxPages": 500,
    "expectedHost": "<host that answers>"
  }
}
```

`robotsOverride` must be `true`. `allowPrivateNetwork`, `testNoDelay` and `testJitterMs` are refused
by the verifier — never add them. Only set a value you observed; a guessed tenant or token is worse than a
blocked row because it silently reads someone else's board.

## 4. Prove it live

```bash
node scripts/verify-company-catalog.mjs --full --candidate docs/company-candidates/<id>.json --report docs/company-proofs/<id>.json
```

This runs the real worker: a full cold traversal with no known hashes, deferred descriptions
fetched for the first, middle and last job, and a warm re-check. It exits non-zero and lists the
reasons in the report's `errors` unless everything holds. A large board can take several minutes;
let it finish.

Common failures and what they actually mean:

- *Full traversal is unfinished* — paging never reached the end, or hit `maxPages`. Look at
  `listing.terminal.payload` for `pages` and `boardTotal` before changing anything.
- *Discovered rows do not match the exact board total* — the board moved, or the traversal read
  overlapping pages. Re-run once; if it repeats, it is a real contract problem, so report it.
- *Warm check found unseen listings* — usually a board that publishes constantly, sometimes an
  unstable identity. Re-run once and say which it was.
- *Duplicate job identities* — the listing's key is not unique on this board. Report it.

Do not paper over a failure by narrowing the read (a query filter, a lower `maxPages`, a facet) so
that less of the board is checked. A partial read is a failed proof.

## 5. When there is no adapter

**"Blocked" is a claim that needs evidence, and it is the single most common wrong answer.** Every
blocked verdict from the first rounds turned out to be readable: Anduril was called a custom Lambda
backend and is plain Greenhouse with 2202 openings; AeroVironment was called misconfigured and is
Workday `avav`/`AVAV` with 351. Before you write "blocked":

- run `scripts/detect-board.mjs` on the careers URL, and on the board it redirects to;
- try the employer's obvious slug against Greenhouse, Ashby and Lever — the detector does this for
  you, and a 404 on one token says nothing about another;
- a 403 on one path is not a blocked board. Eightfold answers a burst with 403; `retryForbidden`
  plus a larger `requestDelayMs` reads straight through it (that is how Northrop Grumman's 3701
  openings were read after a plain attempt died at 1000);
- say which specific thing you observed — a status code, a challenge page, a missing parameter —
  not that a page "blocks automated fetch requests".

Then write up what you found. That report is the deliverable, and it is worth as much as a proof —
one new platform adapter usually unlocks many queued employers. Include:

- the employer, the official careers page, and the board it leads to;
- the platform name;
- the exact listing endpoint, method, headers and body that answered, with the first response's
  top-level keys;
- how it pages, and where the total is published;
- where a description lives (listing row or a per-job endpoint, with that endpoint's URL shape);
- what `robots.txt` on that host says about the path, since a disallowed path often names the API.

## 6. Report back

For every company in your batch, one block:

```
<name> — supported | blocked | needs-adapter
adapter: <adapterId or platform name>
careers: <official careers URL>
board: <base URL>
proof: docs/company-proofs/<id>.json  (jobs: N, complete: true/false)
country: <ISO 2-letter code of the headquarters>
tags: <2-3 lowercase tags, e.g. semiconductor,equipment>
notes: <one or two sentences an integrator needs — tenant/site/token, and anything surprising>
```

`country` and `tags` are required for a supported company; the integrator needs them for the
catalog entry and cannot infer them from the proof.
