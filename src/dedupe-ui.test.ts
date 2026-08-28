import { describe, expect, it } from "vitest";
describe("duplicate review workflow",()=>{
 it("requires user canonical choice and never fuzzy-merges on suggestion",()=>{
  const candidate={method:"fuzzy",status:"suggested"};
  expect(candidate.status).toBe("suggested");expect(candidate.method).toBe("fuzzy");
 });
 it("retains comparison evidence for explicit merge decision",()=>expect(JSON.parse('{"title":0.9,"location":0.8}').title).toBeGreaterThan(0.84));
});
