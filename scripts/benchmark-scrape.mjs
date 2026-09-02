#!/usr/bin/env node
// Read-only scrape benchmark. It drives sidecar/worker.mjs the way the app does and reports the
// performance metrics the worker already emits. Nothing here writes: the database is opened
// read-only for configuration and stored listing hashes, and no job, run or source state is saved.
//
//   node scripts/benchmark-scrape.mjs              fixtures + live checks + warm updates
//   node scripts/benchmark-scrape.mjs --fixtures   deterministic fixtures only, no network
//   node scripts/benchmark-scrape.mjs --json       machine-readable results as well
import { DatabaseSync } from "node:sqlite";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { join } from "node:path";

const argv = new Set(process.argv.slice(2));
const LANES = 5, FIXTURE_RUNS = 7, WARM_SOURCES = ["AMD", "Intel", "Arm"];
const dbPath = join(process.env.LOCALAPPDATA ?? "", "JobScraper-dev", "jobscraper.db");

function worker(input) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["sidecar/worker.mjs"], { cwd: process.cwd() });
    const lines = [];
    child.stdout.on("data", data => lines.push(...String(data).trim().split("\n").filter(Boolean)));
    child.on("error", reject);
    child.on("close", () => {
      const events = lines.map(line => { try { return JSON.parse(line) } catch { return null } }).filter(Boolean);
      resolve(events.at(-1) ?? { event: "failed", payload: { message: "no worker output" } });
    });
    child.stdin.end(JSON.stringify({ protocolVersion: 1, ...input }) + "\n");
  });
}
const median = values => {
  const sorted = [...values].sort((a, b) => a - b);
  if (!sorted.length) return 0;
  const middle = sorted.length >> 1;
  return sorted.length % 2 ? sorted[middle] : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
};
const buckets = terminal => terminal.payload?.performance?.worker?.bucketsMs ?? {};
const slowest = terminal => Object.entries(buckets(terminal))
  .filter(([, value]) => value > 0).sort((a, b) => b[1] - a[1])[0] ?? ["—", 0];
const requestSummary = terminal => Object.entries(terminal.payload?.performance?.worker?.requestsByKind ?? {})
  .filter(([, kind]) => kind.count > 0).map(([kind, value]) => `${kind} ${value.count}`).join(" + ") || "none";
const row = (label, action, terminal) => ({
  label, action,
  outcome: terminal.event === "completed" ? "ok" : (terminal.payload?.code ?? terminal.event),
  totalMs: terminal.payload?.performance?.worker?.totalMs ?? 0,
  setupMs: buckets(terminal).setup ?? 0,
  slowest: `${slowest(terminal)[0]} ${slowest(terminal)[1]}ms`,
  requests: requestSummary(terminal),
  pages: terminal.payload?.pages ?? terminal.payload?.pagesRead ?? 0,
  jobs: terminal.payload?.discovered ?? terminal.payload?.fresh ?? 0,
});

// The deterministic half: one local fixture host, the same board every run, so the numbers move
// only when the worker changes. Repeated FIXTURE_RUNS times and reported as medians.
async function fixtureRuns() {
  const server = createServer(async (req, res) => {
    let body = ""; for await (const part of req) body += part;
    if (req.url === "/robots.txt") return res.setHeader("content-type", "text/plain").end("User-agent: *\nAllow: /\n");
    if (req.url.startsWith("/detail/")) return res.setHeader("content-type", "application/json").end(JSON.stringify({ description: "detail text" }));
    const { limit = 20, offset = 0 } = JSON.parse(body || "{}");
    const postings = Array.from({ length: 100 }, (_, index) => index).slice(offset, offset + limit)
      .map(index => ({ id: `job-${index}`, jobReqId: `job-${index}`, title: `Engineer ${index}`, detailUrl: `/detail/${index}` }));
    res.setHeader("content-type", "application/json").end(JSON.stringify({ total: 100, jobPostings: postings }));
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const source = {
    id: "fixture", name: "Fixture", baseUrl: base, adapterId: "workday", kind: "active",
    allowPrivateNetwork: true, robotsOverride: false,
    configJson: { testNoDelay: true, maxPages: 5, pageSize: 20, listingPath: "/workday", tenant: "fixture", site: "External" },
  };
  const cases = [
    ["fixture cold (100 jobs, details)", { command: "scrape_source", source }],
    ["fixture listings only", { command: "scrape_source", deferDetails: true, source }],
    ["fixture check", { command: "check_source", source }],
  ];
  const results = [];
  for (const [label, input] of cases) {
    const runs = [];
    for (let attempt = 0; attempt < FIXTURE_RUNS; attempt++) runs.push(await worker({ runId: `bench-${attempt}`, ...input }));
    // Every reported number is a median across the runs, including the ranking of the buckets,
    // so the slowest step named here cannot come from a different run than the total.
    const bucketMedians = Object.fromEntries(Object.keys(buckets(runs[0]))
      .map(bucket => [bucket, median(runs.map(run => buckets(run)[bucket] ?? 0))]));
    const worst = Object.entries(bucketMedians).filter(([, value]) => value > 0).sort((a, b) => b[1] - a[1])[0] ?? ["—", 0];
    results.push({
      ...row(label, input.command.replace("_source", ""), runs[0]),
      totalMs: median(runs.map(run => run.payload?.performance?.worker?.totalMs ?? 0)),
      setupMs: bucketMedians.setup ?? 0,
      slowest: `${worst[0]} ${worst[1]}ms`,
    });
  }
  server.close();
  return results;
}

// The live half: one request stream per domain, LANES domains at a time, current pacing intact.
async function lanes(tasks) {
  const byDomain = new Map();
  for (const task of tasks) {
    const domain = new URL(task.source.baseUrl).hostname;
    if (!byDomain.has(domain)) byDomain.set(domain, []);
    byDomain.get(domain).push(task);
  }
  const queues = [...byDomain.values()], results = [];
  const lane = async () => {
    for (let queue = queues.shift(); queue; queue = queues.shift()) {
      for (const task of queue) {
        const terminal = await worker({ runId: `bench-${task.label}`, command: task.command, known: task.known, deferDetails: task.deferDetails, source: task.source });
        results.push(row(task.label, task.action, terminal));
        process.stderr.write(`  ${task.label} · ${task.action}: ${terminal.event}\n`);
      }
    }
  };
  await Promise.all(Array.from({ length: LANES }, lane));
  return results;
}

function liveTasks() {
  const db = new DatabaseSync(dbPath, { readOnly: true });
  const sources = db.prepare(`SELECT s.id,s.name,s.base_url,s.adapter_id,s.kind,s.robots_override,s.allow_private_network,c.config_json
    FROM sources s LEFT JOIN source_configs c ON c.source_id=s.id
    WHERE s.enabled=1 AND s.kind='active' AND s.deleted_at IS NULL ORDER BY s.name`).all();
  const hashes = db.prepare("SELECT listing_hash FROM jobs WHERE source_id=? AND listing_hash IS NOT NULL");
  const tasks = [];
  for (const record of sources) {
    const source = {
      id: record.id, name: record.name, baseUrl: record.base_url, adapterId: record.adapter_id, kind: record.kind,
      robotsOverride: Boolean(record.robots_override), allowPrivateNetwork: Boolean(record.allow_private_network),
      configJson: JSON.parse(record.config_json || "{}"),
    };
    tasks.push({ label: record.name, action: "check", command: "check_source", source, known: [] });
    if (WARM_SOURCES.includes(record.name)) {
      const known = hashes.all(record.id).map(entry => entry.listing_hash);
      // A warm update reads listings only: stored hashes suppress the detail fetch, and
      // deferDetails keeps descriptions out of it entirely.
      tasks.push({ label: `${record.name} (${known.length} known)`, action: "warm update", command: "scrape_source", deferDetails: true, source, known });
    }
  }
  db.close();
  return tasks;
}

const table = rows => {
  const columns = ["label", "action", "outcome", "totalMs", "setupMs", "slowest", "requests", "pages", "jobs"];
  const widths = columns.map(column => Math.max(column.length, ...rows.map(row => String(row[column]).length)));
  const line = cells => `| ${cells.map((cell, index) => String(cell).padEnd(widths[index])).join(" | ")} |`;
  return [line(columns), line(widths.map(width => "-".repeat(width))), ...rows.map(row => line(columns.map(column => row[column])))].join("\n");
};

const results = { fixtures: await fixtureRuns(), live: [] };
if (!argv.has("--fixtures")) {
  const tasks = liveTasks();
  process.stderr.write(`Live: ${tasks.length} runs over ${new Set(tasks.map(task => new URL(task.source.baseUrl).hostname)).size} domains, ${LANES} lanes\n`);
  results.live = await lanes(tasks);
}
console.log(`\nDeterministic fixtures (median of ${FIXTURE_RUNS} runs)\n${table(results.fixtures)}`);
if (results.live.length) console.log(`\nLive sources (read-only, one stream per domain)\n${table(results.live.sort((a, b) => b.totalMs - a.totalMs))}`);
if (argv.has("--json")) console.log(`\n${JSON.stringify(results, null, 2)}`);
