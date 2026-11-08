import { describe, expect, it } from "vitest";
import { boardColumn, columnLabel, isApplied, isEnded, stageLabel, websiteUrl } from "./application-ui";

// Applications are Saved, Applied or Rejected, and every view names them the same way.
describe("stage vocabulary",()=>{
 it("calls the saved-but-not-applied stage Saved everywhere",()=>{
  expect(stageLabel("planned")).toBe("Saved");
  expect(isApplied("planned")).toBe(false);
  expect(boardColumn("planned")).toBe("planned");
 });
 it("folds the in-between stages left from the old board into Applied",()=>{
  for(const stage of ["applied","screening","interviewing","offer","accepted"]){
   expect(stageLabel(stage)).toBe("Applied");
   expect(isApplied(stage)).toBe(true);
   expect(isEnded(stage)).toBe(false);
   expect(boardColumn(stage)).toBe("applied");
  }
 });
 it("puts rejected and withdrawn applications in the Rejected column, each under its own name",()=>{
  expect(stageLabel("rejected")).toBe("Rejected");
  expect(stageLabel("withdrawn")).toBe("Withdrawn");
  for(const stage of ["rejected","withdrawn"]){
   expect(isApplied(stage)).toBe(true);
   expect(isEnded(stage)).toBe(true);
   expect(boardColumn(stage)).toBe("rejected");
  }
  expect(columnLabel("rejected")).toBe("Rejected");
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
