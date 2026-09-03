// What platform actually serves an employer's vacancies, answered from the live pages rather than
// from a guess. Written because the slow half of onboarding is discovery, and because a wrong
// answer here is expensive: a "talent community" board or a similarly-named tenant looks like a
// success until someone reads the rows.
//
//   node scripts/detect-board.mjs "ASM International" https://www.asm.com/careers
//   node scripts/detect-board.mjs --tracker asm-international besi teradyne
//
// It reports what it OBSERVED. It never writes a candidate: a token that answers is a lead, and a
// human still has to confirm the count matches what the employer publishes.
import {readFile}from "node:fs/promises";
const UA={"user-agent":"JobScraper/1.0 local personal use"};
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const text=async(url,init={})=>{const response=await fetch(url,{headers:UA,redirect:"follow",...init});
 return{status:response.status,url:response.url,body:await response.text()}};

// Signatures are ordered: the first that matches a page wins, so a Greenhouse board embedded in a
// corporate page is recognised before the corporate page's own host is reported as "in-house".
const SIGNATURES=[
 {adapter:"workday",re:/https?:\/\/([a-z0-9-]+)\.(wd\d+)\.myworkdayjobs\.com\/(?:[a-z-]+\/)?([A-Za-z0-9_-]+)/i,
  read:m=>({tenant:m[1],site:m[3],host:`${m[1]}.${m[2]}.myworkdayjobs.com`})},
 {adapter:"workday",re:/https?:\/\/(wd\d+)\.myworkdaysite\.com\/(?:[a-z-]+\/)?recruiting\/([a-z0-9-]+)\/([A-Za-z0-9_-]+)/i,
  read:m=>({tenant:m[2],site:m[3],host:`${m[1]}.myworkdaysite.com`})},
 {adapter:"greenhouse",re:/(?:job-)?boards(?:-api)?(?:\.eu)?\.greenhouse\.io\/(?:v1\/boards\/|embed\/job_board\?for=)?([a-z0-9_-]+)/i,
  read:m=>({token:m[1]})},
 {adapter:"ashby",re:/jobs\.ashbyhq\.com\/([a-z0-9._-]+)/i,read:m=>({token:m[1]})},
 {adapter:"lever",re:/jobs\.lever\.co\/([a-z0-9._-]+)/i,read:m=>({token:m[1]})},
 {adapter:"eightfold",re:/https?:\/\/([a-z0-9.-]+)\.eightfold\.ai/i,read:m=>({host:`${m[1]}.eightfold.ai`})},
 {adapter:"eightfold",re:/\/api\/pcsx\/(?:search|position_details)/i,read:()=>({})},
 {adapter:"oracle",re:/https?:\/\/([a-z0-9-]+\.fa\.[a-z0-9-]+\.oraclecloud\.com)/i,read:m=>({apiHost:m[1]})},
 {adapter:"icims",re:/https?:\/\/([a-z0-9-]+\.icims\.com)/i,read:m=>({host:m[1]})},
 // Platforms with no adapter yet. Naming them is the point: it says what to build next, and how
 // many queued employers a given build would unlock.
 {adapter:"?smartrecruiters",re:/smartrecruiters\.com\/([A-Za-z0-9_-]+)/i,read:m=>({token:m[1]})},
 {adapter:"?teamtailor",re:/([a-z0-9-]+)\.teamtailor\.com|teamtailor-cdn\.com/i,read:m=>({token:m[1]||null})},
 {adapter:"?successfactors",re:/([a-z0-9-]+\.successfactors\.(?:com|eu))/i,read:m=>({host:m[1]})},
 {adapter:"?avature",re:/avature\.net|templates-static-assets\.avacdn\.net/i,read:()=>({})},
 {adapter:"?workable",re:/([a-z0-9-]+)\.workable\.com/i,read:m=>({token:m[1]})},
 {adapter:"?recruitee",re:/([a-z0-9-]+)\.recruitee\.com/i,read:m=>({token:m[1]})},
 {adapter:"?personio",re:/([a-z0-9-]+)\.jobs\.personio\.(?:de|com)/i,read:m=>({token:m[1]})},
 {adapter:"?radancy",re:/tbcdn\.talentbrew\.com|search-results-list__list-item/i,read:()=>({})},
 {adapter:"?taleo",re:/([a-z0-9-]+\.taleo\.net)/i,read:m=>({host:m[1]})},
 {adapter:"?phenom",re:/phenompeople\.com|phApp\.ddo/i,read:()=>({})},
];

// A token that answers with real rows is worth far more than a token that merely appears in markup.
const HOSTED={
 greenhouse:async token=>{const r=await fetch(`https://boards-api.greenhouse.io/v1/boards/${token}/jobs`,{headers:UA});
  if(!r.ok)return null;const d=await r.json();return{count:(d.jobs||[]).length,sample:d.jobs?.[0]?.title}},
 ashby:async token=>{const r=await fetch(`https://api.ashbyhq.com/posting-api/job-board/${token}`,{headers:UA});
  if(!r.ok)return null;const d=await r.json();return{count:(d.jobs||[]).length,sample:d.jobs?.[0]?.title}},
 lever:async token=>{const r=await fetch(`https://api.lever.co/v0/postings/${token}?mode=json&limit=100`,{headers:UA});
  if(!r.ok)return null;const d=await r.json();return{count:Array.isArray(d)?d.length:0,sample:d?.[0]?.text}},
};
// Only the whole name, never its first word. "Blue Origin" guessed as "blue" matched a live Lever
// board belonging to someone else, and "Sierra Space" as "sierra" matched Sierra AI — both answered
// with real rows, so nothing downstream would have caught it. A generic first word is not a lead.
const slugs=name=>{const base=name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9 ]/g,"").trim();
 return [...new Set([base.replace(/ /g,""),base.replace(/ /g,"-")])].filter(s=>s.length>2)};

async function detect(name,urls){
 const found=new Map(),pages=[];
 for(const url of urls){
  let page;try{page=await text(url)}catch(error){pages.push(`${url} -> ${error.message}`);continue}
  pages.push(`${url} -> ${page.status} ${page.url}`);
  for(const signature of SIGNATURES){
   const match=signature.re.exec(page.body)||signature.re.exec(page.url);
   if(!match)continue;
   const key=`${signature.adapter}:${JSON.stringify(signature.read(match))}`;
   if(!found.has(key))found.set(key,{adapter:signature.adapter,...signature.read(match)});
  }
  await sleep(500);
 }
 // Nothing in the markup, or a hosted board whose token is worth confirming: ask the API itself.
 const guesses=[...found.values()].filter(f=>HOSTED[f.adapter]).map(f=>[f.adapter,f.token]);
 for(const family of Object.keys(HOSTED))for(const slug of slugs(name))guesses.push([family,slug]);
 const confirmed=[];
 for(const [family,token] of guesses){
  if(!token||confirmed.some(c=>c.adapter===family&&c.token===token))continue;
  let live;try{live=await HOSTED[family](token)}catch{live=null}
  await sleep(300);
  if(live&&live.count>0)confirmed.push({adapter:family,token,...live});
 }
 return{name,pages,signals:[...found.values()],confirmed};
}

const args=process.argv.slice(2);
let targets=[];
if(args[0]==="--tracker"){
 const tracker=JSON.parse(await readFile(new URL("../docs/company-support-tracker.json",import.meta.url),"utf8"));
 for(const id of args.slice(1)){
  const row=tracker.companies.find(company=>company.id===id);
  if(!row)throw Error(`Unknown tracker id: ${id}`);
  const site=row.officialCareersUrl||`https://www.${row.id.replace(/-/g,"")}.com/careers`;
  targets.push({name:row.name,urls:[site]});
 }
}else targets=[{name:args[0],urls:args.slice(1)}];

for(const target of targets){
 const result=await detect(target.name,target.urls);
 console.log(`\n### ${result.name}`);
 for(const page of result.pages)console.log(`  page  ${page}`);
 for(const signal of result.signals)console.log(`  seen  ${signal.adapter} ${JSON.stringify(signal)}`);
 for(const live of result.confirmed)console.log(`  LIVE  ${live.adapter} token=${live.token} jobs=${live.count} e.g. ${live.sample}`);
 if(!result.signals.length&&!result.confirmed.length)console.log("  (nothing recognised — read the page by hand)");
}
