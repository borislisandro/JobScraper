// Writes one worker-source candidate per entry below. Kept as data so a proof can be re-run after a board changes.
import {writeFileSync}from "node:fs";
const gh=(id,name,token,eu=false)=>({id,name,baseUrl:`https://job-boards.${eu?"eu.":""}greenhouse.io/${token}`,adapterId:"greenhouse",configJson:{token,maxPages:500,expectedHost:eu?"boards-api.eu.greenhouse.io":"boards-api.greenhouse.io"}});
const ashby=(id,name,token)=>({id,name,baseUrl:`https://jobs.ashbyhq.com/${encodeURIComponent(token)}`,adapterId:"ashby",configJson:{token,maxPages:500,expectedHost:"api.ashbyhq.com"}});
const lever=(id,name,token,eu=false)=>({id,name,baseUrl:`https://jobs.${eu?"eu.":""}lever.co/${token}`,adapterId:"lever",configJson:{token,maxPages:500,expectedHost:eu?"api.eu.lever.co":"api.lever.co"}});
const sr=(id,name,token)=>({id,name,baseUrl:`https://jobs.smartrecruiters.com/${token}`,adapterId:id,configJson:{maxPages:500,expectedHost:"jobs.smartrecruiters.com"}});
const recruitee=(id,name,host)=>({id,name,baseUrl:`https://${host}/api/offers/`,adapterId:"json",configJson:{itemsPath:"offers",maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"careers_url",externalId:"id",postedAt:"published_at",description:"description",location:"location"}}});
const teamtailor=(id,name,host)=>({id,name,baseUrl:`https://${host}/jobs.json?per_page=100`,adapterId:"json",configJson:{itemsPath:"items",pageParam:"page",pageStart:1,maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"url",externalId:"id",postedAt:"date_published",description:"content_html",location:"_jobposting.jobLocation.0.address"}}});
const all=[gh("datacamp","DataCamp","datacamp"),gh("pagerduty","PagerDuty","pagerduty"),
 ashby("airapps","Air Apps","airapps"),ashby("superhuman","Superhuman","superhuman platform inc"),ashby("thorit","Thorit","thorit"),
 lever("veeva","Veeva Systems","veeva"),lever("emma-sleep","Emma Sleep","emma-sleep"),lever("olx","OLX","olx",true),lever("vwgds","Volkswagen Digital Solutions","vwgds",true),
 sr("natixis-portugal","Natixis in Portugal","NatixisInPortugal"),sr("inetum","Inetum","Inetum2"),sr("act-digital","act digital","AlterSolutions"),sr("ilof","iLoF","ILoF-IntelligentLabOnFiber"),sr("dbservices-portugal","DBServices Portugal","DBServicesPortugal"),
 recruitee("mercedes-benz-io","Mercedes-Benz.io","mbio.recruitee.com"),recruitee("transperfect","TransPerfect (Unbabel)","transperfect.recruitee.com"),
 teamtailor("hiire","Hiire","hiire.teamtailor.com")];
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
