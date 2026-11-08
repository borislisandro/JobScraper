import { describe, expect, it } from "vitest";
import type { WorkerEvent } from "./api";
import { describeRow, emptyProgress, isRunning, reduceBatchEvent, reduceWorkerEvent, rowFraction, type ProgressState } from "./source-progress";

const ev = (event: string, sourceId: string, command: string, runId: string, payload: Record<string, unknown> = {}, batchId: string | null = "batch"): WorkerEvent =>
  ({ event, runId, payload, sourceId, sourceName: sourceId.toUpperCase(), command, batchId });
const fold = (events: ((state: ProgressState) => ProgressState)[]) => events.reduce((state, step) => step(state), emptyProgress());

describe("per-source progress", () => {
  it("keeps parallel sources apart, from queued through read and unchanged", () => {
    const state = fold([
      s => reduceBatchEvent(s, { runId: "batch", phase: "queued", sourceIds: ["a", "b"], total: 2 }, 0),
      s => reduceWorkerEvent(s, ev("started", "a", "check_source", "a1"), 10),
      s => reduceWorkerEvent(s, ev("started", "b", "scrape_source", "b1"), 11),
      s => reduceWorkerEvent(s, ev("completed", "a", "check_source", "a1", { requests: 1 }), 20),
      s => reduceWorkerEvent(s, ev("progress", "b", "scrape_source", "b1", { phase: "fetching", requests: 7 }), 30),
    ]);
    expect(state.rows.a.phase).toBe("checking");
    expect(state.rows.b).toMatchObject({ phase: "reading", requests: 7 });
    const later = fold([
      () => state,
      s => reduceBatchEvent(s, { runId: "batch", sourceId: "a", outcome: "unchanged", completed: 1, total: 2 }, 40),
      s => reduceWorkerEvent(s, ev("progress", "b", "scrape_source", "b1", { phase: "saving", current: 5, total: 10 }), 50),
    ]);
    expect(later.rows.a.phase).toBe("unchanged");
    expect(rowFraction(later.rows.b, 50)).toBe(0.5);
    const done = fold([
      () => later,
      s => reduceWorkerEvent(s, ev("completed", "b", "scrape_source", "b1", { discovered: 12, persisted: 3, requests: 9 }), 60),
      s => reduceBatchEvent(s, { runId: "batch", sourceId: "b", outcome: "read", completed: 2, total: 2 }, 61),
    ]);
    expect(done.rows.b).toMatchObject({ phase: "read", found: 12, saved: 3 });
    expect(done.batch).toMatchObject({ completed: 2, total: 2, finishedAt: 61 });
    expect(Object.values(done.rows).some(isRunning)).toBe(false);
  });

  it("does not end a row on a failed change check, which still leads to a full read", () => {
    const state = fold([
      s => reduceWorkerEvent(s, ev("started", "a", "check_source", "a1"), 0),
      s => reduceWorkerEvent(s, ev("failed", "a", "check_source", "a1", { message: "timeout" }), 5),
    ]);
    expect(state.rows.a.phase).toBe("checking");
    expect(state.log.map(line => line.source)).toEqual(["A"]);
  });

  it("reports a failed read with its reason, and ignores late events from an earlier run", () => {
    const state = fold([
      s => reduceWorkerEvent(s, ev("started", "a", "scrape_source", "a2"), 0),
      s => reduceWorkerEvent(s, ev("progress", "a", "scrape_source", "a1", { phase: "fetching", requests: 99 }), 1),
      s => reduceWorkerEvent(s, ev("failed", "a", "scrape_source", "a2", { message: "Blocked" }), 2),
    ]);
    expect(state.rows.a).toMatchObject({ phase: "failed", requests: 0, message: "Blocked" });
    expect(describeRow(state.rows.a, 3)).toBe("Blocked");
  });

  it("estimates a board read from how long it usually takes, never past 95%", () => {
    const state = reduceWorkerEvent(emptyProgress(), ev("started", "a", "scrape_source", "a1"), 0);
    expect(rowFraction(state.rows.a, 5_000, 10_000)).toBe(0.5);
    expect(rowFraction(state.rows.a, 60_000, 10_000)).toBe(0.95);
    expect(rowFraction(state.rows.a, 5_000)).toBeNull();
  });

  it("logs one labelled line per meaningful event and none for progress ticks", () => {
    const state = fold([
      s => reduceWorkerEvent(s, ev("started", "a", "scrape_source", "a1", { command: "scrape_source", adapter: "workday" }), 0),
      s => reduceWorkerEvent(s, ev("progress", "a", "scrape_source", "a1", { phase: "fetching", requests: 3 }), 1),
      s => reduceWorkerEvent(s, ev("started", "b", "check_source", "b1"), 2),
      s => reduceWorkerEvent(s, ev("completed", "a", "scrape_source", "a1", { discovered: 2, persisted: 2, requests: 3 }), 3),
    ]);
    expect(state.log.map(line => `${line.source}: ${line.tone}`)).toEqual(["A: dim", "A: ok"]);
  });

  it("drops the row of a standalone change check once it completes", () => {
    const state = fold([
      s => reduceWorkerEvent(s, ev("started", "a", "check_source", "a1", {}, null), 0),
      s => reduceWorkerEvent(s, ev("completed", "a", "check_source", "a1", {}, null), 1),
    ]);
    expect(state.rows.a).toBeUndefined();
  });

  it("says when a site has asked us to slow down", () => {
    const state = fold([
      s => reduceWorkerEvent(s, ev("started", "q", "scrape_source", "q1"), 0),
      s => reduceWorkerEvent(s, ev("warning", "q", "scrape_source", "q1", { code: "rate_limited_backoff", message: "slow down" }), 1_000),
    ]);
    expect(state.rows.q.throttled).toBe(true);
    expect(describeRow(state.rows.q, 2_000)).toBe("Reading slowly, the site asked us to · 2.0s");
  });
});
