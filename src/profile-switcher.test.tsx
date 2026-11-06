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
const applied=(extra:Partial<ProfileApplied>={}):ProfileApplied=>({profile:"semiconductor",added:48,switchedOn:0,switchedOff:0,alreadyFollowing:0,readNow:["a","b"],...extra});
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
const select=()=>container.querySelector("select")as HTMLSelectElement;
async function choose(value:string){await act(async()=>{const set=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,"value")!.set!;set.call(select(),value);select().dispatchEvent(new Event("change",{bubbles:true}))});await settle()}
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 state={active:null,profiles};vi.mocked(listProfiles).mockImplementation(async()=>state);
 vi.mocked(scrapeAll).mockResolvedValue({runId:"r",completedSources:2,unchangedSources:0,failedSources:0,cancelledSources:0});
});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(onStatus:(text:string)=>void){await act(async()=>root.render(<QueryClientProvider client={client}><ProfileSwitcher onStatus={onStatus}/></QueryClientProvider>));await settle()}
it("offers every profile plus 'no profile', and explains the choice before one is made",async()=>{
 await render(()=>{});
 expect([...select().options].map(option=>option.textContent)).toEqual(["No profile — I choose my own sources","Semiconductor · 48 companies","Data science · 51 companies"]);
 expect(select().value).toBe("");expect(container.textContent).toContain("Pick a profile for a ready-made set of employers");
});
it("choosing a profile applies it and reads exactly the companies that were never read",async()=>{
 const status=vi.fn();vi.mocked(applyProfile).mockImplementation(async()=>{state={active:"semiconductor",profiles:profiles.map(profile=>profile.id==="semiconductor"?{...profile,on:48}:profile)};return applied()});
 await render(status);await choose("semiconductor");
 expect(applyProfile).toHaveBeenCalledWith("semiconductor");
 expect(scrapeAll).toHaveBeenCalledWith(expect.any(String),["a","b"],false);
 expect(status).toHaveBeenCalledWith(expect.stringContaining("Reading 2 companies now"));
 expect(status).toHaveBeenLastCalledWith("Semiconductor: Updated: 2 read, 0 unchanged.");
 expect(select().value).toBe("semiconductor");expect(container.textContent).toContain("48 of 48 are switched on");
});
it("reads nothing when every company was already read",async()=>{
 vi.mocked(applyProfile).mockResolvedValue(applied({added:0,alreadyFollowing:48,readNow:[]}));
 await render(()=>{});await choose("semiconductor");
 expect(scrapeAll).not.toHaveBeenCalled();
});
it("asks before switching away from an active profile, and does nothing if the answer is no",async()=>{
 state={active:"semiconductor",profiles};const ask=vi.spyOn(window,"confirm").mockReturnValue(false);
 await render(()=>{});await choose("data-science");
 expect(ask).toHaveBeenCalledWith(expect.stringContaining("Switch to Data science?"));expect(applyProfile).not.toHaveBeenCalled();
 ask.mockReturnValue(true);vi.mocked(applyProfile).mockResolvedValue(applied({profile:"data-science",added:51,switchedOff:48,readNow:[]}));
 await choose("data-science");expect(applyProfile).toHaveBeenCalledWith("data-science");
});
it("does not ask when there is nothing to switch away from, and 'no profile' clears it without a question",async()=>{
 const ask=vi.spyOn(window,"confirm");vi.mocked(applyProfile).mockResolvedValue(applied({readNow:[]}));
 await render(()=>{});await choose("semiconductor");expect(ask).not.toHaveBeenCalled();
 state={active:"semiconductor",profiles};await act(async()=>{await client.invalidateQueries({queryKey:["profiles"]})});await settle();
 vi.mocked(applyProfile).mockResolvedValue({profile:null,added:0,switchedOn:0,switchedOff:0,alreadyFollowing:0,readNow:[]});
 await choose("");expect(applyProfile).toHaveBeenLastCalledWith(null);expect(ask).not.toHaveBeenCalled();
});
it("shows an apply failure instead of swallowing it",async()=>{
 const status=vi.fn();vi.mocked(applyProfile).mockRejectedValue("Unknown profile: astronaut");
 await render(status);await choose("semiconductor");expect(status).toHaveBeenCalledWith("Unknown profile: astronaut");
});
it("describes what a profile change did",()=>{
 expect(describeProfileApplied("Semiconductor",applied())).toBe("Semiconductor: 48 companies switched on. Reading 2 companies now.");
 expect(describeProfileApplied("Data science",applied({profile:"data-science",added:1,switchedOn:2,switchedOff:48,alreadyFollowing:3,readNow:["x"]})))
  .toBe("Data science: 3 companies switched on, 48 from the previous profile switched off, their jobs kept, 3 already followed. Reading 1 company now.");
 expect(describeProfileApplied(undefined,applied({added:0,switchedOn:0,readNow:[]}))).toBe("semiconductor: nothing new to switch on.");
 expect(describeProfileApplied(undefined,{profile:null,added:0,switchedOn:0,switchedOff:0,alreadyFollowing:0,readNow:[]})).toBe("No profile selected. Your sources stay exactly as they are.");
});
