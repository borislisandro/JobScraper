// Tier 2: employers on the generic Personio, Workable and BambooHR adapters.   node docs/semiconductor-eu/make-candidates-2.mjs
import {writeFileSync}from "node:fs";
const personio=(id,name,token,domain="de")=>({id,name,baseUrl:`https://${token}.jobs.personio.${domain}/`,adapterId:"personio",configJson:{token,...(domain==="com"?{domain}:{}),maxPages:5,expectedHost:`${token}.jobs.personio.${domain}`}});
const workable=(id,name,token)=>({id,name,baseUrl:`https://apply.workable.com/${token}/`,adapterId:"workable",configJson:{token,maxPages:5,expectedHost:"apply.workable.com"}});
const bamboohr=(id,name,token)=>({id,name,baseUrl:`https://${token}.bamboohr.com/careers`,adapterId:"bamboohr",configJson:{token,maxPages:5,expectedHost:`${token}.bamboohr.com`}});
const all=[personio("axiomise","Axiomise","axiomise"),personio("quantum-motion","Quantum Motion","quantummotion"),personio("semron","SEMRON","semron"),
 workable("raspberry-pi","Raspberry Pi","raspberrypi"),workable("universal-quantum","Universal Quantum","universalquantum"),bamboohr("codasip","Codasip","codasip")];
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
