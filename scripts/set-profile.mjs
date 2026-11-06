// Puts catalog companies into (or takes them out of) a profile, and bumps the catalog version so installs reconcile.
//   node scripts/set-profile.mjs <profile-id> "Company" "Company" ... [--remove] [--version 2026-12-01.1]
import {readFileSync,writeFileSync}from "node:fs";
const args=process.argv.slice(2),remove=args.includes("--remove"),versionAt=args.indexOf("--version"),version=versionAt>=0?args[versionAt+1]:null;
const names=args.filter((arg,index)=>!arg.startsWith("--")&&index!==versionAt+1&&index>0),profile=args[0];
if(!profile||!names.length)throw Error('Usage: set-profile.mjs <profile-id> "Company" ... [--remove] [--version X]');
const url=new URL("../sidecar/company-catalog.json",import.meta.url),catalog=JSON.parse(readFileSync(url,"utf8"));
if(!catalog.profiles.some(entry=>entry.id===profile))throw Error(`Unknown profile ${profile}; known: ${catalog.profiles.map(entry=>entry.id).join(", ")}`);
for(const name of names){const company=catalog.companies.find(entry=>entry.name===name);if(!company)throw Error(`Not in the catalog: ${name}`);
 if(company.kind!=="active")throw Error(`${name} is not an active board`);
 const next=new Set(company.profiles||[]);if(remove)next.delete(profile);else next.add(profile);
 if(next.size)company.profiles=[...next];else delete company.profiles}
if(version)catalog.version=version;
writeFileSync(url,JSON.stringify(catalog,null,1)+"\n");
console.log(`${remove?"removed from":"added to"} ${profile}: ${names.length} compan${names.length===1?"y":"ies"}${version?`, catalog version ${version}`:""}`);
