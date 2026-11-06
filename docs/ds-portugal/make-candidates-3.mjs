// Third batch of candidates (same shapes as make-candidates.mjs).
import {writeFileSync}from "node:fs";
const wd=(id,name,host,tenant,site,query="Portugal")=>({id,name,baseUrl:`https://${host}.myworkdayjobs.com/`,adapterId:"workday",configJson:{maxPages:500,site,tenant,query,expectedHost:`${host}.myworkdayjobs.com`}});
const recruitee=(id,name,host)=>({id,name,baseUrl:`https://${host}/api/offers/`,adapterId:"json",configJson:{itemsPath:"offers",maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"careers_url",externalId:"id",postedAt:"published_at",description:"description",location:"location"}}});
const all=[recruitee("metyis","Metyis","metyisag.recruitee.com"),
 {id:"flix",name:"Flix",baseUrl:"https://job-boards.greenhouse.io/flix",adapterId:"greenhouse",configJson:{token:"flix",maxPages:500,expectedHost:"boards-api.greenhouse.io"}},
 {id:"finom",name:"Finom",baseUrl:"https://jobs.eu.lever.co/pnlfin",adapterId:"lever",configJson:{token:"pnlfin",maxPages:500,expectedHost:"api.eu.lever.co"}},
 wd("iqvia","IQVIA (Portugal)","iqvia.wd1","iqvia","IQVIA"),wd("airbus","Airbus (Portugal)","ag.wd3","ag","Airbus"),wd("accenture","Accenture (Portugal)","accenture.wd103","accenture","AccentureCareers")]; // Expleo (iCIMS, different markup) and Jobgether (3,935 worldwide rows) were tried and dropped; see status.tsv
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
