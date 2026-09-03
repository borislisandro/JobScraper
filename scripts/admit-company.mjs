// Admission is the one step that writes to the shared catalog and tracker, and it is done ~269
// times. Doing it by hand is how a proof and the configuration it certifies drift apart, which is
// exactly what the contract test fails on — so the proof file is the only input for the parts that
// have to match, and everything else is refused unless the proof passed.
//
// node scripts/admit-company.mjs <trackerId> <proof.json> --country NL --tags semiconductor,equipment
//   --careers <official careers URL> [--notes "..."]
import {readFile,writeFile}from "node:fs/promises";
const args=process.argv.slice(2),positional=[],options={};
for(let i=0;i<args.length;i++)if(args[i].startsWith("--")){const key=args[i].slice(2),value=args[++i];if(value===undefined||value.startsWith("--"))throw Error(`Missing value for --${key}`);options[key]=value}else positional.push(args[i]);
const [trackerId,proofPath]=positional;
if(!trackerId||!proofPath)throw Error("Usage: admit-company.mjs <trackerId> <proof.json> --country XX --tags a,b --careers URL [--notes ...]");
for(const required of["country","tags","careers"])if(!options[required])throw Error(`--${required} is required`);

const catalogUrl=new URL("../sidecar/company-catalog.json",import.meta.url);
const trackerUrl=new URL("../docs/company-support-tracker.json",import.meta.url);
const [catalog,tracker,proof]=await Promise.all([catalogUrl,trackerUrl,proofPath].map(async source=>JSON.parse(await readFile(source,"utf8"))));

if(!proof.ok||proof.level!=="full"||proof.errors.length)throw Error(`${proofPath} is not a passing full proof: ${JSON.stringify(proof.errors)}`);
if(!proof.listing.terminal?.payload?.complete)throw Error("Proof traversal is not complete");
const row=tracker.companies.find(company=>company.id===trackerId);
if(!row)throw Error(`Unknown tracker id: ${trackerId}`);
if(row.name!==proof.source.name)throw Error(`Proof is for "${proof.source.name}", tracker row is "${row.name}"`);
if(catalog.companies.some(company=>company.name===row.name))throw Error(`${row.name} is already in the catalog`);
if(catalog.companies.some(company=>company.baseUrl===proof.source.baseUrl))throw Error(`Another catalog board already uses ${proof.source.baseUrl}`);

// Catalog IDs are a UUIDv4-shaped sequence; the next one continues it rather than restarting.
const prefix="00000000-0000-4000-8000-";
const next=Math.max(...catalog.companies.map(company=>Number.parseInt(company.id.slice(prefix.length),16)||0))+1;
const id=prefix+next.toString(16).padStart(12,"0");
// The contract test reads the catalog path from the repo root, and the plan links the tracker
// path with "docs/" stripped, so both are derived from one repo-relative path.
const evidence=proofPath.replaceAll("\\","/").replace(/^\.\//,"");
if(!evidence.startsWith("docs/company-proofs/"))throw Error("Proofs belong in docs/company-proofs/");
catalog.companies.push({id,name:row.name,country:options.country,tags:options.tags.split(",").map(tag=>tag.trim()).filter(Boolean),
 adapterId:proof.source.adapterId,baseUrl:proof.source.baseUrl,kind:"active",
 disabledReason:"Starter source is disabled until you review and enable it.",
 config:proof.source.configJson,starter:false,
 verified:{at:proof.finishedAt.slice(0,10),jobs:proof.listing.jobs,level:"full",evidence}});

Object.assign(row,{status:"supported",officialCareersUrl:options.careers,adapterId:proof.source.adapterId,catalogId:id,
 lastAttemptAt:proof.finishedAt,evidence:[evidence.replace(/^docs\//,"")],
 notes:options.notes||row.notes,nextAction:"Complete; revalidate if this employer changes its board."});
tracker.updatedAt=proof.finishedAt.slice(0,10);

await writeFile(catalogUrl,JSON.stringify(catalog,null,1)+"\n");
await writeFile(trackerUrl,JSON.stringify(tracker,null,2)+"\n");
console.log(`${row.name} admitted as ${id} (${proof.listing.jobs} jobs, ${proof.source.adapterId}).`);
