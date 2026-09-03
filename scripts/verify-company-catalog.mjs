// Opt-in live verification. --full proves traversal, descriptions and a warm change check.
// --candidate accepts a worker source JSON before catalog admission; --report saves evidence.
import {readFile,mkdir,writeFile}from "node:fs/promises";
import {dirname,resolve}from "node:path";
import {proveSource}from "./worker-proof.mjs";
const args=process.argv.slice(2),names=[];let full=false,candidate,report;
for(let i=0;i<args.length;i++){
 if(args[i]==="--full")full=true;
 else if(args[i]==="--candidate"||args[i]==="--report"){
  const flag=args[i],value=args[++i];if(!value||value.startsWith("--"))throw Error(`Missing value for ${flag}`);
  if(flag==="--candidate")candidate=value;else report=value;
 }else if(args[i].startsWith("--"))throw Error(`Unknown option: ${args[i]}`);else names.push(args[i]);
}
if(candidate&&names.length)throw Error("Choose catalog names or one candidate file");
const catalog=JSON.parse(await readFile(new URL("../sidecar/company-catalog.json",import.meta.url),"utf8"));
const companies=catalog.companies.filter(company=>company.kind!=="reference"&&(!names.length||names.includes(company.name)));
for(const name of names)if(!companies.some(company=>company.name===name))throw Error(`Unknown supported company: ${name}`);
const sources=candidate?[JSON.parse(await readFile(candidate,"utf8"))]:companies.map(company=>({
 id:company.id,name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,kind:"active",robotsOverride:true,allowPrivateNetwork:false,configJson:company.config}));
const results=[];
for(const source of sources){
 const result=await proveSource(source,{full});results.push(result);
 console.log(JSON.stringify({name:result.name,level:result.level,ok:result.ok,jobs:result.listing.jobs,
  boardTotal:result.listing.terminal?.payload.boardTotal,complete:result.listing.terminal?.payload.complete,errors:result.errors}));
 // Processes do not share pacing history when two employers use the same ATS origin.
 await new Promise(resolve=>setTimeout(resolve,400));
}
if(report){const path=resolve(report);await mkdir(dirname(path),{recursive:true});await writeFile(path,JSON.stringify(results.length===1?results[0]:results,null,2)+"\n")}
const failures=results.filter(result=>!result.ok).length;
console.error(`${results.length-failures}/${results.length} live boards passed (${full?"full proof":"preview"}).`);
process.exitCode=failures?1:0;
