import { describe, expect, it } from "vitest";
import { describeCheckSummary, describeFailure, describeLogEvent, describeOutcome, describeUpdateSummary, previewFromEvents, probeVerdict, supportsSessionCapture } from "./source-ui";

describe("source preview and session controls", () => {
  it("only offers headed capture to browser-capable adapters", () => {
    expect(supportsSessionCapture("workday")).toBe(true);
    expect(supportsSessionCapture("playwright")).toBe(true);
    expect(supportsSessionCapture("rss")).toBe(false);
  });
  it("keeps preview jobs normalized and raw payload diagnostic-only", () => {
    const preview = previewFromEvents([
      { event: "job", runId: "r", payload: { title: "Firmware", company: "Chip Co", location: "Lisbon" } },
      { event: "completed", runId: "r", payload: { mode: "direct", timingMs: 42, warnings: ["date missing"], missingRequiredFields: ["salary"], complete: false } },
    ]);
    expect(preview.jobs).toHaveLength(1);
    expect(preview.mode).toBe("direct");
    expect(preview.missing).toEqual(["salary"]);
    expect(preview.complete).toBe(false);
  });
});

describe("user-facing failure text", () => {
  it("turns worker codes into one plain sentence and never leaks the code", () => {
    expect(describeFailure("robots_denied")).toBe("This site asks automated tools not to read its job listings.");
    expect(describeFailure("auth_required")).toBe("This site requires a login before it will show its jobs.");
    expect(describeFailure("something_new")).toBe("This website doesn't work with JobScraper.");
    expect(describeFailure()).toBe("This website doesn't work with JobScraper.");
  });
  it("summarizes a run without adapter names, modes or timings", () => {
    const done = describeOutcome([{ event: "completed", runId: "r", payload: { discovered: 33, mode: "direct", timingMs: 900 } }], "Arm");
    expect(done).toBe("Arm: 33 listings found.");
    expect(describeOutcome([{ event: "failed", runId: "r", payload: { code: "rate_limited" } }], "Arm"))
      .toBe("Arm: This site is asking us to slow down. Try again in a few minutes.");
  });
});

describe("warm update summaries", () => {
  it("reports skipped sources separately from sources that were read", () => {
    expect(describeUpdateSummary({ runId: "r", completedSources: 2, unchangedSources: 15, failedSources: 1, cancelledSources: 0 }))
      .toBe("Updated: 2 read, 15 unchanged, 1 failed.");
  });
  it("never presents a failed or uncertain check as a changed source", () => {
    const message = describeCheckSummary([
      { sourceId: "a", name: "Arm", fresh: 2, stored: 10, changed: true, conclusive: true, requests: 1 },
      { sourceId: "b", name: "Apple", fresh: 0, stored: 10, changed: false, conclusive: false, requests: 1, error: "timed out" },
      { sourceId: "c", name: "Unknown", fresh: 0, stored: 10, changed: false, conclusive: false, requests: 1 },
    ]);
    expect(message).toContain("Changed: Arm (2 new)");
    expect(message).toContain("Needs an update to confirm: Unknown");
    expect(message).toContain("Could not check: Apple (timed out)");
    expect(message).not.toContain("Changed: Arm (2 new), Apple");
  });
});

describe("scrape activity log", () => {
  it("compresses raw payloads into progress and elapsed-time summaries", () => {
    expect(describeLogEvent({ event: "started", runId: "r", payload: { adapter: "amd", command: "scrape_source" } }))
      .toBe("Started scrape · AMD");
    expect(describeLogEvent({ event: "progress", runId: "r", payload: { phase: "fetching", requests: 42, elapsedMs: 15_000 } }))
      .toBe("Fetching · 42 requests · 15.0s");
    expect(describeLogEvent({ event: "progress", runId: "r", payload: { phase: "saving", current: 274, total: 685, elapsedMs: 43_200 } }))
      .toBe("Saving · 274/685 (40%) · 43.2s");
    expect(describeLogEvent({ event: "progress", runId: "r", payload: { phase: "enriching", current: 4, total: 10, requests: 4, elapsedMs: 2_000 } }))
      .toBe("Descriptions · 4/10 · 4 requests · 2.0s");
    expect(describeLogEvent({ event: "completed", runId: "r", payload: { mode: "enrichment", persisted: 9, failed: 1, requests: 10, elapsedMs: 4_200 } }))
      .toBe("Descriptions done · 9 saved · 1 failed · 10 requests · 4.2s");
    expect(describeLogEvent({ event: "completed", runId: "r", payload: { discovered: 1086, persisted: 685, filtered: 401, requests: 109, elapsedMs: 46_138 } }))
      .toBe("Done · 1,086 found · 685 saved · 401 filtered · 109 requests · 46.1s");
    expect(describeLogEvent({ event: "warning", runId: "r", payload: { code: "robots_override", message: "long text" } }))
      .toBe("Robots override active");
  });
});

describe("what a probe result means for saving", () => {
  it("keeps a page that answered but listed nothing, so it can be saved and left off", () => {
    const verdict = probeVerdict({ recommendedAdapter: "static-css", unsupported: true, reason: "no_listings", sampleJobs: [] });
    expect(verdict).toEqual({ ok: true, adapterId: "static-css", found: 0 });
  });
  it("reports how many listings a working page produced", () => {
    expect(probeVerdict({ recommendedAdapter: "workday", sampleJobs: [{}, {}, {}] }))
      .toEqual({ ok: true, adapterId: "workday", found: 3 });
  });
  it("refuses a page that could not be read at all, and says why in one sentence", () => {
    expect(probeVerdict({ recommendedAdapter: "static-css", unsupported: true, reason: "blocked" }))
      .toEqual({ ok: false, error: "This site blocks automated tools from reading its job listings." });
    expect(probeVerdict({ reason: "not_found" }))
      .toEqual({ ok: false, error: "That page could not be found. Check the address and try again." });
  });
});

describe("run log wording", () => {
  it("does not report a preview as a run that saved nothing", () => {
    // The run log is where "Done · 20 found · 0 saved" appeared for a test, which reads as a board
    // that returned nothing rather than a preview doing exactly what it was asked to.
    const line = (command: string, persisted: number) =>
      describeLogEvent({ event: "completed", runId: "r", payload: { command, discovered: 20, persisted, requests: 1 } } as never);
    expect(line("test_source", 0)).toContain("20 found");
    expect(line("test_source", 0)).toContain("preview only");
    expect(line("test_source", 0)).not.toContain("0 saved");
    // A real scrape still reports what it stored.
    expect(line("scrape_source", 20)).toContain("20 saved");
  });
});
