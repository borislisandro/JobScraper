// Writes one worker-source candidate per Tier 1 employer (existing adapters, no new code).
// Kept as data so a proof can be re-run after a board changes.   node docs/semiconductor-eu/make-candidates.mjs
import {writeFileSync}from "node:fs";
const gh=(id,name,token,eu=false)=>({id,name,baseUrl:`https://job-boards.${eu?"eu.":""}greenhouse.io/${token}`,adapterId:"greenhouse",configJson:{token,maxPages:500,expectedHost:eu?"boards-api.eu.greenhouse.io":"boards-api.greenhouse.io"}});
const ashby=(id,name,token)=>({id,name,baseUrl:`https://jobs.ashbyhq.com/${encodeURIComponent(token)}`,adapterId:"ashby",configJson:{token,maxPages:500,expectedHost:"api.ashbyhq.com"}});
const teamtailor=(id,name,host)=>({id,name,baseUrl:`https://${host}/jobs.json?per_page=100`,adapterId:"json",configJson:{itemsPath:"items",pageParam:"page",pageStart:1,maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"url",externalId:"id",postedAt:"date_published",description:"content_html",location:"_jobposting.jobLocation.0.address"}}});
const workday=(id,name,tenant,site,wd)=>({id,name,baseUrl:`https://${tenant}.${wd}.myworkdayjobs.com/`,adapterId:"workday",configJson:{tenant,site,maxPages:500,expectedHost:`${tenant}.${wd}.myworkdayjobs.com`}});
const all=[teamtailor("semidynamics","Semidynamics","careers.semidynamics.com"),ashby("fractile","Fractile","fractile"),ashby("arago","Arago","arago"),
 gh("riverlane","Riverlane","riverlane"),gh("baya-systems","Baya Systems","bayasystems"),gh("ionq","IonQ","ionq"),ashby("olix","Olix","olix"),
 teamtailor("dolphin-semiconductor","Dolphin Semiconductor","careers.dolphin-semiconductor.com"),teamtailor("prophesee","Prophesee","careers.prophesee.ai"),
 teamtailor("pragmatic-semiconductor","Pragmatic Semiconductor","talent.pragmaticsemi.com"),
 workday("teledyne","Teledyne","flir","flircareers","wd1"),workday("philips","Philips","philips","jobs-and-careers","wd3")];
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
