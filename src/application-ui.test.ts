import { describe, expect, it } from "vitest";
import { attachmentRequest, boardMoveError, isApplied, stageLabel, timelineText } from "./application-ui";

describe("application detail workflows",()=>{
 it("keeps detail note/document requests tied to one application",()=>{
  expect(attachmentRequest("app",{name:"cover.docx",type:"application/vnd.openxmlformats-officedocument.wordprocessingml.document",base64:"AA=="},"cover_letter")).toMatchObject({applicationId:"app",documentType:"cover_letter",filename:"cover.docx"});
 });
 it("shows immutable event timeline reason and stages",()=>expect(timelineText({eventType:"stage_changed",fromStage:"offer",toStage:"rejected",reason:"Role closed"})).toContain("Role closed"));
 it("models note CRUD by retaining immutable application ownership",()=>{
  const note={id:"note",applicationId:"app",body:"Recruiter replied",updatedAt:"2026-01-01T00:00:00Z"};
  expect(note.applicationId).toBe("app");expect(note.body).toContain("replied");
 });
});
describe("application board accessibility",()=>{
 it("returns same explicit errors for dropdown and drag targets",()=>{
  expect(boardMoveError("planned","applied")).toContain("explicit confirmation");
  expect(boardMoveError("planned","unknown")).toBe("Invalid application stage.");
  expect(boardMoveError("offer","rejected")).toBeUndefined();
 });
});

// The board, the Jobs badge and the timeline all name stages from one place, so a job the user
// pressed Save on never reads "Planned" in one view and "Saved" in another.
describe("stage vocabulary",()=>{
 it("calls the saved-but-not-applied stage Saved everywhere",()=>{
  expect(stageLabel("planned")).toBe("Saved");
  expect(isApplied("planned")).toBe(false);
 });
 it("treats every stage past planned as applied and titles it",()=>{
  expect(stageLabel("interviewing")).toBe("Interviewing");
  expect(isApplied("applied")).toBe(true);
  expect(isApplied("rejected")).toBe(true);
 });
 it("says nothing about a job that was never saved",()=>{
  expect(stageLabel(undefined)).toBe("");
  expect(isApplied(undefined)).toBe(false);
 });
});
