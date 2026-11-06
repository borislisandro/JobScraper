// Tier 2b: SmartRecruiters tenants read through the company adapters' country filter.   node docs/semiconductor-eu/make-candidates-3.mjs
import {writeFileSync}from "node:fs";
const sr=(id,name,token)=>({id,name,baseUrl:`https://jobs.smartrecruiters.com/${token}`,adapterId:id,configJson:{maxPages:500,expectedHost:"jobs.smartrecruiters.com"}});
const all=[sr("cern","CERN","CERN"),sr("bosch","Bosch","BoschGroup")];
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
