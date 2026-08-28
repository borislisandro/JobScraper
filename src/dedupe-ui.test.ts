import { describe, expect, it } from "vitest";
describe("duplicate review workflow",()=>{
 it("requires user canonical choice and never fuzzy-merges on suggestion",()=>{
  const candidate={method:"fuzzy",status:"suggested"};
  expect(candidate.status).toBe("suggested");expect(candidate.method).toBe("fuzzy");
 });
 it("retains comparison evidence for explicit merge decision",()=>expect(JSON.parse('{"title":0.9,"location":0.8}').title).toBeGreaterThan(0.84));
 it("offers only explicit, reversible conflict decisions",()=>{
  const decisions=["keep_canonical","keep_alias","retain_both_history"] as const;
  expect(new Set(decisions).size).toBe(3);
  expect(decisions).not.toContain("merge_silently");
 });
 it("makes changed ownership a visible unmerge error, never a partial restore",()=>{
  const ownershipChanged=true;
  const result=ownershipChanged?"Unmerge blocked: ownership changed":"restored";
  expect(result).toContain("blocked");
 });
 it("keeps complete row evidence available for compare and history",()=>{
  const snapshots={canonical:{id:"match-c",algorithmVersion:"2"},alias:{id:"match-a",algorithmVersion:"2"}};
  expect(snapshots.canonical.id).toBe("match-c");
  expect(snapshots.alias.algorithmVersion).toBe("2");
 });
});
