import { describe, expect, it, vi } from "vitest";
import { listenForScrapeEvents } from "./scrape-events";

describe("scrape event listener",()=>{
 it("forwards pause and terminal events then cleans up",async()=>{
  let handler:((event:{payload:any})=>void)|undefined;const stop=vi.fn();
  const seen:any[]=[];const cleanup=listenForScrapeEvents(async(_name,callback)=>{handler=callback;return stop},event=>seen.push(event));
  await Promise.resolve();handler?.({payload:{event:"needs_user_action",runId:"run",payload:{kind:"login"}}});handler?.({payload:{event:"cancelled",runId:"run",payload:{complete:false}}});
  expect(seen.map(x=>x.event)).toEqual(["needs_user_action","cancelled"]);cleanup();expect(stop).toHaveBeenCalledOnce();
 });
});
