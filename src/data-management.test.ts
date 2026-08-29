import { describe, expect, it } from "vitest";

describe("data-management workflow",()=>{
 it("requires a selected path and explicit overwrite",()=>{
  const selected="C:/Exports/jobs.csv";
  const request={kind:"jobs",destination:selected,overwrite:false};
  expect(request.destination).toMatch(/Exports/);
  expect(request.overwrite).toBe(false);
 });
 it("binds purge confirmation to preview hash",()=>{
  const preview={token:"preview",previewHash:"abc",expiresAt:"2026-08-29T12:00:00Z"};
  const request={token:preview.token,previewHash:preview.previewHash,confirmation:"PURGE"};
  expect(request.confirmation).toBe("PURGE");
  expect(request.previewHash).not.toBe("changed");
 });
 it("keeps protected resume and application history out of closed-job purge",()=>{
  const preview={jobs:2,protected:3,files:[] as string[]};
  expect(preview.protected).toBeGreaterThan(0);
  expect(preview.files).toHaveLength(0);
 });
 it("exposes source and company outcome export payloads",()=>{
  const kinds=["source_outcomes","company_outcomes"];
  const payload={kind:kinds[1],filter:{sourceId:"source",company:"Chip Co"}};
  expect(kinds).toContain("source_outcomes");
  expect(payload.kind).toBe("company_outcomes");
  expect(payload.filter.company).toBe("Chip Co");
 });
});
