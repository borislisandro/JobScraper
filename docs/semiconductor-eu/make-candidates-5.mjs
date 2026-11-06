// Tier 3 boards that need their own adapter (server-rendered HTML).   node docs/semiconductor-eu/make-candidates-5.mjs
import {writeFileSync}from "node:fs";
const company=(id,name,baseUrl,host)=>({id,name,baseUrl,adapterId:id,configJson:{maxPages:500,expectedHost:host}});
const all=[company("openchip","Openchip","https://openchip.factorialhr.com/","openchip.factorialhr.com"),company("racyics","Racyics","https://racyics.de/career/jobs/","racyics.de"),
 company("csem","CSEM","https://www.csem.ch/en/jobs/","www.csem.ch"),company("arteris","Arteris","https://jobs.jobvite.com/arteris/jobs?nl=1","jobs.jobvite.com")]; 
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
