// One driver for preview and acceptance proof. Only the worker reads the board; this process
// observes its public protocol and never substitutes a side-channel API response for a scrape.
import {spawn}from "node:child_process";
import {createInterface}from "node:readline";
import {readFile}from "node:fs/promises";
import {fileURLToPath}from "node:url";
import {createHash}from "node:crypto";
export function runWorker(source,command,extra={},timeoutMs=600_000){return new Promise((resolve,reject)=>{
 const child=spawn(process.execPath,[fileURLToPath(new URL("../sidecar/worker.mjs",import.meta.url))],{stdio:["pipe","pipe","pipe"]});
 const jobs=[],enrichedJobs=[],warnings=[];let terminal,stderr="",protocolError=null;
 const timer=setTimeout(()=>{protocolError="worker timed out";child.kill()},timeoutMs);
 createInterface({input:child.stdout}).on("line",line=>{try{const event=JSON.parse(line);if(event.protocolVersion!==1)throw Error("Unexpected protocol version");
  if(event.event==="job")jobs.push(event.payload);if(event.event==="enriched_job")enrichedJobs.push(event.payload);
  if(event.event==="warning"||event.event==="enrichment_failed")warnings.push(event.payload);
  if(["completed","failed","cancelled"].includes(event.event))terminal=event;
 }catch(error){protocolError=String(error);child.kill()}});
 child.stderr.on("data",chunk=>stderr+=chunk);child.on("error",error=>{clearTimeout(timer);reject(error)});
 child.on("close",code=>{clearTimeout(timer);resolve({terminal,jobs,enrichedJobs,warnings,...(protocolError||code||!terminal?{error:protocolError||stderr||`worker exited ${code} without a terminal event`}:{})})});
 child.stdin.end(JSON.stringify({protocolVersion:1,command,runId:source.id||"company-proof",source,...extra})+"\n");
})}
const sample=job=>({externalId:job.externalId,title:job.title,company:job.company,location:job.location,postedAt:job.postedAt,canonicalUrl:job.canonicalUrl,applyUrl:job.applyUrl,descriptionStatus:job.descriptionStatus,descriptionChars:(job.descriptionText||"").length});
const httpUrl=value=>{try{return["http:","https:"].includes(new URL(value).protocol)}catch{return false}};
export async function proveSource(source,{full=false,timeoutMs=600_000}={}){
 // Candidate files may carry convenient development flags. They are forbidden in evidence.
 // robots enforcement is deliberately NOT one of them: install_starter_pack seeds every catalog
 // source with robots_override=1, so a proof that ran with enforcement on would be certifying a
 // stricter run than the app actually performs. The URL guard and normal pacing stay required —
 // those are the SSRF defence and the politeness the boards actually feel.
 if(source.allowPrivateNetwork||source.configJson?.testNoDelay||source.configJson?.testJitterMs!==undefined)throw Error("Proof requires URL guards and normal pacing");
 const startedAt=new Date().toISOString(),cold=await runWorker(source,full?"scrape_source":"test_source",full?{known:[],deferDetails:true}:{},timeoutMs);
 const payload=cold.terminal?.payload||{},jobs=cold.jobs,errors=[];
 if(cold.error||cold.terminal?.event!=="completed")errors.push(cold.error||payload.message||"Worker did not complete");
 if(!jobs.length)errors.push("No listings returned");
 if(jobs.some(job=>!job.title?.trim()||!job.company?.trim()||!httpUrl(job.applyUrl||job.canonicalUrl)))errors.push("A job lacks a valid title, company or HTTP(S) link");
 const identities=jobs.map(job=>String(job.externalId||job.canonicalUrl||job.applyUrl)),hashes=jobs.map(job=>job.listingHash);
 if(new Set(identities).size!==jobs.length)errors.push("Duplicate job identities");
 if(full&&(!hashes.every(Boolean)||new Set(hashes).size!==jobs.length))errors.push("Missing or duplicate listing hashes");
 if(full&&!payload.complete)errors.push("Full traversal is unfinished");
 if(full&&payload.discovered!==jobs.length)errors.push("Cold read did not emit every discovered row; inspect duplicates or filtered records");
 if(full&&payload.boardTotalExact&&payload.discovered!==payload.boardTotal)errors.push("Discovered rows do not match the exact board total");
 const selected=[...new Set([0,Math.floor(jobs.length/2),jobs.length-1])].filter(index=>index>=0&&jobs[index]).map(index=>jobs[index]);
 let details=null,warm=null;
 if(full&&!errors.length){
  const pending=selected.filter(job=>job.descriptionStatus==="pending");
  if(pending.length){const result=await runWorker({...source,enrichmentJobs:pending.map((job,index)=>({...job,jobId:`proof-${index}`}))},"enrich_source",{},timeoutMs);
   details={terminal:result.terminal,samples:result.enrichedJobs.map(sample),warnings:result.warnings,error:result.error};
   if(result.error||result.terminal?.event!=="completed"||result.terminal.payload.failed||result.enrichedJobs.length!==pending.length||result.enrichedJobs.some(job=>!job.descriptionText?.trim()))errors.push("Deferred descriptions did not all load");
  }
  if(selected.some(job=>job.descriptionStatus!=="pending"&&!job.descriptionText?.trim()))errors.push("An inline description is empty");
  const result=await runWorker(source,"check_source",{known:hashes},timeoutMs);warm={terminal:result.terminal,error:result.error};
  if(result.error||result.terminal?.event!=="completed")errors.push("Warm check did not complete");
  else if(result.terminal.payload.fresh!==0)errors.push("Warm check found unseen listings; inspect board movement or unstable identities");
 }
 const codeHashes={};for(const path of["worker.mjs","adapters.mjs","company-adapters.mjs"])codeHashes[path]=createHash("sha256").update(await readFile(new URL(`../sidecar/${path}`,import.meta.url))).digest("hex");
 return{schemaVersion:1,name:source.name,level:full?"full":"preview",ok:errors.length===0,startedAt,finishedAt:new Date().toISOString(),source,codeHashes,
  listing:{terminal:cold.terminal,jobs:jobs.length,uniqueIdentities:new Set(identities).size,uniqueHashes:new Set(hashes).size,samples:selected.map(sample),warnings:cold.warnings,error:cold.error},details,warm,errors};
}
