/** @vitest-environment jsdom */
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {applyProfile,listProfiles,scrapeAll,type ProfileApplied,type ProfilesState}from "./api";
import {ProfileSwitcher}from "./main";
import {describeProfileApplied}from "./source-ui";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("./api",async original=>({...await original<typeof import("./api")>(),listProfiles:vi.fn(),applyProfile:vi.fn(),scrapeAll:vi.fn()}));
let container:HTMLDivElement,root:Root,client:QueryClient,state:ProfilesState;
const profiles:ProfilesState["profiles"]=[{id:"semiconductor",name:"Semiconductor",description:"Chip employers.",companies:48,on:0},{id:"data-science",name:"Data science",description:"Data and AI employers.",companies:51,on:0}];
const applied=(extra:Partial<ProfileApplied>={}):ProfileApplied=>({profile:"semiconductor",added:48,switchedOn:0,alreadyFollowing:0,readNow:["a","b"],...extra});
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
const select=()=>container.querySelector("select")as HTMLSelectElement;
async function choose(value:string){await act(async()=>{const set=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,"value")!.set!;set.call(select(),value);select().dispatchEvent(new Event("change",{bubbles:true}))});await settle()}
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 state={active:null,profiles};vi.mocked(listProfiles).mockImplementation(async()=>state);
 vi.mocked(scrapeAll).mockResolvedValue({runId:"r",completedSources:2,unchangedSources:0,failedSources:0,cancelledSources:0});
});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(onStatus:(text:string)=>void){await act(async()=>root.render(<QueryClientProvider client={client}><ProfileSwitcher onStatus={onStatus}/></QueryClientProvider>));await settle()}
it("offers every profile plus 'All profiles', which is what an install with no profile chosen shows",async()=>{
 await render(()=>{});
 expect([...select().options].map(option=>option.textContent)).toEqual(["Semiconductor","Data science","All profiles"]);
 expect(select().value).toBe("");
});
it("choosing a profile applies it, reads exactly the companies never read, and refreshes every tab",async()=>{
 const status=vi.fn(),invalidate=vi.spyOn(client,"invalidateQueries");
 vi.mocked(applyProfile).mockImplementation(async()=>{state={active:"semiconductor",profiles:profiles.map(profile=>profile.id==="semiconductor"?{...profile,on:48}:profile)};return applied()});
 await render(status);await choose("semiconductor");
 expect(applyProfile).toHaveBeenCalledWith("semiconductor");
 expect(scrapeAll).toHaveBeenCalledWith(expect.any(String),["a","b"],false);
 expect(status).toHaveBeenCalledWith(expect.stringContaining("Reading 2 companies now"));
 expect(status).toHaveBeenLastCalledWith("Semiconductor: Updated: 2 read, 0 unchanged.");
 const refreshed=invalidate.mock.calls.map(([filters])=>filters?.queryKey?.[0]);
 for(const key of ["sources","apps","jobs","job-countries","profiles","new-since"])expect(refreshed).toContain(key);
 expect(select().value).toBe("semiconductor");
});
it("reads nothing when every company was already read",async()=>{
 vi.mocked(applyProfile).mockResolvedValue(applied({added:0,alreadyFollowing:48,readNow:[]}));
 await render(()=>{});await choose("semiconductor");
 expect(scrapeAll).not.toHaveBeenCalled();
});
it("switches between profiles without asking, because switching hides and never switches anything off",async()=>{
 state={active:"semiconductor",profiles};const ask=vi.spyOn(window,"confirm");
 vi.mocked(applyProfile).mockResolvedValue(applied({profile:"data-science",added:0,alreadyFollowing:51,readNow:[]}));
 await render(()=>{});await choose("data-science");
 expect(ask).not.toHaveBeenCalled();expect(applyProfile).toHaveBeenCalledWith("data-science");
 vi.mocked(applyProfile).mockResolvedValue({profile:null,added:0,switchedOn:0,alreadyFollowing:0,readNow:[]});
 await choose("");expect(applyProfile).toHaveBeenLastCalledWith(null);
});
it("shows an apply failure instead of swallowing it",async()=>{
 const status=vi.fn();vi.mocked(applyProfile).mockRejectedValue("Unknown profile: astronaut");
 await render(status);await choose("semiconductor");expect(status).toHaveBeenCalledWith("Unknown profile: astronaut");
});
it("describes what a profile switch did",()=>{
 expect(describeProfileApplied("Semiconductor",applied())).toBe("Semiconductor: showing only its sources, jobs and applications. 48 companies added. Reading 2 companies now.");
 expect(describeProfileApplied("Data science",applied({profile:"data-science",added:0,switchedOn:1,alreadyFollowing:50,readNow:[]})))
  .toBe("Data science: showing only its sources, jobs and applications. 1 company added.");
 expect(describeProfileApplied(undefined,applied({added:0,readNow:[]}))).toBe("semiconductor: showing only its sources, jobs and applications.");
 expect(describeProfileApplied(undefined,{profile:null,added:0,switchedOn:0,alreadyFollowing:0,readNow:[]})).toBe("All profiles: showing every source, job and application. Updates read every profile's sources.");
});
