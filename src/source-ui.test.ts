import { describe, expect, it } from "vitest";
import { personaFormValues, previewFromEvents, supportsSessionCapture } from "./source-ui";

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

describe("persona edit values", () => {
  it("loads every editable persona field and preserves its ID", () => {
    const value = personaFormValues({ persona: { id: "p", name: "Embedded", targetTitlesJson: '["Firmware engineer"]', includeKeywordsJson: '["rust"]', includeKeywordMode: "all", excludeKeywordsJson: '["intern"]', location: "Lisbon", workMode: "hybrid", seniority: "senior", salaryMin: 90000, threshold: 72, unknownPolicy: "require_known", resumeDocumentId: "r" }, confirmedSkills: ["Rust", "Linux"] });
    expect(value).toMatchObject({ id: "p", titles: "Firmware engineer", skills: "Rust, Linux", includeMode: "all", salary: "90000", threshold: 72 });
  });
  it("cancel/create values reset to a blank persona", () => expect(personaFormValues()).toMatchObject({ name: "", includeMode: "any", threshold: 60 }));
});
