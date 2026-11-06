// Tier 3 boards that a configured generic JSON source already reads: Elmos (softgarden publishes a schema.org DataFeed)
// and Kalray (a WordPress custom post type over REST, with its office taxonomy embedded).   node docs/semiconductor-eu/make-candidates-4.mjs
import {writeFileSync}from "node:fs";
const all=[
 {id:"elmos",name:"Elmos Semiconductor",baseUrl:"https://elmos.career.softgarden.de/jobs.feed.json",adapterId:"json",configJson:{itemsPath:"dataFeedElement",maxPages:5,expectedHost:"elmos.career.softgarden.de",feedIsWholeBoard:true,
  fieldMap:{title:"item.title",url:"item.url",externalId:"item.identifier.value",postedAt:"item.datePosted",description:"item.description",location:"item.jobLocation.address.addressLocality"}}},
 {id:"kalray",name:"Kalray",baseUrl:"https://www.kalrayinc.com/wp-json/wp/v2/jobs?per_page=100&_embed=1",adapterId:"json",configJson:{itemsPath:"",maxPages:5,expectedHost:"www.kalrayinc.com",
  fieldMap:{title:"title.rendered",url:"link",externalId:"id",postedAt:"date",description:"content.rendered",location:"_embedded.wp:term.1.0.name"}}}];
for(const c of all)writeFileSync(new URL(`../company-candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:true,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
