// Boards that moved: Wayve left Greenhouse for Ashby, Shield AI left Ashby for Lever.   node docs/semiconductor-eu/make-candidates-6.mjs
import {writeFileSync}from "node:fs";
const all=[{id:"wayve",name:"Wayve",baseUrl:"https://jobs.ashbyhq.com/wayve",adapterId:"ashby",configJson:{token:"wayve",maxPages:500,expectedHost:"api.ashbyhq.com"}},
 {id:"shield-ai",name:"Shield AI",baseUrl:"https://jobs.lever.co/shieldai",adapterId:"lever",configJson:{token:"shieldai",maxPages:500,expectedHost:"api.lever.co"}}];
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
