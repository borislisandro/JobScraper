/** @vitest-environment jsdom */
// A save that fails must leave what was typed where it was: the panel used to clear the note box, close
// the note editor and empty the interview notes whether or not the save had worked.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,type Application,type ApplicationDetails}from "./api";
import {ApplicationPanel}from "./main";
vi.mock("@tauri-apps/plugin-opener",()=>({openUrl:vi.fn(),revealItemInDir:vi.fn()}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const application:Application={id:"app-1",jobId:"job-1",currentStage:"applied",appliedAt:"2026-11-01T10:00:00Z",title:"Verification Engineer",company:"Chip Co",pendingConfirmation:false};
const details:ApplicationDetails={application,events:[],documents:[],notes:[{id:"note-1",body:"Sent the CV",createdAt:"2026-11-02T10:00:00Z"}]};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
const button=(label:string)=>Array.from(container.querySelectorAll("button")).find(found=>found.textContent===label)as HTMLButtonElement;
const click=async(label:string)=>{await act(async()=>{button(label).click()});await settle()};
const type=async(field:HTMLTextAreaElement|HTMLInputElement,value:string)=>{await act(async()=>{
 Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field),"value")!.set!.call(field,value);field.dispatchEvent(new Event("input",{bubbles:true}))})};
const draft=()=>container.querySelector('textarea[placeholder^="What you sent"]')as HTMLTextAreaElement;
const interviewNotes=()=>container.querySelector('input[placeholder^="Notes (who"]')as HTMLInputElement;
beforeEach(async()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 vi.spyOn(api,"applicationDetails").mockResolvedValue(details);vi.spyOn(api,"interviews").mockResolvedValue([]);
 await act(async()=>root.render(<QueryClientProvider client={client}><ApplicationPanel applicationId="app-1" onClose={()=>{}} onChanged={()=>{}}/></QueryClientProvider>));await settle()});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks()});

it("keeps a note that could not be saved, and says why",async()=>{
 vi.spyOn(api,"saveNote").mockRejectedValue("database is locked");
 await type(draft(),"Call back on Monday");await click("Add note");
 expect(draft().value).toBe("Call back on Monday");
 expect(container.textContent).toContain("database is locked");
});
it("clears the note box once the note is saved",async()=>{
 vi.spyOn(api,"saveNote").mockResolvedValue({id:"note-2",body:"Call back on Monday",createdAt:"2026-11-03T10:00:00Z"});
 await type(draft(),"Call back on Monday");await click("Add note");
 expect(api.saveNote).toHaveBeenCalledWith({applicationId:"app-1",body:"Call back on Monday"});
 expect(draft().value).toBe("");
 expect(container.textContent).toContain("Note added.");
});
it("leaves a note open for editing when saving the edit fails",async()=>{
 vi.spyOn(api,"saveNote").mockRejectedValue("database is locked");
 await click("Edit");
 const editor=Array.from(container.querySelectorAll("textarea")).find(area=>area.value==="Sent the CV")!;
 await type(editor,"Sent the CV and the cover letter");await click("Save");
 expect(Array.from(container.querySelectorAll("textarea")).map(area=>area.value)).toContain("Sent the CV and the cover letter");
});
it("keeps the interview notes when the interview could not be added",async()=>{
 vi.spyOn(api,"saveInterview").mockRejectedValue("Application was not found");
 await type(interviewNotes(),"Video call with the hiring manager");await click("Add interview");
 expect(interviewNotes().value).toBe("Video call with the hiring manager");
 expect(container.textContent).toContain("Application was not found");
});
