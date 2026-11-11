/** @vitest-environment jsdom */
// Two things the Jobs page must get right while the list changes underneath it: a list that could not
// be read is an error, not "no jobs"; and the reader stays on the job it was opened on when an update
// stores new rows ahead of it.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,countJobs,getSetting,jobCountries,jobDescription,listProfiles,setSetting,startAutomaticSync,type Job,type Source}from "./api";
import {Jobs,readerPosition}from "./main";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock("./api",async original=>{const actual=await original<typeof import("./api")>();
 return {...actual,listProfiles:vi.fn(),getSetting:vi.fn(),setSetting:vi.fn(),jobCountries:vi.fn(),countJobs:vi.fn(),startAutomaticSync:vi.fn(),jobDescription:vi.fn(),
  api:{...actual.api,listSources:vi.fn(),jobs:vi.fn()}}});
let container:HTMLDivElement,root:Root,client:QueryClient;
// The page asks for the list more than once while it settles (sources load, the filter debounces), so the
// backend is a switch the test flips rather than a queue of answers.
let listed:string[],failing:string|undefined;
const source:Source={id:"chip",name:"Chip Co",baseUrl:"https://chip.test",adapterId:"json",adapterVersion:"1",enabled:true,kind:"active",robotsOverride:false,jobCount:3,closedCount:0};
const job=(id:string):Job=>({id,sourceId:"chip",title:`Engineer ${id}`,company:"Chip Co",descriptionText:"",descriptionStatus:"complete",availability:"active"});
const page=(ids:string[])=>({items:ids.map(job),total:ids.length,offset:0,hasMore:false});
const settle=(ms=20)=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,ms))});
const button=(label:string)=>Array.from(container.querySelectorAll("button")).find(found=>found.textContent===label)as HTMLButtonElement|undefined;
const readerTitle=()=>container.querySelector(".reader h2")?.textContent;
const mount=async()=>{await act(async()=>root.render(<QueryClientProvider client={client}><Jobs onCount={()=>{}}/></QueryClientProvider>));await settle(40)};
// What a finished source does during an update: everything under the "jobs" key is refetched.
const refetch=async()=>{await act(async()=>{await client.invalidateQueries({queryKey:["jobs"]})});await settle()};
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false,refetchOnWindowFocus:false}}});
 vi.mocked(listProfiles).mockResolvedValue({active:"semiconductor",profiles:[]});
 vi.mocked(getSetting).mockResolvedValue(null);vi.mocked(setSetting).mockResolvedValue(undefined);
 vi.mocked(api.listSources).mockResolvedValue([source]);listed=[];failing=undefined;
 vi.mocked(api.jobs).mockImplementation(async()=>{if(failing)throw failing;return page(listed)});
 vi.mocked(jobCountries).mockResolvedValue([[],0]);vi.mocked(countJobs).mockResolvedValue(0);vi.mocked(startAutomaticSync).mockResolvedValue(false);
 vi.mocked(jobDescription).mockResolvedValue({text:"",status:"complete"})});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.clearAllMocks()});

it("says the list could not be loaded instead of calling it empty, and loads it on Try again",async()=>{
 failing="database is locked";
 await mount();
 expect(container.textContent).toContain("The job list could not be loaded");
 expect(container.textContent).toContain("database is locked");
 expect(container.textContent).not.toContain("No jobs stored for these sources yet");
 failing=undefined;listed=["a"];
 await act(async()=>{button("Try again")!.click()});await settle();
 expect(container.textContent).toContain("Engineer a");
 expect(container.textContent).not.toContain("could not be loaded");
});
it("keeps the rows on screen and says so when a refresh fails",async()=>{
 listed=["a"];
 await mount();failing="database is locked";await refetch();
 expect(container.textContent).toContain("Engineer a");
 expect(container.textContent).toContain("The list could not be refreshed");
});
it("keeps the reader on the job it was opened on when an update stores new rows ahead of it",async()=>{
 listed=["a","b","c"];
 await mount();
 const row=Array.from(container.querySelectorAll(".jobs > *")).find(element=>element.textContent?.includes("Engineer b"))!;
 await act(async()=>{row.dispatchEvent(new MouseEvent("click",{bubbles:true}))});await settle();
 expect(readerTitle()).toBe("Engineer b");
 // Two new jobs arrive at the top of the list while the reader is open.
 listed=["new-1","new-2","a","b","c"];
 await refetch();
 expect(readerTitle()).toBe("Engineer b");
 expect(container.querySelector(".reader .count")?.textContent).toBe("4 of 5");
 // The open job leaves the list (Not for me): the reader moves to the one that took its place.
 listed=["new-1","new-2","a","c"];
 await refetch();
 expect(readerTitle()).toBe("Engineer c");
});
it("resolves the reader's row from the job, falling back to the position only when the job is gone",()=>{
 const list=["a","b","c"].map(id=>({id}));
 expect(readerPosition(list,null)).toBeNull();
 expect(readerPosition([],{id:"b",index:1})).toBeNull();
 expect(readerPosition(list,{id:"c",index:0})).toBe(2);
 expect(readerPosition(list,{id:"gone",index:1})).toBe(1);
 expect(readerPosition(list,{id:"gone",index:9})).toBe(2);
});
