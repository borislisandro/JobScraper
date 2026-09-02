/** @vitest-environment jsdom */
import React,{act,useState}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider}from "@tanstack/react-query";
import {afterEach,beforeEach,describe,expect,it,vi}from "vitest";
import {api,type Job}from "./api";
import {InterviewPanel,JobCard}from "./main";

(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement;let root:Root;
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container)});
afterEach(()=>{act(()=>root.unmount());container.remove();vi.restoreAllMocks()});
const client=()=>new QueryClient({defaultOptions:{queries:{retry:false,refetchOnWindowFocus:false}}});
const provider=(child:React.ReactNode)=>React.createElement(QueryClientProvider,{client:client()},child);

describe("hidden application work",()=>{
 it("fetches only the interview panel first opened and keeps its body mounted",async()=>{
  const interviews=vi.spyOn(api,"interviews").mockResolvedValue([]);
  await act(async()=>{root.render(provider(React.createElement(React.Fragment,null,...Array.from({length:100},(_,index)=>React.createElement(InterviewPanel,{key:index,applicationId:`app-${index}`,refresh:()=>{}})))))});
  expect(interviews).not.toHaveBeenCalled();
  const panel=container.querySelectorAll("details")[42] as HTMLDetailsElement;
  await act(async()=>{panel.open=true;panel.dispatchEvent(new Event("toggle"));await Promise.resolve()});
  expect(interviews).toHaveBeenCalledTimes(1);
  expect(interviews).toHaveBeenCalledWith("app-42");
  const field=panel.querySelector("input");
  await act(async()=>{panel.open=false;panel.dispatchEvent(new Event("toggle"))});
  expect(panel.querySelector("input")).toBe(field);
  await act(async()=>{panel.open=true;panel.dispatchEvent(new Event("toggle"));await Promise.resolve()});
  expect(interviews).toHaveBeenCalledTimes(1);
 });

 it("does not rerender an unchanged job card for parent progress updates",async()=>{
  let titleReads=0;
  const job={id:"job",sourceId:"source",get title(){titleReads+=1;return "Engineer"},company:"Company",descriptionText:"",descriptionStatus:"pending",availability:"active"}as Job;
  function Harness(){const [progress,setProgress]=useState(0);const [,setMessage]=useState("");return React.createElement(React.Fragment,null,React.createElement("button",{onClick:()=>setProgress(value=>value+1)},String(progress)),React.createElement(JobCard,{job,onStatus:setMessage}))}
  await act(async()=>root.render(provider(React.createElement(Harness))));
  const initialReads=titleReads;
  await act(async()=>{(container.querySelector("button")as HTMLButtonElement).click()});
  expect(titleReads).toBe(initialReads);
 });
});
