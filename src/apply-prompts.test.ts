import { describe, expect, it } from "vitest";
import { listenForApplyConfirmation, type ApplyAttempt } from "./apply-prompts";

describe("typed apply-confirmation listener",()=>{
  it("consumes native event and unregisters on cleanup",async()=>{
    let handler:((event:{payload:ApplyAttempt})=>void)|undefined;let stopped=0;const received:ApplyAttempt[]=[];
    const stop=listenForApplyConfirmation(async(_event,next)=>{handler=next;return()=>{stopped++}},value=>received.push(value));
    await Promise.resolve();handler?.({payload:{applicationId:"app",attemptId:"attempt"}});expect(received).toEqual([{applicationId:"app",attemptId:"attempt"}]);stop();expect(stopped).toBe(1);
  });
  it("unregisters when component unmounts before Tauri listener resolves",async()=>{
    let release:(value:()=>void)=>void=()=>{};let stopped=0;const stop=listenForApplyConfirmation(()=>new Promise(resolve=>{release=resolve}),()=>{});stop();release(()=>{stopped++});await Promise.resolve();expect(stopped).toBe(1);
  });
});
