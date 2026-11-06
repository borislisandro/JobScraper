import { describe, expect, it } from "vitest";
import { boardColumn, isApplied, stageLabel, websiteUrl } from "./application-ui";

// Applications are Saved or Applied — nothing in between — and every view names them the same way.
describe("stage vocabulary",()=>{
 it("calls the saved-but-not-applied stage Saved everywhere",()=>{
  expect(stageLabel("planned")).toBe("Saved");
  expect(isApplied("planned")).toBe(false);
  expect(boardColumn("planned")).toBe("planned");
 });
 it("folds every stage past planned into Applied, including ones left from the old board",()=>{
  for(const stage of ["applied","screening","interviewing","offer","accepted","rejected","withdrawn"]){
   expect(stageLabel(stage)).toBe("Applied");
   expect(isApplied(stage)).toBe(true);
   expect(boardColumn(stage)).toBe("applied");
  }
 });
 it("says nothing about a job that was never saved",()=>{
  expect(stageLabel(undefined)).toBe("");
  expect(isApplied(undefined)).toBe(false);
 });
});
describe("Open website",()=>{
 it("prefers the listing page and falls back to the apply link",()=>{
  expect(websiteUrl({canonicalUrl:"https://a.test/job",applyUrl:"https://a.test/apply"})).toBe("https://a.test/job");
  expect(websiteUrl({applyUrl:"https://a.test/apply"})).toBe("https://a.test/apply");
  expect(websiteUrl({})).toBeUndefined();
 });
});
