/** @vitest-environment jsdom */
// Save, Apply and "Not for me" all route through useJobActions' refresh(), which used to call
// removeQueries on the jobs cache. That deletes the cached pages synchronously, so the Jobs list
// (an infinite-scroll query) collapsed to its loading skeleton and rebuilt from page 0 — the user
// lost their place and landed back at the top. invalidateQueries marks the same cache stale and
// refetches quietly instead, so it must still be readable immediately after the action settles.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,setJobDismissed,type Job}from "./api";
import {JobCard}from "./main";
vi.mock("./api",async original=>({...await original<typeof import("./api")>(),setJobDismissed:vi.fn()}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const job:Job={id:"job-1",sourceId:"source-1",title:"Digital Verification Engineer",company:"Chip Co",applyUrl:"https://example.test/apply",descriptionText:"",descriptionStatus:"complete",availability:"active"};
const jobsKey=["jobs","some-filter"];
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 // Stands in for what the Jobs page's useInfiniteQuery has already cached — an unrelated key
 // prefixed "jobs", exactly what refresh()'s partial-match invalidate/remove call targets.
 client.setQueryData(jobsKey,{pages:[{items:[job],total:1,offset:0,hasMore:false}],pageParams:[0]})});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(){await act(async()=>root.render(<QueryClientProvider client={client}><JobCard job={job} onStatus={()=>{}}/></QueryClientProvider>));await settle()}
const click=async(label:string)=>{await act(async()=>{(Array.from(container.querySelectorAll("button")).find(b=>b.textContent===label)as HTMLButtonElement).click()});await settle()};

it("Save keeps the rest of the cached job list on screen instead of clearing it",async()=>{
 vi.spyOn(api,"createApp").mockResolvedValue({id:"app-1",jobId:job.id,currentStage:"planned",pendingConfirmation:false});
 await render();await click("Save");
 expect(client.getQueryData(jobsKey)).toBeDefined();
});
it("Apply keeps the rest of the cached job list on screen instead of clearing it",async()=>{
 vi.spyOn(api,"createApp").mockResolvedValue({id:"app-1",jobId:job.id,currentStage:"planned",pendingConfirmation:false});
 vi.spyOn(api,"openApply").mockResolvedValue(undefined);
 await render();await click("Apply");
 expect(client.getQueryData(jobsKey)).toBeDefined();
});
it("Not for me keeps the rest of the cached job list on screen instead of clearing it",async()=>{
 vi.mocked(setJobDismissed).mockResolvedValue(undefined);
 await render();await click("Not for me");
 expect(client.getQueryData(jobsKey)).toBeDefined();
});
