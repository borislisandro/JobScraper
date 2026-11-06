// Admits every manifest entry whose proof passed: adds a tracker row, then hands the proof to the
// repo's own admit-company.mjs, which writes the catalog entry from the proof so the two cannot drift.
// Idempotent: entries already in the catalog are skipped.
//   node docs/ds-portugal/admit-all.mjs
import {readFile,writeFile}from "node:fs/promises";
import {spawnSync}from "node:child_process";
const root=new URL("../../",import.meta.url);
const read=async path=>JSON.parse(await readFile(new URL(path,root),"utf8"));
const manifest=await read("docs/ds-portugal/manifest.json");
for(const entry of manifest){
 const proofPath=`docs/company-proofs/ds-pt-${entry.id}.json`;
 let proof;try{proof=await read(proofPath)}catch{console.log(`wait  ${entry.id} (no proof yet)`);continue}
 if(!proof.ok||proof.level!=="full"){console.log(`wait  ${entry.id} (proof did not pass)`);continue}
 const catalog=await read("sidecar/company-catalog.json");
 if(catalog.companies.some(company=>company.name===proof.source.name)){console.log(`skip  ${proof.source.name} (already in catalog)`);continue}
 const tracker=await read("docs/company-support-tracker.json"),trackerId=`ds-pt-${entry.id}`;
 if(!tracker.companies.some(row=>row.id===trackerId))
  tracker.companies.push({sequence:tracker.companies.length+1,id:trackerId,name:proof.source.name,category:"Data and AI employers in Portugal",priority:"P1",status:"queued",
   officialCareersUrl:entry.careers,adapterId:null,catalogId:null,lastAttemptAt:null,evidence:[],notes:"",nextAction:"Admit from the passing full proof.",batch:"ds-portugal",inScope:true});
 await writeFile(new URL("docs/company-support-tracker.json",root),JSON.stringify(tracker,null,2)+"\n");
 const result=spawnSync(process.execPath,["scripts/admit-company.mjs",trackerId,proofPath,"--country",entry.country,"--tags",entry.tags.join(","),"--careers",entry.careers,"--notes",`Portugal data/AI employer; ${proof.listing.jobs} rows on the board at proof time.`],{cwd:new URL(".",root).pathname,encoding:"utf8"});
 console.log(result.status===0?`admit ${result.stdout.trim()}`:`FAIL  ${proof.source.name}: ${result.stderr.trim().split("\n").pop()}`);
}
