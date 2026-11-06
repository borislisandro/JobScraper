/** @vitest-environment jsdom */
// The Applications card is three buttons: Open website, then Apply / Back to saved, then
// Unsave / Remove. Nothing else to open.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {DndContext}from "@dnd-kit/core";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,type Application}from "./api";
import {AppCard}from "./main";
vi.mock("@tauri-apps/plugin-dialog",()=>({ask:vi.fn()}));
vi.mock("@tauri-apps/plugin-opener",()=>({openUrl:vi.fn()}));
import {ask}from "@tauri-apps/plugin-dialog";
import {openUrl}from "@tauri-apps/plugin-opener";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const saved:Application={id:"app-1",jobId:"job-1",currentStage:"planned",title:"Digital Verification Engineer",company:"Chip Co",pendingConfirmation:false,canonicalUrl:"https://chip.test/jobs/1",applyUrl:"https://chip.test/apply/1"};
const applied:Application={...saved,currentStage:"applied",appliedAt:"2026-11-01T10:00:00Z"};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 vi.mocked(openUrl).mockResolvedValue(undefined)});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(app:Application,refresh=vi.fn()){
 await act(async()=>root.render(<QueryClientProvider client={client}><DndContext><AppCard app={app} onStatus={()=>{}} refresh={refresh}/></DndContext></QueryClientProvider>));
 await settle();return refresh}
const labels=()=>Array.from(container.querySelectorAll("button")).map(b=>b.textContent);
const click=async(label:string)=>{await act(async()=>{(Array.from(container.querySelectorAll("button")).find(b=>b.textContent===label)as HTMLButtonElement).click()});await settle()};

it("a saved card offers exactly Open website, Apply and Unsave",async()=>{
 await render(saved);
 expect(labels()).toEqual(["Open website","Apply","Unsave"]);
});
it("an applied card offers exactly Open website, Back to saved and Remove",async()=>{
 await render(applied);
 expect(labels()).toEqual(["Open website","Back to saved","Remove"]);
});
it("Open website opens the listing page without recording anything",async()=>{
 const stage=vi.spyOn(api,"stage");const openApply=vi.spyOn(api,"openApply");
 await render(saved);await click("Open website");
 expect(openUrl).toHaveBeenCalledWith("https://chip.test/jobs/1");
 expect(stage).not.toHaveBeenCalled();expect(openApply).not.toHaveBeenCalled();
});
it("Apply marks a saved job applied",async()=>{
 const stage=vi.spyOn(api,"stage").mockResolvedValue(applied);
 const refresh=await render(saved);await click("Apply");
 expect(stage).toHaveBeenCalledWith("app-1","applied",expect.any(String),true);
 expect(refresh).toHaveBeenCalled();
});
it("Apply answers a pending submit question instead of leaving it open",async()=>{
 const decide=vi.spyOn(api,"applyDecision").mockResolvedValue(undefined);const stage=vi.spyOn(api,"stage");
 await render({...saved,pendingConfirmation:true});await click("Apply");
 expect(decide).toHaveBeenCalledWith("app-1","yes");expect(stage).not.toHaveBeenCalled();
});
it("Back to saved moves an applied job back to Saved",async()=>{
 const stage=vi.spyOn(api,"stage").mockResolvedValue(saved);
 await render(applied);await click("Back to saved");
 expect(stage).toHaveBeenCalledWith("app-1","planned",expect.any(String),true);
});
it("Unsave removes a saved job straight away",async()=>{
 const del=vi.spyOn(api,"deleteApplication").mockResolvedValue(undefined);
 await render(saved);await click("Unsave");
 expect(ask).not.toHaveBeenCalled();expect(del).toHaveBeenCalledWith("app-1");
});
it("Remove asks first on an applied job and does nothing if declined",async()=>{
 vi.mocked(ask).mockResolvedValue(false);
 const del=vi.spyOn(api,"deleteApplication").mockResolvedValue(undefined);
 await render(applied);await click("Remove");
 expect(ask).toHaveBeenCalled();expect(del).not.toHaveBeenCalled();
 vi.mocked(ask).mockResolvedValue(true);
 await click("Remove");
 expect(del).toHaveBeenCalledWith("app-1");
});
