// Eighth batch: ITJobs.pt role pages as sources. One ITJobs source covers every consultancy and recruiter that posts
// there but has no board of its own; the employer comes from each row, not from the source.
import {writeFileSync}from "node:fs";
// Only /emprego/ai is a plain list page. The other role slugs redirect to ?q= free-text searches, which robots.txt disallows, so they are not sources.
const roles=[["ai","AI"]];
const all=roles.map(([slug,label])=>({id:`itjobs-${slug}`,name:`ITJobs.pt - ${label}`,baseUrl:`https://www.itjobs.pt/emprego/${slug}`,adapterId:"static-css",
 configJson:{itemSelector:"ul.listing > li",titleSelector:"a.title",urlSelector:"a.title",companySelector:".list-name a",locationSelector:".list-details",maxPages:1,expectedHost:"www.itjobs.pt",detailSelector:".content-block"}}));
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:false,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
