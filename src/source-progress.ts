import { useSyncExternalStore } from "react";
import type { WorkerEvent } from "./api";
import { describeFailure, describeLogEvent, duration } from "./source-ui";

// Per-source progress for the Sources tab. Lanes read several sources at once, so the worker's
// events arrive interleaved; folding them by source turns one unreadable log into one bar per row.

export type RowPhase = "queued" | "checking" | "reading" | "saving" | "testing"
  | "read" | "unchanged" | "tested" | "failed" | "cancelled";
export type RowProgress = {
  phase: RowPhase; batchId?: string | null; runId?: string; requests: number;
  current?: number; total?: number; startedAt: number; finishedAt?: number;
  found?: number; saved?: number; message?: string;
  /** The site answered 429 and the worker is backing off and reading it more slowly. */
  throttled?: boolean;
};
export type BatchProgress = { id: string; total: number; completed: number; failed: number; startedAt: number; finishedAt?: number };
export type LogLine = { id: number; at: number; runId: string; source: string; text: string; tone: "ok" | "warning" | "error" | "dim" };
export type ProgressState = { rows: Record<string, RowProgress>; batch?: BatchProgress; log: LogLine[]; nextLogId: number };
export type BatchEvent = { runId: string; phase?: string; sourceIds?: string[]; sourceId?: string; outcome?: string; completed?: number; total?: number; failed?: number };

export const emptyProgress = (): ProgressState => ({ rows: {}, log: [], nextLogId: 0 });
export const isRunning = (row?: RowProgress) =>
  !!row && ["queued", "checking", "reading", "saving", "testing"].includes(row.phase);

const finishedPhase = (outcome?: string): RowPhase | undefined =>
  outcome === "read" ? "read" : outcome === "unchanged" ? "unchanged" : outcome === "failed" ? "failed" : outcome === "cancelled" ? "cancelled" : undefined;

// The commands that belong on a source's row. A probe of a source being added has no row yet, and
// a session capture is a browser window the user is already looking at.
const rowCommands = new Set(["check_source", "scrape_source", "test_source"]);

function withLog(state: ProgressState, event: WorkerEvent, at: number): ProgressState {
  const command = event.command ?? String(event.payload.command ?? "");
  // Progress ticks live on the row now. The change check before each read only matters when it fails.
  if (event.event === "progress" || event.event === "job") return state;
  if (command === "check_source" && event.event !== "failed") return state;
  const tone = event.event === "failed" ? "error" : event.event === "warning" || event.event === "needs_user_action" ? "warning" : event.event === "completed" ? "ok" : "dim";
  const line: LogLine = { id: state.nextLogId, at, runId: event.runId, source: event.sourceName || "Source", text: describeLogEvent(event), tone };
  return { ...state, log: [...state.log, line].slice(-300), nextLogId: state.nextLogId + 1 };
}

export function reduceWorkerEvent(state: ProgressState, event: WorkerEvent, now: number): ProgressState {
  const logged = withLog(state, event, now);
  const id = event.sourceId, command = event.command ?? "";
  if (!id || !rowCommands.has(command)) return logged;
  const previous = logged.rows[id], payload = event.payload;
  const batchId = event.batchId ?? null;
  let row: RowProgress | undefined = previous;
  if (event.event === "started") {
    const phase: RowPhase = command === "check_source" ? "checking" : command === "test_source" ? "testing" : "reading";
    row = { phase, batchId, runId: event.runId, requests: 0, startedAt: now };
  } else if (previous?.runId !== event.runId) {
    return logged; // A late event from a run this row has already moved past.
  } else if (event.event === "warning") {
    if (payload.code !== "rate_limited_backoff") return logged;
    row = { ...previous, throttled: true };
  } else if (event.event === "progress") {
    const requests = Number(payload.requests ?? previous.requests) || previous.requests;
    if (payload.phase === "saving" && command === "scrape_source") {
      row = { ...previous, phase: "saving", requests, current: Number(payload.current ?? 0), total: Number(payload.total ?? 0) };
    } else row = { ...previous, requests };
  } else if (event.event === "completed") {
    const requests = Number(payload.requests ?? previous.requests) || previous.requests;
    // In a batch the check only decides whether to read; the batch says how the source ended.
    if (command === "check_source") row = batchId ? { ...previous, requests } : undefined;
    else row = { ...previous, phase: command === "test_source" ? "tested" : "read", requests, finishedAt: now,
      found: Number(payload.discovered ?? 0), saved: Number(payload.persisted ?? 0), current: undefined, total: undefined };
  } else if (event.event === "failed" || event.event === "cancelled") {
    // A failed check in a batch still goes on to a full read, so the row keeps waiting for it.
    if (command === "check_source" && batchId && event.event === "failed") return logged;
    row = { ...previous, phase: event.event === "failed" ? "failed" : "cancelled", finishedAt: now,
      message: event.event === "failed" ? String(payload.message ?? describeFailure(payload.code as string | undefined)) : undefined };
  }
  const rows = { ...logged.rows };
  if (row) rows[id] = row; else delete rows[id];
  return { ...logged, rows };
}

export function reduceBatchEvent(state: ProgressState, event: BatchEvent, now: number): ProgressState {
  if (event.phase === "queued") {
    const rows = { ...state.rows };
    for (const id of event.sourceIds ?? []) rows[id] = { phase: "queued", batchId: event.runId, requests: 0, startedAt: now };
    const total = event.total ?? event.sourceIds?.length ?? 0;
    return { ...state, rows, batch: { id: event.runId, total, completed: 0, failed: 0, startedAt: now, finishedAt: total ? undefined : now } };
  }
  if (event.phase) return state; // source-progress ticks duplicate the per-source scrape events.
  const batch: BatchProgress = state.batch?.id === event.runId
    ? { ...state.batch } : { id: event.runId, total: 0, completed: 0, failed: 0, startedAt: now };
  batch.completed = event.completed ?? batch.completed;
  batch.total = event.total ?? batch.total;
  batch.failed = event.failed ?? batch.failed;
  if (batch.total && batch.completed >= batch.total) batch.finishedAt = now;
  const rows = { ...state.rows }, phase = finishedPhase(event.outcome);
  if (event.sourceId && phase) {
    const row = rows[event.sourceId];
    rows[event.sourceId] = { ...(row ?? { requests: 0, startedAt: now }), phase, batchId: event.runId, finishedAt: now,
      current: undefined, total: undefined,
      // A read that ended without the whole board is a failure for the batch, though the worker said "completed".
      message: phase !== "failed" ? undefined : row?.message ?? (row?.phase === "read" ? "Only part of the board could be read. Try again later." : undefined) };
  }
  return { ...state, rows, batch };
}

/**
 * How far along a row is, 0..1, or null when there is no honest number to give. Reading a board
 * has no total up front, so the bar estimates from how long this source usually takes and never
 * claims more than 95% until the read actually ends.
 */
export function rowFraction(row: RowProgress, now: number, typicalMs?: number): number | null {
  switch (row.phase) {
    case "queued": return 0;
    case "saving": return row.total ? Math.min(1, (row.current ?? 0) / row.total) : null;
    case "reading": case "testing":
      return typicalMs && typicalMs > 0 ? Math.min(0.95, Math.max(0.02, (now - row.startedAt) / typicalMs)) : null;
    case "checking": return null;
    default: return 1;
  }
}

const plural = (count: number, word: string) => `${count.toLocaleString()} ${word}${count === 1 ? "" : "s"}`;

/** The one line under a source's name while it runs, and right after. */
export function describeRow(row: RowProgress, now: number) {
  const elapsed = duration((row.finishedAt ?? now) - row.startedAt);
  const requests = row.requests ? ` · ${plural(row.requests, "request")}` : "";
  switch (row.phase) {
    case "queued": return "Queued";
    case "checking": return `Checking for changes${requests}`;
    case "reading": return `${row.throttled ? "Reading slowly, the site asked us to" : "Reading the board"}${requests} · ${elapsed}`;
    case "testing": return `Testing${requests} · ${elapsed}`;
    case "saving": return `Saving ${(row.current ?? 0).toLocaleString()} of ${(row.total ?? 0).toLocaleString()}`;
    case "read": return row.found === undefined ? "Read" : `Read · ${plural(row.found, "listing")} · ${elapsed}`;
    case "tested": return `Test found ${plural(row.found ?? 0, "listing")} · nothing stored`;
    case "unchanged": return "No changes since the last read";
    case "failed": return row.message || describeFailure();
    case "cancelled": return "Cancelled";
  }
}

// ---- the store ------------------------------------------------------------------------------
// Module-level so it survives the Sources tab being mounted late: the startup update begins
// before anyone opens that tab, and its rows should already be filled in when they do.
let state = emptyProgress();
const listeners = new Set<() => void>();
const publish = (next: ProgressState) => { if (next !== state) { state = next; listeners.forEach(listener => listener()) } };
export const progressStore = {
  get: () => state,
  subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
  worker: (event: WorkerEvent) => publish(reduceWorkerEvent(state, event, Date.now())),
  batch: (event: BatchEvent) => publish(reduceBatchEvent(state, event, Date.now())),
  clearLog: () => publish({ ...state, log: [] }),
  // Clears a finished update's summary and its finished rows; anything still running stays.
  dismissBatch: () => publish({ ...state, batch: undefined, rows: Object.fromEntries(Object.entries(state.rows).filter(([, row]) => isRunning(row))) }),
  reset: () => publish(emptyProgress()),
};
export const useSourceProgress = () => useSyncExternalStore(progressStore.subscribe, progressStore.get);

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;
/** Started once by the App; returns the cleanup. */
export function feedProgressStore(listen: Listen) {
  let active = true;
  const stops = [
    listen<WorkerEvent>("scrape-event", event => { if (active) progressStore.worker(event.payload) }),
    listen<BatchEvent>("scrape-all-progress", event => { if (active) progressStore.batch(event.payload) }),
  ];
  return () => { active = false; stops.forEach(stop => stop.then(unlisten => unlisten()).catch(() => {})) };
}
