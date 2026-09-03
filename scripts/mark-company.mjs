// The other half of admission: recording an outcome that is not a catalog entry. A blocked or
// still-being-worked row has to keep its findings, or the next pass rediscovers the same wall.
// Refuses "supported" outright — that goes through admit-company.mjs, which requires a proof.
//
// node scripts/mark-company.mjs <trackerId> blocked --notes "..." [--careers URL] [--adapter id] [--next "..."]
import {readFile,writeFile}from "node:fs/promises";
const statuses=["queued","discovering","implementing","validating","blocked","covered_by_parent"];
const args=process.argv.slice(2),positional=[],options={};
for(let i=0;i<args.length;i++)if(args[i].startsWith("--")){const key=args[i].slice(2),value=args[++i];if(value===undefined||value.startsWith("--"))throw Error(`Missing value for --${key}`);options[key]=value}else positional.push(args[i]);
const [trackerId,status]=positional;
if(!statuses.includes(status))throw Error(`Status must be one of: ${statuses.join(", ")}`);
if(!options.notes)throw Error("--notes is required: record what was observed, not just that it failed");
const url=new URL("../docs/company-support-tracker.json",import.meta.url);
const tracker=JSON.parse(await readFile(url,"utf8"));
const row=tracker.companies.find(company=>company.id===trackerId);
if(!row)throw Error(`Unknown tracker id: ${trackerId}`);
if(row.status==="supported")throw Error(`${row.name} is already supported; remove its catalog entry first`);
const now=new Date().toISOString();
Object.assign(row,{status,notes:options.notes,lastAttemptAt:now,
 ...(options.careers?{officialCareersUrl:options.careers}:{}),
 ...(options.adapter?{adapterId:options.adapter}:{}),
 ...(options.next?{nextAction:options.next}:{})});
tracker.updatedAt=now.slice(0,10);
await writeFile(url,JSON.stringify(tracker,null,2)+"\n");
console.log(`${row.name} -> ${status}`);
