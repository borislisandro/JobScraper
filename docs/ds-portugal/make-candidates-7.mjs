// Seventh batch: sources that need detailSelector (descriptions live on each vacancy page).
import {writeFileSync}from "node:fs";
const all=[
 {id:"mobilab",name:"MobiLab Solutions",baseUrl:"https://mobilab-solutions-gmbh.jobs.personio.com/xml",adapterId:"static-xpath",configJson:{maxPages:1,expectedHost:"mobilab-solutions-gmbh.jobs.personio.com",
  itemXPath:"//position",titleXPath:"./name",locationXPath:"./office",urlXPath:"concat('https://mobilab-solutions-gmbh.jobs.personio.com/job/', ./id, '?language=en')",detailSelector:"main#main-content"}},
 {id:"smartex",name:"Smartex.ai",baseUrl:"https://odoo.smartex.ai/jobs",adapterId:"static-css",configJson:{itemSelector:"div.card",titleSelector:"h3",urlSelector:"a[href^=\"/jobs/\"]",locationSelector:"address",
  maxPages:1,expectedHost:"odoo.smartex.ai",detailSelector:"#wrap"}}];
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
