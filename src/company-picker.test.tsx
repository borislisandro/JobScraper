/** @vitest-environment jsdom */
import React,{act}from "react";
import {createRoot,type Root}from "react-dom/client";
import {QueryClient,QueryClientProvider,useQuery}from "@tanstack/react-query";
import {afterEach,beforeEach,expect,it,vi}from "vitest";
import {api,listCompanyCatalog,sourceConfig,type CatalogCompany,type Source}from "./api";
import {CompanyPicker,SourceForm}from "./main";
import {catalogSource}from "./source-ui";
vi.mock("./api",async original=>({...await original<typeof import("./api")>(),listCompanyCatalog:vi.fn(),sourceConfig:vi.fn()}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement,root:Root,client:QueryClient;
const company:CatalogCompany={id:"catalog-id",name:"Chip Company",baseUrl:"https://jobs.ashbyhq.com/chip",adapterId:"ashby",country:"CA",tags:["semiconductor"],starter:false,kind:"active",config:{token:"chip",maxPages:500,requestDelayMs:400}};
const saved:Source={id:"saved-id",name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,adapterVersion:"1.1.0",kind:"active",enabled:true,robotsOverride:false,jobCount:0,closedCount:0};
const settle=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10))});
beforeEach(()=>{container=document.createElement("div");document.body.append(container);root=createRoot(container);client=new QueryClient({defaultOptions:{queries:{retry:false},mutations:{retry:false}}});
 Object.assign(HTMLDialogElement.prototype,{showModal(this:HTMLDialogElement){this.open=true},close(this:HTMLDialogElement){this.open=false}});
 vi.mocked(listCompanyCatalog).mockResolvedValue([company]);vi.mocked(sourceConfig).mockResolvedValue({...company.config,headless:false});
});
afterEach(()=>{act(()=>root.unmount());container.remove();client.clear();vi.restoreAllMocks();vi.clearAllMocks()});
async function render(child:React.ReactNode){await act(async()=>root.render(<QueryClientProvider client={client}>{child}</QueryClientProvider>));await settle()}
function Picker(){const {data:sources=[]}=useQuery({queryKey:["sources"],queryFn:api.listSources});return <CompanyPicker sources={sources} onClose={()=>{}} onManual={()=>{}}/>}
it("adds a catalog company with all curated settings, without probing or reusing a deleted starter ID",async()=>{
 let sources:Source[]=[];vi.spyOn(api,"listSources").mockImplementation(async()=>sources);
 const save=vi.spyOn(api,"saveSource").mockImplementation(async()=>{sources=[saved];return saved}),probe=vi.spyOn(api,"probe");
 await render(<Picker/>);const button=container.querySelector('[aria-label="Add Chip Company"]')as HTMLButtonElement;
 await act(async()=>button.click());await settle();
 expect(save).toHaveBeenCalledWith({name:company.name,baseUrl:company.baseUrl,adapterId:"ashby",kind:"active",enabled:true,disabledReason:null,robotsOverride:true,allowPrivateNetwork:false,configJson:{schemaVersion:"1.1.0",adapterVersion:"1.1.0",mode:"direct",...company.config}});
 expect(probe).not.toHaveBeenCalled();expect((container.querySelector('[aria-label="Added Chip Company"]')as HTMLButtonElement).disabled).toBe(true);
});
it("enables a renamed existing source and preserves its settings",async()=>{
 const existing={...saved,name:"My chips",enabled:false,baseUrl:"https://JOBS.ASHBYHQ.COM/chip/"};expect(catalogSource(company,[existing])).toEqual(existing);
 const enable=vi.spyOn(api,"setSourceEnabled").mockResolvedValue(undefined),save=vi.spyOn(api,"saveSource");
 await render(<CompanyPicker sources={[existing]} onClose={()=>{}} onManual={()=>{}}/>);
 await act(async()=>{(container.querySelector('[aria-label="Enable Chip Company"]')as HTMLButtonElement).click()});await settle();
 expect(enable).toHaveBeenCalledWith("saved-id",true);expect(save).not.toHaveBeenCalled();
});
it("searches by country and sector, exposes manual fallback, and keeps save errors visible",async()=>{
 const manual=vi.fn();vi.spyOn(api,"saveSource").mockRejectedValue("That board is already followed.");
 await render(<CompanyPicker sources={[]} onClose={()=>{}} onManual={manual}/>);
 const input=container.querySelector('input[type="search"]')as HTMLInputElement;
 const search=async(value:string)=>{await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}))})};
 await search("Canada semiconductor");expect(container.querySelectorAll(".company-row")).toHaveLength(1);
 await search("No such company");expect(container.textContent).toContain("No matching companies");
 await search("");await act(async()=>{(container.querySelector('[aria-label="Add Chip Company"]')as HTMLButtonElement).click()});await settle();
 expect(container.querySelector('[role="status"]')?.textContent).toContain("already followed");
 await act(async()=>{(Array.from(container.querySelectorAll("button")).find(button=>button.textContent==="Add source manually")as HTMLButtonElement).click()});expect(manual).toHaveBeenCalledOnce();
});
it("saving a catalog source with Advanced closed retains its adapter, token and headless preference",async()=>{
 const onSave=vi.fn();await render(<SourceForm source={saved} onCancel={()=>{}} onSave={onSave}/>);
 await act(async()=>{container.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true}))});
 expect(onSave).toHaveBeenCalledWith(expect.objectContaining({manual:true,adapterId:"ashby",headless:false,config:expect.objectContaining({token:"chip",requestDelayMs:400})}));
});
