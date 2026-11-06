// Probes the public job-board APIs of the hosted ATS vendors for an employer name and reports how
// many rows are located in Portugal. A token that merely answers proves nothing (a name collision
// answers with real rows); Portuguese locations on the board are the evidence that it is theirs.
//
//   node scripts/probe-ats-pt.mjs "Feedzai" [extraSlug ...]
//   node scripts/probe-ats-pt.mjs --file docs/ds-portugal/candidates.tsv
//
// Read-only and unauthenticated. It reports what it OBSERVED and never writes a candidate.
import {readFile}from "node:fs/promises";
const UA={"user-agent":"JobScraper/1.0 local personal use",accept:"application/json"};
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const PT=/portugal|porto\b|oporto|lisbo|lisboa|aveiro|braga|coimbra|maia|matosinhos|guimar|faro|\bpt\b/i;
const DS=/\b(data|machine learning|ml|ai|a\.i\.|analyst|analytics|scientist|nlp|llm|bi|mlops|genai)\b/i;
const get=async(url,asText=false)=>{const response=await fetch(url,{headers:UA,redirect:"follow",signal:AbortSignal.timeout(20_000)});
 if(!response.ok)return null;return asText?response.text():response.json()};

// Each vendor yields {title, where} rows from its public listing endpoint.
const VENDORS={
 greenhouse:async t=>{const d=await get(`https://boards-api.greenhouse.io/v1/boards/${t}/jobs`);
  return d?.jobs?.map(j=>({title:j.title,where:j.location?.name||""}))},
 "greenhouse-eu":async t=>{const d=await get(`https://boards-api.eu.greenhouse.io/v1/boards/${t}/jobs`);
  return d?.jobs?.map(j=>({title:j.title,where:j.location?.name||""}))},
 ashby:async t=>{const d=await get(`https://api.ashbyhq.com/posting-api/job-board/${t}`);
  return d?.jobs?.map(j=>({title:j.title,where:[j.location,...(j.secondaryLocations||[]).map(l=>l.location),j.address?.postalAddress?.addressCountry].filter(Boolean).join("; ")}))},
 lever:async t=>{const d=await get(`https://api.lever.co/v0/postings/${t}?mode=json&limit=500`);
  return Array.isArray(d)?d.map(j=>({title:j.text,where:[j.categories?.location,...(j.categories?.allLocations||[]),j.country].filter(Boolean).join("; ")})):null},
 "lever-eu":async t=>{const d=await get(`https://api.eu.lever.co/v0/postings/${t}?mode=json&limit=500`);
  return Array.isArray(d)?d.map(j=>({title:j.text,where:[j.categories?.location,...(j.categories?.allLocations||[]),j.country].filter(Boolean).join("; ")})):null},
 smartrecruiters:async t=>{const d=await get(`https://api.smartrecruiters.com/v1/companies/${t}/postings?limit=100&country=pt`);
  return d?.totalFound>0?d.content.map(j=>({title:j.name,where:[j.location?.city,j.location?.country,j.location?.remote&&"remote"].filter(Boolean).join(", ")})):null},
 workable:async t=>{const d=await get(`https://apply.workable.com/api/v1/widget/accounts/${t}`);
  return d?.jobs?.length?d.jobs.map(j=>({title:j.title,where:[j.city,j.state,j.country].filter(Boolean).join(", ")})):null},
 recruitee:async t=>{const d=await get(`https://${t}.recruitee.com/api/offers/`);
  return d?.offers?.length?d.offers.map(j=>({title:j.title,where:[j.city,j.country,j.location].filter(Boolean).join(", ")})):null},
 breezy:async t=>{const d=await get(`https://${t}.breezy.hr/json`);
  return Array.isArray(d)&&d.length?d.map(j=>({title:j.name,where:[...(j.locations||[j.location]).map(l=>l?.name),...(j.locations||[j.location]).map(l=>l?.country?.name)].filter(Boolean).join("; ")})):null},
 pinpoint:async t=>{const d=await get(`https://${t}.pinpointhq.com/postings.json`);
  return d?.data?.length?d.data.map(j=>({title:j.title,where:[j.location?.name,j.location?.city,j.location?.province].filter(Boolean).join(", ")})):null},
 personio:async t=>{const x=await get(`https://${t}.jobs.personio.de/xml`,true)||await get(`https://${t}.jobs.personio.com/xml`,true);
  if(!x||!x.includes("<position"))return null;
  return [...x.matchAll(/<position>[\s\S]*?<\/position>/g)].map(m=>({title:/<name>(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?<\/name>/.exec(m[0])?.[1]||"",where:/<office>(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?<\/office>/.exec(m[0])?.[1]||""}))},
 teamtailor:async t=>{const x=await get(`https://${t}.teamtailor.com/jobs.rss`,true);
  if(!x||!x.includes("<item>"))return null;
  return [...x.matchAll(/<item>[\s\S]*?<\/item>/g)].map(m=>({title:/<title>(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?<\/title>/.exec(m[0])?.[1]||"",where:[...m[0].matchAll(/<tt:(?:city|country|name)>(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?</g)].map(x=>x[1]).join(", ")}))},
};

// PROBE_SKIP=workable,breezy leaves vendors out, e.g. while one of them is rate-limiting this machine.
const SKIP=new Set((process.env.PROBE_SKIP||"").split(",").filter(Boolean));
for(const vendor of SKIP)delete VENDORS[vendor];
const slugs=(name,extra)=>{const base=name.toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g,"").replace(/[^a-z0-9 ]/g,"").trim();
 const parts=base.split(" ");
 return [...new Set([...extra,base.replace(/ /g,""),base.replace(/ /g,"-"),parts[0]===base?null:`${base.replace(/ /g,"")}portugal`,
  `${base.replace(/ /g,"")}pt`,`${base.replace(/ /g,"")}-careers`].filter(s=>s&&s.length>2))]};

export async function probe(name,extra=[]){
 const hits=[];
 for(const slug of slugs(name,extra))for(const [vendor,read]of Object.entries(VENDORS)){
  let rows;try{rows=await read(slug)}catch{rows=null}
  await sleep(120);
  if(!rows?.length)continue;
  const pt=rows.filter(r=>PT.test(r.where)),ds=pt.filter(r=>DS.test(r.title));
  hits.push({vendor,slug,total:rows.length,pt:pt.length,ptDs:ds.length,sample:(ds.length?ds:pt).slice(0,3).map(r=>`${r.title} [${r.where}]`)});
 }
 return hits;
}

if(import.meta.url===`file://${process.argv[1]}`){
 const args=process.argv.slice(2);let targets=[];
 if(args[0]==="--file"){
  const lines=(await readFile(args[1],"utf8")).split("\n").filter(line=>line&&!line.startsWith("#"));
  targets=lines.map(line=>{const [name,,,,slugCol]=line.split("|");return{name,extra:(slugCol||"").split(",").map(s=>s.trim()).filter(Boolean)}});
 }else if(args[0]==="--boards"){
  // vendor:token pairs, e.g. greenhouse:tripadvisor lever:olx. SmartRecruiters rows are already Portugal-only.
  for(const pair of args.slice(1)){const [vendor,token]=pair.split(":");let rows;try{rows=await VENDORS[vendor](token)}catch{rows=null}
   if(!rows?.length){console.log(`\n### ${pair}\n  (empty or no such board)`);continue}
   const pt=rows.filter(r=>PT.test(r.where)||vendor==="smartrecruiters"),ds=pt.filter(r=>DS.test(r.title));
   console.log(`\n### ${pair}\n  total=${rows.length} PT=${pt.length} PT+data/AI=${ds.length}${(ds.length?ds:pt).slice(0,3).map(r=>`\n     e.g. ${r.title} [${r.where}]`).join("")}`);
   await sleep(200)}
  process.exit(0);
 }else targets=[{name:args[0],extra:args.slice(1)}];
 for(const target of targets){
  const hits=await probe(target.name,target.extra);
  console.log(`\n### ${target.name}`);
  if(!hits.length)console.log("  (no hosted board answered)");
  for(const hit of hits)console.log(`  ${hit.vendor} token=${hit.slug} total=${hit.total} PT=${hit.pt} PT+data/AI=${hit.ptDs}${hit.sample.length?`\n     e.g. ${hit.sample.join("\n          ")}`:""}`);
 }
}
