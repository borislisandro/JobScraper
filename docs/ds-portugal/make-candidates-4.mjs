// Fourth batch of candidates (same shapes as make-candidates.mjs).
import {writeFileSync}from "node:fs";
const gh=(id,name,token)=>({id,name,baseUrl:`https://job-boards.greenhouse.io/${token}`,adapterId:"greenhouse",configJson:{token,maxPages:500,expectedHost:"boards-api.greenhouse.io"}});
const teamtailor=(id,name,host)=>({id,name,baseUrl:`https://${host}/jobs.json?per_page=100`,adapterId:"json",configJson:{itemsPath:"items",pageParam:"page",pageStart:1,maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"url",externalId:"id",postedAt:"date_published",description:"content_html",location:"_jobposting.jobLocation.0.address"}}});
const all=[teamtailor("axians","Axians Portugal","axiansgroup.teamtailor.com"),teamtailor("bearingpoint-portugal","BearingPoint Portugal","bearingpointportugal.teamtailor.com"),
 gh("gocardless","GoCardless","gocardless"),gh("indicium-ai","Indicium AI","indiciumai")];
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
