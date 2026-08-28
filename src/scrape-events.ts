import type { WorkerEvent } from "./api";

export type TauriWorkerListen=(event:string,handler:(event:{payload:WorkerEvent})=>void)=>Promise<()=>void>;

/** Event subscription is scoped to Sources and never polls scraper state. */
export function listenForScrapeEvents(listen:TauriWorkerListen,onEvent:(event:WorkerEvent)=>void){
 let active=true;let stop:(()=>void)|undefined;
 void listen("scrape-event",event=>{if(active)onEvent(event.payload)}).then(unlisten=>{if(active)stop=unlisten;else unlisten()});
 return()=>{active=false;stop?.()};
}
