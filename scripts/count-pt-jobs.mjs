// Counts the vacancies each catalog employer currently lists in Portugal, split into core data/AI
// roles (data science, ML/AI, data analysis, BI) and adjacent data engineering. It reads every
// board through the real worker, so the numbers are what the app would store.
//
//   node scripts/count-pt-jobs.mjs [--tag data-ai-pt] [--out docs/ds-portugal/counts.json]
import {readFile,writeFile}from "node:fs/promises";
import {runWorker}from "./worker-proof.mjs";
const args=process.argv.slice(2),option=(name,fallback)=>{const i=args.indexOf(`--${name}`);return i>=0?args[i+1]:fallback};
const tag=option("tag","data-ai-pt"),out=option("out","docs/ds-portugal/counts.json"),only=option("only","");
const catalog=JSON.parse(await readFile(new URL("../sidecar/company-catalog.json",import.meta.url),"utf8"));
const PT=/portugal|\bPT\b|lisbo|lisboa|porto(?! alegre)|oporto|braga|aveiro|coimbra|matosinhos|maia|guimar|leiria|faro|set[uú]bal|amadora|sintra|oeiras|cascais|gaia|[aá]gueda|ovar|[ií]lhavo/i;
const CORE=/\b(data scien\w*|machine learning|ml|mlops|ai|a\.i\.|artificial intelligence|gen ?ai|llm|nlp|computer vision|deep learning|data analy\w*|analytics|business intelligence|bi|statistic\w*)\b/i;
const ADJACENT=/\b(data engineer\w*|data architect\w*|data platform|big data|data governance|data steward\w*|dataops|databricks|etl)\b/i;
const NOISE=/\b(trainer|tester|annotat\w+|linguist|translator|interpreter|evaluator|raters?)\b/i;
const text=value=>typeof value==="string"?value:value&&typeof value==="object"?Object.values(value).flat(Infinity).map(text).join(" "):"";
const companies=catalog.companies.filter(c=>(c.tags||[]).includes(tag)&&(!only||c.name.toLowerCase().includes(only.toLowerCase())));
const rows=[],boards=new Map(),listing=[];
const area=job=>NOISE.test(job.title)?"other":CORE.test(job.title)?"data-ai":ADJACENT.test(job.title)?"data-engineering":"other";
for(const company of companies){
 const source={id:company.id,name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,kind:"active",robotsOverride:true,allowPrivateNetwork:false,configJson:company.config};
 const result=await runWorker(source,"scrape_source",{known:[],deferDetails:true});
 if(result.error||result.terminal?.event!=="completed"){rows.push({company:company.name,error:String(result.error||result.terminal?.payload?.message||"failed").slice(0,120)});console.error(`FAIL ${company.name}`);continue}
 // A job-board source is Portugal-only by construction (some rows give only a work mode, "Híbrido", as a place).
 const board=(company.tags||[]).includes("portugal-board");
 const inPortugal=result.jobs.filter(job=>PT.test(text(job.location))||(board&&!/,\s*[A-Z]{2}\s*$/.test(text(job.location))));
 const core=inPortugal.filter(job=>CORE.test(job.title)&&!NOISE.test(job.title)),adjacent=inPortugal.filter(job=>!core.includes(job)&&ADJACENT.test(job.title)&&!NOISE.test(job.title));
 for(const job of inPortugal)listing.push({company:board?job.company:company.name,via:board?company.name:"",area:area(job),title:job.title,location:text(job.location).slice(0,80),postedAt:job.postedAt||"",url:job.applyUrl||job.canonicalUrl});
 if(board){
  // Attribute each row to the employer that posted it, and merge the same employer across boards by link.
  for(const job of inPortugal){const name=`${job.company} (via ${company.name.split(" - ")[0]})`,key=name;
   const row=boards.get(key)||{company:name,boardRows:0,portugal:0,core:0,adjacent:0,titles:[],adjacentTitles:[],seen:new Set()};
   if(row.seen.has(job.applyUrl))continue;row.seen.add(job.applyUrl);row.portugal++;
   if(CORE.test(job.title)&&!NOISE.test(job.title)){row.core++;row.titles.push({title:job.title,location:text(job.location).slice(0,60),url:job.applyUrl})}
   else if(ADJACENT.test(job.title)&&!NOISE.test(job.title)){row.adjacent++;row.adjacentTitles.push(job.title)}
   boards.set(key,row)}
  console.error(`${company.name}: ${inPortugal.length} rows attributed to employers`);continue}
 rows.push({company:company.name,boardRows:result.jobs.length,portugal:inPortugal.length,core:core.length,adjacent:adjacent.length,
  titles:core.map(job=>({title:job.title,location:text(job.location).slice(0,60),url:job.applyUrl||job.canonicalUrl})),adjacentTitles:adjacent.map(job=>job.title)});
 console.error(`${company.name}: ${inPortugal.length} PT, ${core.length} core, ${adjacent.length} adjacent`);
}
for(const row of boards.values()){delete row.seen;rows.push(row)}
rows.sort((a,b)=>(b.core||0)-(a.core||0)||(b.portugal||0)-(a.portugal||0));
const seen=new Set(),unique=listing.filter(job=>!seen.has(job.url)&&seen.add(job.url));
const order={"data-ai":0,"data-engineering":1,other:2};unique.sort((a,b)=>order[a.area]-order[b.area]||a.company.localeCompare(b.company)||a.title.localeCompare(b.title));
const cell=value=>String(value??"").replace(/[\t\r\n]+/g," ");
await writeFile(new URL(out.replace(/\.json$/,"-jobs.tsv"),new URL("../",import.meta.url)),["area\tcompany\ttitle\tlocation\tposted\turl\tvia",...unique.map(j=>[j.area,j.company,j.title,j.location,j.postedAt,j.url,j.via].map(cell).join("\t"))].join("\n")+"\n");
await writeFile(new URL(`../${out}`,import.meta.url),JSON.stringify({countedAt:new Date().toISOString(),tag,rows},null,1)+"\n");
