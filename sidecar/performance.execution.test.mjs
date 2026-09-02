// Where a run's time went, measured through the real worker process against a fixture host.
// The point of these tests is that the numbers are attributable: a request counted as a detail
// fetch really was one, and a wait charged to pacing really was pacing rather than the site.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { after, before, test } from "node:test";
import { listingHash } from "./worker.mjs";
import { scrapeCompany } from "./company-adapters.mjs";

let server, base;
const seen = [];
let jobCount = 1, failOnce = false, failFromOffset = null;
before(async () => {
  server = createServer(async (req, res) => {
    let body = ""; for await (const part of req) body += part;
    seen.push({ url: req.url, method: req.method });
    if (req.url === "/robots.txt") return res.setHeader("content-type", "text/plain").end("User-agent: *\nAllow: /\n");
    if (req.url.startsWith("/detail/")) return res.setHeader("content-type", "application/json").end(JSON.stringify({ description: "detail text" }));
    if (failOnce) { failOnce = false; return res.writeHead(503).end("busy"); }
    const { limit = 1, offset = 0 } = JSON.parse(body || "{}");
    // A board that dies partway through, and stays dead through every retry.
    if (failFromOffset !== null && offset >= failFromOffset) return res.writeHead(500).end("gone");
    const postings = Array.from({ length: jobCount }, (_, index) => index).slice(offset, offset + limit)
      .map(index => ({ id: `job-${index}`, jobReqId: `job-${index}`, title: `Engineer ${index}`, detailUrl: `/detail/${index}` }));
    res.setHeader("content-type", "application/json").end(JSON.stringify({ total: jobCount, jobPostings: postings }));
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});
after(() => new Promise(resolve => server.close(resolve)));

const source = config => ({
  id: "s", name: "Fixture", baseUrl: base, adapterId: "workday", kind: "active",
  allowPrivateNetwork: true, robotsOverride: false,
  configJson: { testNoDelay: true, maxPages: 5, pageSize: 1, listingPath: "/workday", tenant: "fixture", site: "External", ...config },
});
function execute(input, { cancelAfterMs } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["sidecar/worker.mjs"], { cwd: process.cwd() }), lines = [];
    child.stdout.on("data", data => lines.push(...String(data).trim().split("\n").filter(Boolean)));
    child.on("error", reject);
    child.on("close", () => resolve(lines.map(JSON.parse)));
    child.stdin.write(JSON.stringify(input) + "\n");
    if (cancelAfterMs === undefined) child.stdin.end();
    else setTimeout(() => child.stdin.end(JSON.stringify({ protocolVersion: 1, command: "cancel", runId: input.runId }) + "\n"), cancelAfterMs);
  });
}
const scrape = (config = {}, extra = {}, options) =>
  execute({ protocolVersion: 1, command: "scrape_source", runId: "perf", source: source(config), ...extra }, options);
const finalOf = events => events.at(-1);
// Every number the payload carries, flattened, so a single walk can assert all of them at once.
const numbers = (value, path = "") => typeof value === "number" ? [[path, value]]
  : value && typeof value === "object" ? Object.entries(value).flatMap(([key, child]) => numbers(child, path ? `${path}.${key}` : key))
  : [];
const shape = value => value && typeof value === "object" && !Array.isArray(value)
  ? Object.keys(value).sort().map(key => `${key}(${shape(value[key])})`).join(",")
  : typeof value;
const kinds = final => final.payload.performance.worker.requestsByKind;

test("every terminal result carries finite, non-negative metrics", async () => {
  jobCount = 1;
  const cases = [
    ["completed", await scrape()],
    ["failed", await execute({ protocolVersion: 1, command: "scrape_source", runId: "perf", source: { ...source(), adapterId: "reference", kind: "reference" } })],
    ["cancelled", await scrape({ testNoDelay: false, requestDelayMs: 400 }, {}, { cancelAfterMs: 120 })],
  ];
  for (const [outcome, events] of cases) {
    const final = finalOf(events);
    assert.equal(final.event, outcome, JSON.stringify(final));
    const performance = final.payload.performance;
    assert.equal(performance.version, 1, `${outcome} result is versioned`);
    const measured = numbers(performance);
    assert.ok(measured.length > 10, `${outcome} reports its buckets`);
    for (const [path, value] of measured) {
      assert.ok(Number.isFinite(value), `${outcome} ${path} is finite: ${value}`);
      assert.ok(value >= 0, `${outcome} ${path} is not negative: ${value}`);
    }
    assert.ok(performance.worker.totalMs > 0, `${outcome} measures its own run`);
  }
});

test("request kinds account for exactly the requests that were issued", async () => {
  jobCount = 3; seen.length = 0;
  const final = finalOf(await scrape({ pageSize: 3 }));
  assert.equal(final.event, "completed", JSON.stringify(final));
  const byKind = kinds(final);
  const counted = Object.values(byKind).reduce((sum, kind) => sum + kind.count, 0);
  assert.equal(counted, final.payload.requests, "no request is counted twice or missed");
  assert.equal(counted, seen.length, "the fixture host saw exactly what the worker counted");
  assert.equal(byKind.robots.count, 1, "robots.txt is charged to robots, not to the listing");
  assert.equal(byKind.detail.count, 3, "one detail page per fresh listing");
  assert.equal(byKind.listing.count, seen.filter(request => request.url === "/workday").length);
  assert.equal(byKind.probe.count + byKind.token.count + byKind.other.count, 0);
});

test("waiting on the site, reading its body, pacing and retry backoff land in separate buckets", async () => {
  jobCount = 1; seen.length = 0; failOnce = true;
  const final = finalOf(await scrape({ testNoDelay: false, requestDelayMs: 60 }));
  assert.equal(final.event, "completed", JSON.stringify(final));
  const { bucketsMs, requestsByKind, retries } = final.payload.performance.worker;
  assert.equal(retries, 1, "the 503 was retried once");
  assert.ok(bucketsMs.backoff >= 400, `the retry wait is backoff, not response time: ${bucketsMs.backoff}`);
  assert.ok(bucketsMs.pacing >= 60, `the deliberate delay is pacing: ${bucketsMs.pacing}`);
  assert.ok(bucketsMs.response > 0, "waiting for response headers is measured");
  // A private-network fixture resolves without DNS, so the guard rounds to 0ms here; what this
  // asserts is that the bucket exists and never absorbs the network time next to it.
  assert.ok(bucketsMs.urlGuard < bucketsMs.response, `address checks stay their own bucket: ${bucketsMs.urlGuard}`);
  assert.ok(bucketsMs.backoff <= final.payload.performance.worker.totalMs);
  assert.ok(requestsByKind.listing.backoffMs >= 400, "the retry is charged to the listing request that failed");
  assert.equal(requestsByKind.detail.backoffMs, 0, "the detail request never retried");
});

test("a warm Workday run costs no detail requests and says so in its metrics", async () => {
  jobCount = 1; seen.length = 0;
  const known = [listingHash("/detail/0", "Engineer 0", undefined)];
  const final = finalOf(await scrape({}, { known }));
  assert.equal(final.event, "completed", JSON.stringify(final));
  assert.equal(kinds(final).detail.count, 0, "the listing was recognised, so its detail page was never read");
  assert.equal(seen.some(request => request.url.startsWith("/detail/")), false);
});

// Arm addresses its employer's own host, so its warm path is exercised through the injected
// request function rather than the fixture server; the invariant measured is the same one.
test("a warm Arm run costs no detail requests", async () => {
  const listing = `<ul><li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/known/1/7">Known Engineer</a><span class="location">Lisbon</span></li></ul>
   <select id="pagination-current-bottom"><option value="1">1</option></select>`;
  const fetched = [];
  const request = async url => { fetched.push(url); return { text: listing, url, type: "text/html" } };
  const result = await scrapeCompany("arm", { request, maxPages: 1, hashListing: key => `hash:${key}`,
    known: new Set(["hash:https://careers.arm.com/job/known/1/7"]) });
  assert.equal(result.jobs.length, 0);
  assert.equal(fetched.filter(url => url.includes("/job/known/")).length, 0);
  assert.equal(fetched.length, 1, "one listing page and nothing else");
});

test("the metric payload stays a fixed size as the number of jobs grows", async () => {
  jobCount = 1;
  const one = finalOf(await scrape({ pageSize: 1, maxPages: 2 })).payload.performance;
  jobCount = 40;
  const many = finalOf(await scrape({ pageSize: 40 })).payload.performance;
  assert.equal(shape(many), shape(one), "no per-job sample is retained");
  assert.equal(numbers(many).length, numbers(one).length);
  assert.ok(JSON.stringify(many).length < JSON.stringify(one).length + 200, "40 jobs cost no extra payload beyond larger numbers");
});

// The reason jobs are published as they are accepted rather than after the last page. A board
// that dies on page three used to emit nothing at all: the worker buffered every listing and
// only emitted once the whole read succeeded, so a failure or a cancellation late in a long
// board discarded everything already read. What was read has to survive the failure.
test("a board that fails partway through still delivers the pages it did read", async () => {
  jobCount = 6; seen.length = 0; failFromOffset = 4;
  const events = await scrape({ pageSize: 2 });
  failFromOffset = null;
  const final = finalOf(events);
  assert.equal(final.event, "failed", JSON.stringify(final));
  const jobs = events.filter(event => event.event === "job");
  assert.equal(jobs.length, 4, "both pages that were read are delivered, not discarded");
  assert.deepEqual(jobs.map(event => event.payload.externalId), ["job-0", "job-1", "job-2", "job-3"]);
  // Availability reconciliation keys off this, so a partial read must never claim to be whole.
  assert.equal(final.payload.complete, false, "a partial read never reports itself complete");
  assert.ok(events.indexOf(jobs.at(-1)) < events.indexOf(final), "jobs arrive before the terminal event");
});

// The backstop for every adapter, including the ones whose completeness logic nobody re-read.
// A board reporting zero openings that also calls itself complete is the single input that
// reconciles every stored job for that source to closed, and a broken read looks exactly like it.
test("a board that reads as empty is reported unfinished, not as an empty board", async () => {
  jobCount = 0; seen.length = 0;
  const final = finalOf(await scrape({ pageSize: 2 }));
  assert.equal(final.event, "completed", JSON.stringify(final));
  assert.equal(final.payload.discovered, 0);
  assert.equal(final.payload.complete, false, "nothing read proves nothing about what closed");
  assert.ok(final.payload.warnings.some(warning => warning.includes("no listings at all")), final.payload.warnings.join(" | "));
  jobCount = 1;
  assert.equal(finalOf(await scrape()).payload.complete, true, "a board with listings still completes normally");
});
