/** @vitest-environment jsdom */
// Two things reported against the "open a saved application" drawer:
//  - there was no way to remove an application once saved, only to dismiss the underlying job;
//  - moving the Stage dropdown straight to Applied was an unconditional block with no way through,
//    even though the backend already supports a manual override with a stated reason.
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,type Application,type ApplicationDetails}from "./api";
import {ApplicationDrawer}from "./main";
vi.mock("@tauri-apps/plugin-dialog",()=>({ask:vi.fn()}));
import {ask}from "@tauri-apps/plugin-dialog";
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const application:Application={id:"app-1",jobId:"job-1",currentStage:"planned",title:"Digital Verification Engineer",company:"Chip Co",pendingConfirmation:false};
const details:ApplicationDetails={application,events:[],notes:[],documents:[]};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);
 client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 vi.spyOn(api,"applicationDetails").mockResolvedValue(details)});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(onClose=vi.fn(),refresh=vi.fn()){
 await act(async()=>root.render(<QueryClientProvider client={client}><ApplicationDrawer applicationId={application.id} onClose={onClose} refresh={refresh}/></QueryClientProvider>));
 await settle();return{onClose,refresh}}
const click=async(label:string)=>{await act(async()=>{(Array.from(container.querySelectorAll("button")).find(b=>b.textContent===label)as HTMLButtonElement).click()});await settle()};

it("deletes the application after the user confirms, then closes and refreshes the board",async()=>{
 vi.mocked(ask).mockResolvedValue(true);
 const del=vi.spyOn(api,"deleteApplication").mockResolvedValue(undefined);
 const {onClose,refresh}=await render();
 await click("Remove from saved");
 expect(ask).toHaveBeenCalled();
 expect(del).toHaveBeenCalledWith("app-1");
 expect(onClose).toHaveBeenCalled();expect(refresh).toHaveBeenCalled();
});
it("declining the confirmation leaves the application untouched",async()=>{
 vi.mocked(ask).mockResolvedValue(false);
 const del=vi.spyOn(api,"deleteApplication");
 const {onClose}=await render();
 await click("Remove from saved");
 expect(del).not.toHaveBeenCalled();expect(onClose).not.toHaveBeenCalled();
});
it("moving Stage straight to Applied asks for a reason and sends it as a manual override",async()=>{
 const stage=vi.spyOn(api,"stage").mockResolvedValue(application);
 vi.stubGlobal("prompt",vi.fn().mockReturnValue("Applied directly on the company site"));
 await render();
 const select=container.querySelector("select")as HTMLSelectElement;
 await act(async()=>{select.value="applied";select.dispatchEvent(new Event("change",{bubbles:true}))});await settle();
 expect(prompt).toHaveBeenCalled();
 expect(stage).toHaveBeenCalledWith("app-1","applied","Applied directly on the company site",true);
});
it("cancelling the reason prompt does not move the stage",async()=>{
 const stage=vi.spyOn(api,"stage");
 vi.stubGlobal("prompt",vi.fn().mockReturnValue(null));
 await render();
 const select=container.querySelector("select")as HTMLSelectElement;
 await act(async()=>{select.value="applied";select.dispatchEvent(new Event("change",{bubbles:true}))});await settle();
 expect(stage).not.toHaveBeenCalled();
});
