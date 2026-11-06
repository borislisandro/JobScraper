// Ninth batch: Landing.jobs role pages for Portugal. Only pages that list every result statically are used:
// the data-analyst and data-engineer pages hold 52 results but render 50, and have no static next link.
import {writeFileSync}from "node:fs";
const pages=[["data-scientist","Data"],["machine-learning","Machine Learning"],["artificial-intelligence","Artificial Intelligence"]];
const all=pages.map(([slug,label])=>({id:`landingjobs-${slug}`,name:`Landing.jobs - ${label}`,baseUrl:`https://landing.jobs/jobs/in/portugal/for/${slug}`,adapterId:"static-css",
 configJson:{itemSelector:"article.lj-jobcard-static",titleSelector:".lj-jobcard-static__link",urlSelector:".lj-jobcard-static__link",companySelector:".lj-jobcard-static__company",
  locationSelector:".lj-jobcard-static__location",maxPages:1,expectedHost:"landing.jobs",detailSelector:"#job-summary-content"}}));
for(const c of all)writeFileSync(new URL(`candidates/${c.id}.json`,import.meta.url),JSON.stringify({...c,kind:"active",robotsOverride:false,allowPrivateNetwork:false},null,1)+"\n");
console.log(all.map(c=>c.id).join(" "));
