/** @vitest-environment jsdom */
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,type Source}from "./api";
import {readAgo,SourceRow}from "./main";
import type {RowProgress}from "./source-progress";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const source:Source={id:"s",name:"Chip Company",baseUrl:"https://www.chip.example/careers",adapterId:"ashby",adapterVersion:"1",kind:"active",enabled:true,robotsOverride:false,jobCount:42,closedCount:0,lastSuccessAt:new Date(1_000_000-5*60_000).toISOString()};
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);client=new QueryClient();vi.spyOn(api,"listSources").mockResolvedValue([])});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks()});
const render=(progress?:RowProgress,handlers:Partial<{onTest:()=>void;onEdit:()=>void;onToggle:(on:boolean)=>void;onDelete:()=>void}>={})=>act(()=>root.render(<QueryClientProvider client={client}>
 <SourceRow source={source} progress={progress} now={1_000_000} typicalMs={20_000} batchRunning={false} onTest={handlers.onTest??(()=>{})} onEdit={handlers.onEdit??(()=>{})} onToggle={handlers.onToggle??(()=>{})} onDelete={handlers.onDelete??(()=>{})}/></QueryClientProvider>));

it("shows the host, how long ago it was read, a switch, and Test, Edit and Delete when idle",()=>{
 const toggled=vi.fn();render(undefined,{onToggle:toggled});
 expect(container.querySelector(".source-url")?.textContent).toBe("chip.example");
 expect(container.querySelector(".source-health")?.textContent).toBe("5 min ago");
 expect([...container.querySelectorAll("button")].map(button=>button.textContent)).toEqual(["Test","Edit","Delete"]);
 expect(container.querySelector('[role="progressbar"]')).toBeNull();
 act(()=>(container.querySelector('[role="switch"]')as HTMLInputElement).click());
 expect(toggled).toHaveBeenCalledWith(false);
});
it("opens the source's settings from Edit, so a browser session can still be captured",()=>{
 const edited=vi.fn();render(undefined,{onEdit:edited});
 act(()=>([...container.querySelectorAll("button")].find(button=>button.textContent==="Edit")as HTMLButtonElement).click());
 expect(edited).toHaveBeenCalled();
});

it("puts a progress bar on the row while it is read, and offers Stop instead of Test",()=>{
 render({phase:"reading",runId:"r",requests:4,startedAt:1_000_000-10_000});
 const bar=container.querySelector('[role="progressbar"]');
 expect(bar?.getAttribute("aria-valuenow")).toBe("50");
 expect(container.querySelector(".source-health")?.textContent).toBe("Reading the board · 4 requests · 10.0s");
 expect([...container.querySelectorAll("button")].map(button=>button.textContent)).toEqual(["Stop","Edit","Delete"]);
});

it("reads ages in minutes, hours, then days",()=>{
 const now=Date.UTC(2026,10,7,12);
 expect(readAgo(new Date(now-30_000).toISOString(),now)).toBe("Just now");
 expect(readAgo(new Date(now-3*3_600_000).toISOString(),now)).toBe("3 h ago");
 expect(readAgo(undefined,now)).toBe("");
});
