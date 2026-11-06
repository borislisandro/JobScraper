// Re-points an existing catalog entry at the board an employer moved to, from a passing full proof, so the entry and the evidence
// that certifies it cannot drift. admit-company.mjs refuses an employer that is already in the catalog; this is its counterpart.
//   node scripts/repair-company.mjs "<Name>" docs/company-proofs/<id>.json
import {readFile,writeFile}from "node:fs/promises";
const [name,proofPath]=process.argv.slice(2);if(!name||!proofPath)throw Error('Usage: repair-company.mjs "<Name>" <proof.json>');
const catalogUrl=new URL("../sidecar/company-catalog.json",import.meta.url),[catalog,proof]=await Promise.all([catalogUrl,proofPath].map(async source=>JSON.parse(await readFile(source,"utf8"))));
if(!proof.ok||proof.level!=="full"||proof.errors.length||!proof.listing.terminal?.payload?.complete)throw Error(`${proofPath} is not a passing, complete full proof`);
if(proof.source.name!==name)throw Error(`Proof is for "${proof.source.name}", not "${name}"`);
const company=catalog.companies.find(entry=>entry.name===name);if(!company)throw Error(`${name} is not in the catalog`);
const clash=catalog.companies.find(entry=>entry!==company&&entry.baseUrl===proof.source.baseUrl);if(clash)throw Error(`${clash.name} already uses ${proof.source.baseUrl}`);
const before=`${company.adapterId} ${company.baseUrl}`;
Object.assign(company,{adapterId:proof.source.adapterId,baseUrl:proof.source.baseUrl,config:proof.source.configJson,
 verified:{at:proof.finishedAt.slice(0,10),jobs:proof.listing.jobs,level:"full",evidence:proofPath.replaceAll("\\","/").replace(/^\.\//,"")}});
await writeFile(catalogUrl,JSON.stringify(catalog,null,1)+"\n");
console.log(`${name}: ${before} -> ${company.adapterId} ${company.baseUrl} (${proof.listing.jobs} jobs)`);
