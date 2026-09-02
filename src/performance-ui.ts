import type { PerformanceAggregate, PerformanceHistory, PerformanceRun } from "./api";
import { duration } from "./source-ui";

export { duration as formatDuration };

// Metric keys are wire names; nobody reading Diagnostics should have to decode "adapterOther".
// Anything unmapped falls back to its own words so a new bucket stays readable without a release.
const phaseLabels: Record<string, string> = {
  "worker.setup": "Worker setup",
  "worker.urlGuard": "Address checks",
  "worker.pacing": "Request pacing",
  "worker.backoff": "Retry waits",
  "worker.response": "Site responses",
  "worker.body": "Response bodies",
  "worker.browser": "Browser automation",
  "worker.adapterOther": "Parsing and transformation",
  "worker.emit": "Worker output",
  "app.prepare": "Preparation",
  "app.startup": "Worker startup",
  "app.active": "Worker running",
  "app.shutdown": "Worker shutdown",
  "app.finalize": "Finalization",
  "work.configLoad": "Configuration load",
  "work.filterLoad": "Filter load",
  "work.sessionLoad": "Browser session load",
  "work.knownHashLoad": "Known-job lookup",
  "work.jobPersistence": "Job writes",
  "work.sightingPersistence": "Sighting writes",
  "work.enrichmentPersistence": "Description writes",
  "work.runRecord": "Run record",
  "work.availability": "Availability update",
  "work.sourceChecks": "Source checks",
  "work.preflight": "Preflight checks",
  "work.scrape": "Scraping",
  "work.enrichment": "Description downloads",
};
export const phaseLabel = (key: string) =>
  phaseLabels[key] ??
  key.split(".").pop()!.replace(/([a-z])([A-Z])/g, (_, a, b) => `${a} ${b.toLowerCase()}`)
    .replace(/^./, c => c.toUpperCase());

export type PhaseRow = { key: string; label: string; milliseconds: number };
// Nested work never sums to its parent phase, so a breakdown only ranks within one group.
export const phaseRows = (run: PerformanceRun, group: "worker" | "app" | "work"): PhaseRow[] =>
  Object.entries(run.phases ?? {})
    .filter(([key, milliseconds]) => key.startsWith(`${group}.`) && milliseconds > 0)
    .map(([key, milliseconds]) => ({ key, label: phaseLabel(key), milliseconds }))
    .sort((a, b) => b.milliseconds - a.milliseconds || a.key.localeCompare(b.key));

export type RequestKindRow = { kind: string; count: number; responseMs: number; bodyMs: number; pacingMs: number; backoffMs: number };
export const requestKindRows = (run: PerformanceRun): RequestKindRow[] =>
  Object.entries(run.requestsByKind ?? {})
    .filter(([, kind]) => Number(kind?.count) > 0)
    .map(([kind, value]) => ({
      kind,
      count: Number(value.count) || 0,
      responseMs: Number(value.responseMs) || 0,
      bodyMs: Number(value.bodyMs) || 0,
      pacingMs: Number(value.pacingMs) || 0,
      backoffMs: Number(value.backoffMs) || 0,
    }))
    .sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind));

export type LatestSummary = { at: string; action: string; sourceName: string; outcome: string; total: string; slowest?: PhaseRow; requests: number; pages: number; jobs: number };
export function latestSummary(history?: PerformanceHistory): LatestSummary | undefined {
  const run = history?.recent?.[0];
  if (!run) return undefined;
  const slowest = run.slowestPhase && run.slowestPhase.milliseconds > 0
    ? { key: run.slowestPhase.key, label: phaseLabel(run.slowestPhase.key), milliseconds: run.slowestPhase.milliseconds }
    : undefined;
  return {
    at: run.at.replace("T", " ").slice(0, 19),
    action: run.action.replace("_source", "").replace(/_/g, " "),
    sourceName: run.sourceName ?? "all sources",
    outcome: run.outcome,
    total: duration(run.totalMs),
    slowest,
    requests: run.requests,
    pages: run.pages,
    jobs: run.jobs,
  };
}

// Failures say nothing about how long the work normally takes, so they stay out of the medians
// (the Rust side already groups only completed runs) and get their own short list instead.
export const recentFailures = (history?: PerformanceHistory): PerformanceRun[] =>
  (history?.recent ?? []).filter(run => run.outcome !== "completed");

export type AggregateRow = { label: string; action: string; samples: number; median: string; p95?: string; slowest?: string };
export const aggregateRows = (history?: PerformanceHistory): AggregateRow[] =>
  (history?.aggregates ?? []).map((row: PerformanceAggregate) => ({
    label: row.sourceName ?? "all sources",
    action: row.action.replace("_source", "").replace(/_/g, " "),
    samples: row.samples,
    median: duration(row.medianMs),
    p95: row.p95Ms === undefined || row.p95Ms === null ? undefined : duration(row.p95Ms),
    slowest: row.slowestPhase ? phaseLabel(row.slowestPhase.key) : undefined,
  }));
