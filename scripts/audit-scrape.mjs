// Scrapes every catalog company with the real worker and stores what the app would store (titles, places, links), so any search term
// can be counted afterwards without scraping again.   node scripts/audit-scrape.mjs [--concurrency 4] [--only "Name,Name"]
import {readFile,writeFile,mkdir}from "node:fs/promises";
import {runWorker}from "./worker-proof.mjs";
const args=process.argv.slice(2),opt=name=>{const i=args.indexOf(`--${name}`);return i>=0?args[i+1]:null};
const concurrency=Number(opt("concurrency")||4),only=opt("only")?.split(",").map(name=>name.trim());
const outDir=new URL("../docs/scrape-audit/corpus/",import.meta.url);await mkdir(outDir,{recursive:true});
const catalog=JSON.parse(await readFile(new URL("../sidecar/company-catalog.json",import.meta.url),"utf8"));
const slug=name=>name.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"");
const {existsSync}=await import("node:fs"),[shard,shards]=(opt("shard")||"0/1").split("/").map(Number);
// --skip-done resumes an interrupted run; --shard i/n lets several processes split the list (different hosts, so pacing is not shared).
const companies=catalog.companies.filter(company=>company.kind!=="reference"&&(!only||only.includes(company.name))).filter((company,index)=>index%shards===shard).filter(company=>!(args.includes("--skip-done")&&existsSync(new URL(`${slug(company.name)}.json`,outDir))));
let next=0,done=0;
async function work(){
 while(next<companies.length){
  const company=companies[next++],started=Date.now(),source={id:company.id,name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,kind:"active",robotsOverride:true,allowPrivateNetwork:false,configJson:company.config};
  let record;
  try{
   const run=await runWorker(source,"scrape_source",{known:[],deferDetails:true},900_000),payload=run.terminal?.payload||{};
   record={name:company.name,adapterId:company.adapterId,baseUrl:company.baseUrl,ok:run.terminal?.event==="completed"&&!run.error,error:run.error||(run.terminal?.event!=="completed"?payload.message||payload.code||run.terminal?.event:null),
    complete:Boolean(payload.complete),boardTotal:payload.boardTotal??null,boardTotalExact:Boolean(payload.boardTotalExact),discovered:payload.discovered??run.jobs.length,warnings:payload.warnings||[],
    durationMs:Date.now()-started,jobs:run.jobs.map(job=>({id:job.externalId,title:job.title,location:job.location,url:job.canonicalUrl||job.applyUrl}))}
  }catch(error){record={name:company.name,adapterId:company.adapterId,baseUrl:company.baseUrl,ok:false,error:String(error.message||error),complete:false,boardTotal:null,jobs:[],durationMs:Date.now()-started}}
  await writeFile(new URL(`${slug(company.name)}.json`,outDir),JSON.stringify(record));
  console.log(`${++done}/${companies.length} ${company.name}: ${record.ok?"ok":"FAIL"} ${record.jobs.length} jobs${record.complete?"":" (incomplete)"} ${Math.round(record.durationMs/1000)}s${record.error?" — "+String(record.error).slice(0,100):""}`);
 }}
await Promise.all(Array.from({length:concurrency},work));
