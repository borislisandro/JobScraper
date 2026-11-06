// Runs the app's own "paste a careers URL" detector (the worker's probe_source command) on a URL, so
// a JavaScript-only careers page can be tested end to end, including the browser it renders in.
//   node scripts/probe-url.mjs "Critical Software" https://careers.criticalsoftware.com/
import {runWorker}from "./worker-proof.mjs";
const [name,url]=process.argv.slice(2);
if(!name||!url)throw Error("Usage: probe-url.mjs <name> <url>");
const source={id:"probe",name,baseUrl:url,adapterId:"static-css",kind:"active",robotsOverride:true,allowPrivateNetwork:false,configJson:{}};
const result=await runWorker(source,"probe_source",{url},180_000);
const payload=result.terminal?.payload||{};
console.log(JSON.stringify({name,event:result.terminal?.event,error:result.error,payload},null,1).slice(0,2500));
