// Sixth batch of candidates (same shapes as make-candidates.mjs): SIXT, Zego, Loka, Intermedia (Pinpoint), IG&H.
import {writeFileSync}from "node:fs";
const recruitee=(id,name,host)=>({id,name,baseUrl:`https://${host}/api/offers/`,adapterId:"json",configJson:{itemsPath:"offers",maxPages:500,expectedHost:host,
 fieldMap:{title:"title",url:"careers_url",externalId:"id",postedAt:"published_at",description:"description",location:"location"}}});
const all=[{id:"sixt",name:"SIXT (Portugal)",baseUrl:"https://jobs.smartrecruiters.com/SIXT",adapterId:"sixt",configJson:{maxPages:500,expectedHost:"jobs.smartrecruiters.com"}},
 {id:"zego",name:"Zego",baseUrl:"https://jobs.ashbyhq.com/zego",adapterId:"ashby",configJson:{token:"zego",maxPages:500,expectedHost:"api.ashbyhq.com"}},
 {id:"loka",name:"Loka",baseUrl:"https://job-boards.greenhouse.io/lokainc",adapterId:"greenhouse",configJson:{token:"lokainc",maxPages:500,expectedHost:"boards-api.greenhouse.io"}},
 {id:"intermedia",name:"Intermedia",baseUrl:"https://intermedia.pinpointhq.com/postings.json",adapterId:"json",configJson:{itemsPath:"data",maxPages:1,expectedHost:"intermedia.pinpointhq.com",
  fieldMap:{title:"title",url:"url",externalId:"id",description:"description",location:"location"}}},
 recruitee("igh","IG&H Portugal","igh.recruitee.com")];
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
