import { describe, expect, it } from "vitest";
import { boardColumn, columnLabel, describeEvent, followUpDue, isApplied, isEnded, listingGone, orderApplications, stageLabel, websiteUrl } from "./application-ui";

// Applications are Saved, Applied or Rejected, and every view names them the same way.
describe("stage vocabulary",()=>{
 it("calls the saved-but-not-applied stage Saved everywhere",()=>{
  expect(stageLabel("planned")).toBe("Saved");
  expect(isApplied("planned")).toBe(false);
  expect(boardColumn("planned")).toBe("planned");
 });
 it("keeps the in-between stages in the Applied column, each under its own name",()=>{
  expect(stageLabel("interviewing")).toBe("Interviewing");expect(stageLabel("offer")).toBe("Offer");
  for(const stage of ["applied","screening","interviewing","offer","accepted"]){
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
describe("follow-up and the saved pile",()=>{
 it("is due only for an application still in Applied whose reminder has come due",()=>{
  const now=Date.parse("2026-11-10T12:00:00Z");
  expect(followUpDue({currentStage:"applied",followUpAt:"2026-11-09T00:00:00Z"},now)).toBe(true);
  expect(followUpDue({currentStage:"applied",followUpAt:"2026-11-20T00:00:00Z"},now)).toBe(false);
  expect(followUpDue({currentStage:"interviewing",followUpAt:"2026-11-09T00:00:00Z"},now)).toBe(false);
  expect(followUpDue({currentStage:"applied"},now)).toBe(false);
 });
 it("orders saved jobs by saved date, posting date or closing date, undated last",()=>{
  const apps=[{id:"a",createdAt:"2026-11-01T00:00:00Z",jobPostedAt:"2026-10-01",jobClosingAt:null},{id:"b",createdAt:"2026-11-05T00:00:00Z",jobPostedAt:null,jobClosingAt:"2026-11-30"},{id:"c",createdAt:"2026-11-03T00:00:00Z",jobPostedAt:"2026-10-20",jobClosingAt:"2026-11-15"}];
  expect(orderApplications(apps,"saved").map(a=>a.id)).toEqual(["b","c","a"]);
  expect(orderApplications(apps,"posted").map(a=>a.id)).toEqual(["c","a","b"]);
  expect(orderApplications(apps,"closing").map(a=>a.id)).toEqual(["c","b","a"]);
 });
 it("badges a listing its board stopped showing",()=>{
  expect(listingGone("closed")).toBe("Closed");expect(listingGone("possibly_closed")).toBe("May be gone");expect(listingGone("active")).toBeNull();
 });
 it("describes timeline events in the board's words",()=>{
  expect(describeEvent({eventType:"stage_changed",toStage:"interviewing",reason:"Set in the application panel",payloadJson:"{}"})).toBe("Interviewing — Set in the application panel");
  expect(describeEvent({eventType:"ghost_threshold_changed",payloadJson:'{"days":10}'})).toBe("Follow-up set to 10 days after applying");
  expect(describeEvent({eventType:"document_attached",payloadJson:'{"filename":"cv.pdf"}'})).toBe("Attached cv.pdf");
 });
});
