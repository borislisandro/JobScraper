// Fifth batch of candidates: Breezy HR boards (root-array JSON feed at /json) and Workable accounts.
import {writeFileSync}from "node:fs";
const breezy=(id,name,slug)=>({id,name,baseUrl:`https://${slug}.breezy.hr/json?verbose=true`,adapterId:"json",configJson:{maxPages:1,expectedHost:`${slug}.breezy.hr`,
 fieldMap:{title:"name",url:"url",externalId:"id",postedAt:"published_date",description:"description",location:"location.name"}}});
// Workable's widget endpoint returns a board's whole job list, descriptions included, in one request.
const workable=(id,name,slug)=>({id,name,baseUrl:`https://apply.workable.com/api/v1/widget/accounts/${slug}?details=true`,adapterId:"json",configJson:{itemsPath:"jobs",maxPages:1,expectedHost:"apply.workable.com",
 fieldMap:{title:"title",url:"url",externalId:"shortcode",postedAt:"published_on",description:"description",location:"locations"}}});
const all=[breezy("critical-software","Critical Software","critical-software"),
 workable("usercentrics","Usercentrics","usercentrics"),workable("knok","knok","knok"),workable("greenvolt","Greenvolt","greenvolt"),workable("payabl","payabl.","payabl"),
 workable("joom","Joom","joom"),workable("gerodigital","GeroDigital","gerodigital"),workable("bnberry","Bnberry","bnberry"),workable("trade-nation","Trade Nation","trade-nation"),
 workable("deblock","Deblock","deblock"),workable("covergo","CoverGo","covergo")];
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
