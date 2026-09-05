/** @vitest-environment jsdom */
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {jobDescription,type Job,type JobDescription}from "./api";
import {JobReader}from "./main";
vi.mock("./api",async original=>({...await original<typeof import("./api")>(),jobDescription:vi.fn()}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const jobs=["A","B"].map(id=>({id,sourceId:"source",title:`Engineer ${id}`,company:"Company",descriptionText:"",descriptionStatus:"complete",availability:"active"}as Job));
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);client=new QueryClient({defaultOptions:{queries:{retry:false}}});vi.mocked(jobDescription).mockReset()});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear()});
const render=async(index=0)=>{await act(async()=>root.render(<QueryClientProvider client={client}><JobReader jobs={jobs} index={index} sourceName={()=>"Source"} onMove={()=>{}} onClose={()=>{}} onStatus={()=>{}}/></QueryClientProvider>))};
const refresh=async()=>{await act(async()=>{Array.from(container.querySelectorAll("button")).find(button=>button.textContent==="Refresh description")!.click()})};
it("refresh failure keeps cached text readable",async()=>{
 vi.mocked(jobDescription).mockResolvedValueOnce({text:"Existing qualifications",status:"complete"}).mockRejectedValueOnce(Error("Publisher offline"));
 await render();await refresh();
 expect(jobDescription).toHaveBeenLastCalledWith("A",true);
 expect(container.textContent).toContain("Existing qualifications");
 expect(container.textContent).toContain("Publisher offline");
});
it("late refresh results cannot overwrite the next job's description",async()=>{
 let finish!:(value:JobDescription)=>void;
 vi.mocked(jobDescription).mockResolvedValueOnce({text:"Original A",status:"complete"}).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve})).mockResolvedValueOnce({text:"Current B",status:"complete"});
 await render();await refresh();await render(1);
 await act(async()=>{finish({text:"Late A",status:"complete"})});
 expect(container.textContent).toContain("Current B");expect(container.textContent).not.toContain("Late A");
});
