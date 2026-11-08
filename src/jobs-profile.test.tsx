/** @vitest-environment jsdom */
// Each profile is its own job search, so the Jobs page keeps one set of filters per profile: switching
// profile must open the other search exactly as it was left, and typing in one must never touch the other.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,countJobs,getSetting,jobCountries,listProfiles,setSetting,startAutomaticSync,type ProfilesState,type Source}from "./api";
import {Jobs}from "./main";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn(async()=>()=>{})}));
vi.mock("./api",async original=>{const actual=await original<typeof import("./api")>();
 return {...actual,listProfiles:vi.fn(),getSetting:vi.fn(),setSetting:vi.fn(),jobCountries:vi.fn(),countJobs:vi.fn(),startAutomaticSync:vi.fn(),
  api:{...actual.api,listSources:vi.fn(),jobs:vi.fn()}}});
let container:HTMLDivElement,root:Root,client:QueryClient,profile:string,stored:Record<string,string>;
const sources:Record<string,Source[]>={semiconductor:[{id:"chip",name:"Chip Co",baseUrl:"https://chip.test",adapterId:"json",adapterVersion:"1",enabled:true,kind:"active",robotsOverride:false,jobCount:3,closedCount:0}],
 "data-science":[{id:"data",name:"Data Co",baseUrl:"https://data.test",adapterId:"json",adapterVersion:"1",enabled:true,kind:"active",robotsOverride:false,jobCount:3,closedCount:0}]};
const filter=(title:string,countries:string[]=[])=>JSON.stringify({title,keyword:"",sort:"recent",savedOnly:false,includeClosed:false,postedWithinDays:365,countries,includeUnknownLocations:false});
const settle=(ms=10)=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,ms))});
const titleBox=()=>container.querySelector('input[name="title"]')as HTMLInputElement;
const chosenCountries=()=>[...container.querySelectorAll(".chosen button")].map(button=>button.textContent?.replace("×","").trim());
async function type(value:string){await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(titleBox(),value);titleBox().dispatchEvent(new Event("input",{bubbles:true}))})}
// What the profile switch does: the backend changes the active profile, then every tab's data is refetched.
async function switchTo(next:string){profile=next;await act(async()=>{await Promise.all(["profiles","sources"].map(key=>client.invalidateQueries({queryKey:[key]})))});await settle(30)}
beforeEach(async()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false,refetchOnWindowFocus:false}}});
 profile="semiconductor";
 stored={"jobs.activeFilter.semiconductor":filter("verification",["PT"]),"jobs.activeFilter.data-science":filter("data scientist")};
 vi.mocked(listProfiles).mockImplementation(async():Promise<ProfilesState>=>({active:profile,profiles:[]}));
 vi.mocked(getSetting).mockImplementation(async key=>stored[key]??null);
 vi.mocked(setSetting).mockImplementation(async(key,value)=>{stored[key]=value});
 vi.mocked(api.listSources).mockImplementation(async()=>sources[profile]);
 vi.mocked(api.jobs).mockResolvedValue({items:[],total:0,offset:0,hasMore:false});
 vi.mocked(jobCountries).mockResolvedValue([[{code:"PT",jobs:3}],0]);
 vi.mocked(countJobs).mockResolvedValue(0);vi.mocked(startAutomaticSync).mockResolvedValue(false);
 await act(async()=>root.render(<QueryClientProvider client={client}><Jobs onCount={()=>{}}/></QueryClientProvider>));await settle(30);
});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.clearAllMocks()});

it("opens each profile on its own saved filters, and comes back to the first exactly as it was",async()=>{
 expect(titleBox().value).toBe("verification");expect(chosenCountries()).toEqual(["Portugal"]);
 await switchTo("data-science");
 expect(titleBox().value).toBe("data scientist");expect(chosenCountries()).toEqual([]);
 await switchTo("semiconductor");
 expect(titleBox().value).toBe("verification");expect(chosenCountries()).toEqual(["Portugal"]);
});
it("saves what is typed under the profile it was typed in, and never under the other",async()=>{
 await switchTo("data-science");
 await type("machine learning");await settle(400);
 expect(JSON.parse(stored["jobs.activeFilter.data-science"]).title).toBe("machine learning");
 expect(JSON.parse(stored["jobs.activeFilter.semiconductor"]).title).toBe("verification");
 await switchTo("semiconductor");expect(titleBox().value).toBe("verification");
 await switchTo("data-science");expect(titleBox().value).toBe("machine learning");
});
it("keeps the last word typed even when the profile is switched before the pause that saves it",async()=>{
 await type("rtl design");await switchTo("data-science");
 expect(JSON.parse(stored["jobs.activeFilter.semiconductor"]).title).toBe("rtl design");
 expect(titleBox().value).toBe("data scientist");
});
it("starts a profile that never saved filters of its own from the ones every profile shared",async()=>{
 stored={"jobs.activeFilter":filter("shared search",["PT"])};
 await switchTo("data-science");
 expect(titleBox().value).toBe("shared search");
 await type("analytics");await settle(400);
 expect(JSON.parse(stored["jobs.activeFilter.data-science"]).title).toBe("analytics");
 expect(JSON.parse(stored["jobs.activeFilter"]).title).toBe("shared search");
});
