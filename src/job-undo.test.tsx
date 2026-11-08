/** @vitest-environment jsdom */
// Every quick action on a job is reversible from the message it leaves: Not for me, Save and Apply each
// hand the status line an Undo that puts back exactly what was there before.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,jobDescription,setJobDismissed,type Job}from "./api";
import {JobCard,JobReader,postedAgo,updatedLabel}from "./main";
vi.mock("./api",async original=>({...await original<typeof import("./api")>(),setJobDismissed:vi.fn(),jobDescription:vi.fn()}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const job:Job={id:"job-1",sourceId:"source-1",title:"Data Engineer",company:"Data Co",applyUrl:"https://example.test/apply",descriptionText:"",descriptionStatus:"complete",availability:"active"};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}})});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
const click=async(label:string)=>{await act(async()=>{(Array.from(container.querySelectorAll("button")).find(b=>b.textContent===label)as HTMLButtonElement).click()});await settle()};
async function renderCard(value:Job,onStatus:(text:string,undo?:()=>Promise<unknown>)=>void){
 await act(async()=>root.render(<QueryClientProvider client={client}><JobCard job={value} onStatus={onStatus}/></QueryClientProvider>));await settle()}

it("Not for me offers an Undo that brings the job back",async()=>{
 vi.mocked(setJobDismissed).mockResolvedValue(undefined);const onStatus=vi.fn();
 await renderCard(job,onStatus);await click("Not for me");
 expect(setJobDismissed).toHaveBeenCalledWith("job-1",true);
 const undo=onStatus.mock.calls.at(-1)?.[1] as ()=>Promise<unknown>;
 await act(async()=>{await undo()});
 expect(setJobDismissed).toHaveBeenLastCalledWith("job-1",false);
});
it("Undo after Save removes the application it created",async()=>{
 vi.spyOn(api,"createApp").mockResolvedValue({id:"app-1",jobId:job.id,currentStage:"planned",pendingConfirmation:false});
 const del=vi.spyOn(api,"deleteApplication").mockResolvedValue(undefined);const onStatus=vi.fn();
 await renderCard(job,onStatus);await click("Save");
 await act(async()=>{await (onStatus.mock.calls.at(-1)?.[1] as ()=>Promise<unknown>)()});
 expect(del).toHaveBeenCalledWith("app-1");
});
it("Undo after Apply on a saved job puts it back to Saved instead of deleting it",async()=>{
 vi.spyOn(api,"createApp").mockResolvedValue({id:"app-1",jobId:job.id,currentStage:"planned",pendingConfirmation:false});
 vi.spyOn(api,"openApply").mockResolvedValue(undefined);
 const stage=vi.spyOn(api,"stage").mockResolvedValue({id:"app-1",jobId:job.id,currentStage:"planned",pendingConfirmation:false});
 const del=vi.spyOn(api,"deleteApplication");const onStatus=vi.fn();
 await renderCard({...job,applicationStage:"planned"},onStatus);await click("Apply");
 await act(async()=>{await (onStatus.mock.calls.at(-1)?.[1] as ()=>Promise<unknown>)()});
 expect(stage).toHaveBeenCalledWith("app-1","planned",expect.any(String),true);expect(del).not.toHaveBeenCalled();
});
it("the reader moves with the arrow keys and can mark a job Not for me",async()=>{
 vi.mocked(jobDescription).mockResolvedValue({text:"About the role",status:"complete"});vi.mocked(setJobDismissed).mockResolvedValue(undefined);
 const jobs=[job,{...job,id:"job-2",title:"ML Engineer"}],onMove=vi.fn();
 await act(async()=>root.render(<QueryClientProvider client={client}><JobReader jobs={jobs} index={0} sourceName={()=>"Data Co"} onMove={onMove} onClose={()=>{}} onStatus={()=>{}}/></QueryClientProvider>));await settle();
 await act(async()=>{window.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowRight"}))});
 expect(onMove).toHaveBeenCalledWith(1);
 await click("Not for me");
 expect(setJobDismissed).toHaveBeenCalledWith("job-1",true);
});
it("says how old a posting is in words, keeping undated ones honest",()=>{
 const now=new Date(2026,10,8,15).getTime();
 expect(postedAgo("2026-11-08",now)).toBe("today");
 expect(postedAgo("2026-11-07",now)).toBe("yesterday");
 expect(postedAgo("2026-11-02",now)).toBe("6 d ago");
 expect(postedAgo("2026-10-11",now)).toBe("4 w ago");
 expect(postedAgo("2026-06-01",now)).toBe("5 mo ago");
 expect(postedAgo(undefined,now)).toBe("not listed");
});
it("names the day of the last update when it was not today",()=>{
 const now=new Date(2026,10,8,15).getTime();
 expect(updatedLabel(new Date(2026,10,8,11,23).getTime(),now)).toMatch(/^Updated \d/);
 expect(updatedLabel(new Date(2026,10,7,11,23).getTime(),now)).toMatch(/^Updated yesterday /);
 expect(updatedLabel(new Date(2026,10,1,11,23).getTime(),now)).not.toMatch(/yesterday|^Updated \d\d?:\d\d/);
});
