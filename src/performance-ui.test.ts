import { describe, expect, it } from "vitest";
import type { PerformanceHistory, PerformanceRun } from "./api";
import { aggregateRows, formatDuration, latestSummary, phaseLabel, phaseRows, recentFailures, requestKindRows } from "./performance-ui";

const run = (overrides: Partial<PerformanceRun> = {}): PerformanceRun => ({
  id: "log-1",
  at: "2026-08-31T09:15:42.100Z",
  sourceId: "src-amd",
  sourceName: "AMD",
  action: "scrape_source",
  outcome: "completed",
  totalMs: 12_400,
  workerMs: 11_800,
  requests: 7,
  pages: 3,
  jobs: 120,
  slowestPhase: { key: "worker.response", milliseconds: 6_100 },
  phases: {
    "worker.response": 6_100, "worker.pacing": 3_000, "worker.body": 400, "worker.emit": 0,
    "app.prepare": 40, "app.startup": 300, "app.active": 11_900, "app.finalize": 160,
    "work.jobPersistence": 120, "work.knownHashLoad": 30,
  },
  requestsByKind: {
    robots: { count: 1, pacingMs: 0, backoffMs: 0, responseMs: 90, bodyMs: 5 },
    listing: { count: 6, pacingMs: 3_000, backoffMs: 0, responseMs: 6_010, bodyMs: 395 },
    detail: { count: 0, pacingMs: 0, backoffMs: 0, responseMs: 0, bodyMs: 0 },
  },
  performance: { version: 1 },
  ...overrides,
});

describe("empty and legacy history", () => {
  it("shows nothing rather than zeroes when no run carries metrics", () => {
    expect(latestSummary(undefined)).toBeUndefined();
    expect(latestSummary({ recent: [], aggregates: [] })).toBeUndefined();
    expect(aggregateRows({ recent: [], aggregates: [] })).toEqual([]);
    expect(recentFailures(undefined)).toEqual([]);
  });
  it("survives a run whose metric groups are missing entirely", () => {
    const legacy = run({ phases: {}, requestsByKind: {}, slowestPhase: undefined });
    expect(phaseRows(legacy, "worker")).toEqual([]);
    expect(requestKindRows(legacy)).toEqual([]);
    expect(latestSummary({ recent: [legacy], aggregates: [] })?.slowest).toBeUndefined();
  });
});

describe("labels and durations", () => {
  it("names each wire key in words a reader recognises", () => {
    expect(phaseLabel("worker.adapterOther")).toBe("Parsing and transformation");
    expect(phaseLabel("app.startup")).toBe("Worker startup");
    expect(phaseLabel("work.jobPersistence")).toBe("Job writes");
  });
  it("falls back to the key's own words for a bucket it has never seen", () => {
    expect(phaseLabel("work.futureThing")).toBe("Future thing");
  });
  it("formats milliseconds, seconds and minutes", () => {
    expect(formatDuration(940)).toBe("940ms");
    expect(formatDuration(12_400)).toBe("12.4s");
    expect(formatDuration(125_000)).toBe("2m 5s");
  });
});

describe("latest run summary", () => {
  it("reports the total, the slowest step and the work done", () => {
    const summary = latestSummary({ recent: [run()], aggregates: [] })!;
    expect(summary.total).toBe("12.4s");
    expect(summary.slowest).toEqual({ key: "worker.response", label: "Site responses", milliseconds: 6_100 });
    expect(summary).toMatchObject({ action: "scrape", sourceName: "AMD", requests: 7, pages: 3, jobs: 120 });
    expect(summary.at).toBe("2026-08-31 09:15:42");
  });
});

describe("breakdowns", () => {
  it("ranks the slowest measured cost first and drops empty buckets", () => {
    expect(phaseRows(run(), "worker").map(row => row.label))
      .toEqual(["Site responses", "Request pacing", "Response bodies"]);
    expect(phaseRows(run(), "work")[0]).toEqual({ key: "work.jobPersistence", label: "Job writes", milliseconds: 120 });
  });
  it("keeps request kinds that never fired out of the table", () => {
    expect(requestKindRows(run()).map(row => row.kind)).toEqual(["listing", "robots"]);
    expect(requestKindRows(run())[0].count).toBe(6);
  });
});

describe("aggregates and failures", () => {
  const history: PerformanceHistory = {
    recent: [run({ id: "a", outcome: "failed" }), run({ id: "b", outcome: "cancelled" }), run({ id: "c" })],
    aggregates: [
      { sourceId: "src-amd", sourceName: "AMD", action: "scrape_source", samples: 6, medianMs: 12_400, p95Ms: 19_000, slowestPhase: { key: "worker.response", milliseconds: 6_100 } },
      { sourceId: "src-arm", sourceName: "Arm", action: "check_source", samples: 3, medianMs: 900, slowestPhase: { key: "worker.body", milliseconds: 200 } },
    ],
  };
  it("keeps failures out of the aggregates and lists them separately", () => {
    expect(recentFailures(history).map(entry => entry.id)).toEqual(["a", "b"]);
    expect(aggregateRows(history).every(row => row.samples > 0)).toBe(true);
  });
  it("hides p95 until five samples exist", () => {
    const rows = aggregateRows(history);
    expect(rows[0]).toMatchObject({ label: "AMD", action: "scrape", median: "12.4s", p95: "19.0s", slowest: "Site responses" });
    expect(rows[1].p95).toBeUndefined();
  });
});
