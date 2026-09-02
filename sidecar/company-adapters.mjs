// Company-specific boards that no generic ATS adapter reads. Each one is the live-verified
// contract for that employer, ported from the standalone scraper: a generic "static-css" or
// "icims" guess against these hosts returns navigation links or 404s, not vacancies.
let cheerio,cheerioPromise;
const loadCheerio=()=>cheerioPromise??=import("cheerio").then(module=>cheerio=module);

export const companyAdapters={
 // Arm's search results carry no posting date at all; the vacancy page does ("Date posted
 // Jun. 09, 2026"), so for this board the detail fetch is what a date costs and skipping it
 // leaves every Arm listing undated.
 arm:{name:"Arm",host:"careers.arm.com",base:"https://careers.arm.com",listing:"https://careers.arm.com/search-jobs",datesOnDetailOnly:true},
 amd:{name:"AMD",host:"careers.amd.com",base:"https://careers.amd.com"},
 mediatek:{name:"MediaTek",host:"careers.mediatek.com",base:"https://careers.mediatek.com"},
 google:{name:"Google",host:"www.google.com",base:"https://www.google.com/about/careers/applications/",listing:"https://www.google.com/about/careers/applications/jobs/results/"},
 cisco:{name:"Cisco",host:"careers.cisco.com",base:"https://careers.cisco.com",listing:"https://careers.cisco.com/global/en/search-results"},
 "sk-hynix":{name:"SK hynix",host:"talent.skhynix.com",base:"https://talent.skhynix.com"},
 // u-blox renders its openings from an Algolia index, not from the page HTML: the job-openings
 // page ships an empty widget and fills it client-side. These are the same public search-only
 // credentials that page hands the browser, read from its own bundle, and the index name is the
 // one its "Job Openings" widget is configured with. Vacancy detail lives on Salesforce Recruit.
 "u-blox":{name:"u-blox",host:"www.u-blox.com",base:"https://www.u-blox.com",
  algolia:{appId:"L87M8KNOMJ",apiKey:"b5a0aca646f3378b36d8758634d16954",index:"open_positions_en-US"}}
};
export const companyAdapterIds=Object.keys(companyAdapters);
// Boards whose listing pages publish no posting date. A scrape defers detail pages for speed,
// which for these means storing every job undated — so they opt out of the deferral.
export const companyDatesNeedDetail=id=>Boolean(companyAdapters[id]?.datesOnDetailOnly);

const array=(v,label)=>{if(!Array.isArray(v))throw Error(`parse_error: Structure changed: ${label} is not an array`);return v};
const clean=v=>v==null?null:String(v).replace(/\u00a0/g," ").replace(/\s+/g," ").trim()||null;
const absolute=(v,base)=>{if(!v)return null;try{return new URL(v,base).toString()}catch{return null}};
const slug=v=>clean(v)?.toLowerCase().normalize("NFKD").replace(/[̀-ͯ]/g,"").replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"")||"job";
// MediaTek ships label/code the wrong way round on locations ({label:"0000009255",code:"Taipei"}),
// so pick whichever side is not an internal number rather than trusting the field name.
const readable=(...values)=>values.map(clean).find(v=>v&&!/^\d+$/.test(v))||null;
const json=async(request,url,options={})=>{const r=await request(url,options);try{return JSON.parse(r.text)}catch(e){throw Error(`parse_error: Invalid JSON from ${url}: ${e.message}`,{cause:e})}};
const markup=async(request,url,options={})=>(await request(url,options)).text;
const robotsDenied=e=>/^robots_denied/.test(String(e?.message||e));

function armPage(html,site){
 const $=cheerio.load(html);
 const jobs=$("li.job-card").filter((_,el)=>{const card=$(el);return Boolean(card.attr("data-job-id")||card.find("a.job-card__title").first().attr("href")?.startsWith("/job/"))})
  .map((_,el)=>{const card=$(el),link=card.find("a.job-card__title").first();return{
   title:clean(link.text()),location:clean(card.find(".location").first().text()),
   url:absolute(link.attr("href"),site.base),externalId:clean(card.attr("data-job-id")||link.attr("data-job-id")),
   postedAt:card.find("time").attr("datetime")||clean(card.find(".date, [class*=date]").first().text())||card.text().match(/(?:posted\s+)?(?:today|yesterday|\d+\+?\s+days?\s+ago)/i)?.[0],
   description:clean(card.find(".category").first().text())}}).get();
 // Math.max(1,...[]) is 1, so a page selector that stopped matching used to read as a board
 // exactly one page long. Not knowing how many pages there are is reported as not knowing.
 const pager=$("nav[data-pagination-for='search-results-jobs'] #pagination-current-bottom, #pagination-current-bottom").first();
 const declaredMax=Number(pager.attr("max"));
 const pages=pager.find("option").map((_,el)=>Number($(el).attr("value")||$(el).text())).get().filter(Number.isFinite);
 return{jobs,maxPage:Number.isFinite(declaredMax)&&declaredMax>0?declaredMax:pages.length?Math.max(...pages):null}}

function armDetail(html,current){
 const $=cheerio.load(html),withoutLabel=(selector,label)=>clean($(selector).first().text()?.replace(new RegExp(`^${label}\\s*`,"i"),""));
 return{...current,externalId:withoutLabel(".job-id","Job ID")||current.externalId,location:withoutLabel(".job-location","Location")||current.location,
  postedAt:withoutLabel(".job-date","Date posted")||current.postedAt,descriptionHtml:$(".ats-description").first().html()||undefined,
  description:clean($(".ats-description").first().text())||current.description}}

async function arm(site,{request,maxPages,known,hashListing,acceptTitle=()=>true,fetchDetail=true}){
 const jobs=[];let page=1,maxPage=Infinity;
 while(page<=maxPage&&page<=maxPages){
  const parsed=armPage(await markup(request,`${site.listing}?p=${page}`),site);
  jobs.push(...parsed.jobs);if(parsed.maxPage!==null)maxPage=parsed.maxPage;
  if(!parsed.jobs.length)break;page+=1}
 // Detail pages carry the requisition ID, the full location and the description; a listing-only
 // record is still a valid vacancy, so a failed detail fetch degrades instead of failing the run.
 // A listing row identical to the one already stored skips its detail page altogether.
  const enriched=[],seen=[];let failures=0,filtered=0;
  for(const item of jobs){
   if(!item.url){enriched.push(item);continue}
   const hash=hashListing(item.url,item.title,item.location);
   if(!acceptTitle(item.title)){filtered+=1;if(known.has(hash))seen.push(hash);continue}
   if(known.has(hash)){seen.push(hash);continue}
  // The change check only asks what is on page one; enriching those rows would cost a request
  // each for information it is not going to store.
  if(!fetchDetail){enriched.push({...item,listingHash:hash,detailUrl:item.url,descriptionStatus:"pending"});continue}
  try{enriched.push({...armDetail(await markup(request,item.url,{metricKind:"detail"}),item),listingHash:hash,detailUrl:item.url,descriptionStatus:"complete"})}
  catch{failures+=1;enriched.push({...item,listingHash:hash,detailUrl:item.url,descriptionStatus:"failed",descriptionError:"Arm detail page could not be read."})}}
  // Complete means the last page the board itself named was read. Running out of listings is a
  // reason to stop, not proof of the end: a page that fails to render is indistinguishable from
  // one past the end, and treating it as the end reconciles every unread opening to closed.
  const extentKnown=Number.isFinite(maxPage),complete=extentKnown&&page>maxPage;
  return{jobs:enriched,seen,filtered,pages:Math.min(page-1,maxPage),complete,total:jobs.length,totalExact:complete,
  warnings:[...(failures?[`${failures} Arm detail pages could not be read; their listing data was kept.`]:[]),
   ...(extentKnown?[]:["Arm published no page selector, so how much of the board was read is unknown. This run is treated as unfinished and nothing was marked closed."])]}}

// One Algolia query per page of vacancies. The index carries no posting date, so these rows
// keep none rather than inventing one, and the apply link doubles as the listing's own key.
const uba = site => `https://${site.algolia.appId.toLowerCase()}-dsn.algolia.net/1/indexes/*/queries?x-algolia-application-id=${site.algolia.appId}&x-algolia-api-key=${site.algolia.apiKey}`;
export const ubloxVacancyNo = url => {
 try{return new URL(String(url)).searchParams.get("vacancyNo")||null}catch{return null}};
function ubloxHits(payload,site){
 const result=payload?.results?.[0];
 if(!result||!Array.isArray(result.hits))throw Error("parse_error: Structure changed: u-blox Algolia response has no hits");
 const jobs=result.hits.map(hit=>{const url=clean(hit.link_https||hit.link);return{
  title:clean(hit.vacancy_name||hit.name),
  // locations_legacy names the office, also_available_in the other sites the role can sit at.
  location:[...new Set([...(hit.locations_legacy||[]),...(hit.also_available_in||[])].map(clean).filter(Boolean))].join("; ")||null,
  url:url?absolute(url,site.base):null,externalId:ubloxVacancyNo(url)||clean(hit.id||hit.objectID),
  postedAt:null,description:clean(hit.department),workMode:clean(hit.percentage)}});
 return{jobs,nbPages:Number(result.nbPages),total:Number(result.nbHits),hitsPerPage:Number(result.hitsPerPage)}}
function ubloxDetail(html,current){
 const $=cheerio.load(html);
 // Salesforce Recruit puts every rich-text field in .sfdc_richtext: the role, then the company
 // blurb. Both belong to the description; the field table above them is already in the listing.
 const blocks=$(".sfdc_richtext").map((_,el)=>$(el)).get();
 const text=blocks.map(block=>clean(block.text())).filter(Boolean).join("\n\n");
 return{...current,description:text||current.description,descriptionHtml:blocks.map(block=>block.html()||"").join("")||undefined}}
async function ublox(site,{request,maxPages,known,hashListing,acceptTitle=()=>true,fetchDetail=true}){
 const jobs=[];let page=0,nbPages=Infinity,total=null,exhausted=false;
 while(page<nbPages&&page<maxPages){
  const parsed=ubloxHits(await json(request,uba(site),{method:"POST",headers:{accept:"application/json","content-type":"application/json"},
   body:{requests:[{indexName:site.algolia.index,params:`hitsPerPage=100&page=${page}&query=&attributesToHighlight=`}]}}),site);
  // Defaulting to page+1 made an unreadable page count read as "this was the last page".
  jobs.push(...parsed.jobs);if(Number.isFinite(parsed.nbPages))nbPages=parsed.nbPages;
  if(total===null)total=Number.isFinite(parsed.total)?parsed.total:null;
  // Algolia's other end-of-index signal, and the reason a missing nbPages does not mean paging
  // blindly to the cap: a page holding fewer hits than were asked for is the last page.
  const short=parsed.jobs.length<parsed.hitsPerPage;
  page+=1;
  if(!parsed.jobs.length||short){exhausted=true;break}}
 // Same shape as Arm: the vacancy page is the only place a description exists, a listing already
 // stored skips it, and a page that cannot be read degrades to listing data instead of failing.
 const enriched=[],seen=[];let failures=0,filtered=0;
 for(const item of jobs){
  if(!item.url){enriched.push(item);continue}
  const hash=hashListing(item.url,item.title,item.location);
  if(!acceptTitle(item.title)){filtered+=1;if(known.has(hash))seen.push(hash);continue}
  if(known.has(hash)){seen.push(hash);continue}
  if(!fetchDetail){enriched.push({...item,listingHash:hash,detailUrl:item.url,descriptionStatus:"pending"});continue}
  try{enriched.push({...ubloxDetail(await markup(request,item.url,{metricKind:"detail"}),item),listingHash:hash,detailUrl:item.url,descriptionStatus:"complete"})}
  catch{failures+=1;enriched.push({...item,listingHash:hash,detailUrl:item.url,descriptionStatus:"failed",descriptionError:"u-blox vacancy page could not be read."})}}
 const extentKnown=Number.isFinite(nbPages)||exhausted,complete=extentKnown&&(exhausted||page>=nbPages);
 return{jobs:enriched,seen,filtered,pages:page,complete,total,totalExact:total!==null,
  warnings:[...(failures?[`${failures} u-blox vacancy pages could not be read; their listing data was kept.`]:[]),
   ...(extentKnown?[]:["u-blox reported no page count, so how much of the board was read is unknown. This run is treated as unfinished and nothing was marked closed."])]}}

async function amd(site,{request,maxPages}){
 const jobs=[];let page=1,total=Infinity;
 while(jobs.length<total&&page<=maxPages){
  const payload=await json(request,`${site.base}/api/jobs?page=${page}&limit=100`,{headers:{accept:"application/json"}});
  total=Number(payload?.totalCount);
  if(!Number.isFinite(total))throw Error("parse_error: Structure changed: AMD totalCount is missing");
  const pageJobs=array(payload?.jobs,"jobs").map(wrapped=>{const item=wrapped.data||wrapped;return{
   title:clean(item.title),location:clean(item.full_location||item.short_location||[item.city,item.state,item.country].filter(Boolean).join(", ")),
   url:absolute(`/careers-home/jobs/${item.slug}`,site.base),externalId:clean(item.req_id||item.slug),
   postedAt:item.posted_date,description:item.description,descriptionHtml:item.description}});
  jobs.push(...pageJobs);if(!pageJobs.length)break;page+=1}
  return{jobs,pages:page-1,complete:jobs.length>=total,total,totalExact:true,warnings:[]}}

async function mediatek(site,{request,maxPages}){
 const jobs=[];let page=1,totalPages=Infinity,itemTotal=null;
 while(page<=totalPages&&page<=maxPages){
  const input={json:{locales:"en_US",page,jobQueryInfo:{},filters:{categorys:[],workExperiences:[],locations:[],programs:[]},sortBy:"publishedDate",order:"DESC",limit:100}};
  // Without NEXT_LOCALE the Next.js app answers the API call with a locale redirect loop.
  const payload=await json(request,`${site.base}/api/trpc/job.getJobs?input=${encodeURIComponent(JSON.stringify(input))}`,{headers:{accept:"application/json",cookie:"NEXT_LOCALE=en"}});
  const data=payload?.result?.data?.json;
  totalPages=Number(data?.pagination?.total_pages);if(itemTotal===null)itemTotal=Number(data?.pagination?.total_items)||null;
  if(!Number.isFinite(totalPages))throw Error("parse_error: Structure changed: MediaTek pagination is missing");
  const pageJobs=array(data?.jobs,"result.data.json.jobs").map(item=>({
   title:clean(item.title),location:readable(item.location,item.properties?.location?.code,item.properties?.location?.label),
   url:absolute(`/en/jobs/${item.id}`,site.base),externalId:clean(item.id||item.applicationId),
   postedAt:item.publishedDate||item.postedDate||item.datePosted,description:item.summary||item.description}));
  jobs.push(...pageJobs);if(!pageJobs.length)break;page+=1}
  return{jobs,pages:page-1,complete:page>totalPages,total:itemTotal,totalExact:itemTotal!==null,warnings:[]}}

function googlePage(html,site){
 const $=cheerio.load(html);
 const jobs=$("li.lLd3Je").map((_,el)=>{const card=$(el),href=card.find("a").first().attr("href");return{
  title:clean(card.find("h3.QJPWVe, h3").first().text()),
  // Google renders extra locations as sibling spans that already carry their "; " separator,
  // and repeats the whole set for the collapsed and expanded layouts.
  location:[...new Set(card.find(".r0wTof").map((__,node)=>clean($(node).text())?.replace(/^;\s*/,"")).get().filter(Boolean))].join("; ")||null,
  // Detail hrefs carry the whole search query back as tracking noise; the path alone is canonical.
  url:absolute(href?.split("?")[0],site.base),externalId:href?.match(/results\/(\d+)/)?.[1]||null,
  postedAt:card.find("[datetime]").first().attr("datetime")||card.text().match(/(?:posted\s+)?(?:today|yesterday|\d+\+?\s+days?\s+ago)/i)?.[0],
  description:clean(card.find(".Xsxa1e, .BVG0Nb, .WpHeLc").first().text())}}).get();
 const total=Number(clean($(".SWhIm").first().text())?.replace(/,/g,""));
 return{jobs,total:Number.isFinite(total)?total:null}}

async function google(site,{request,maxPages}){
 const jobs=[];const warnings=[];let page=1,total=Infinity;
 while(jobs.length<total&&page<=maxPages){
  let html;
  // google.com/robots.txt disallows "…/jobs/results/?page=" for every crawler, so only the
  // unpaged first page is readable under policy. Report that instead of stopping silently.
  try{html=await markup(request,page===1?site.listing:`${site.listing}?page=${page}`)}
  catch(e){if(page>1&&robotsDenied(e)){warnings.push("Google's robots.txt disallows paged careers results, so only the first page was read. Enable this source's robots override to read the rest.");return{jobs,pages:page-1,complete:false,warnings}}throw e}
  const parsed=googlePage(html,site);
  if(parsed.total!==null)total=parsed.total;
  jobs.push(...parsed.jobs);if(!parsed.jobs.length)break;page+=1}
  return{jobs,pages:page-1,complete:jobs.length>=total,total:Number.isFinite(total)?total:null,totalExact:Number.isFinite(total),warnings}}

function ciscoPage(html,site){
 // Cisco's Phenom tenant renders the result list client-side but ships the same payload
 // inline as phApp.ddo, which is the only readable listing index on this host.
 const startMarker="phApp.ddo = ",endMarker="; phApp.experimentData";
 const start=html.indexOf(startMarker),end=html.indexOf(endMarker,start+startMarker.length);
 if(start<0||end<0)throw Error("parse_error: Structure changed: Cisco embedded search data is missing");
 let data;try{data=JSON.parse(html.slice(start+startMarker.length,end))?.eagerLoadRefineSearch}
 catch(e){throw Error(`parse_error: Structure changed: Cisco embedded search data is unreadable: ${e.message}`,{cause:e})}
 const jobs=array(data?.data?.jobs,"eagerLoadRefineSearch.data.jobs").map(item=>({
  title:clean(item.title),
  location:(item.multi_location_array?.length?item.multi_location_array.map(x=>x?.location||x?.city||x?.name):item.multi_location?.length?item.multi_location:[item.location||item.cityStateCountry])
   .map(clean).filter(Boolean).filter((v,i,a)=>a.indexOf(v)===i).join("; ")||null,
  url:absolute(`/global/en/job/${item.jobSeqNo||item.jobId}/${slug(item.title)}`,site.base),
  externalId:clean(item.reqId||item.jobId||item.jobSeqNo),postedAt:item.postedDate||item.posted_date||item.datePosted,
  description:item.descriptionTeaser}));
 return{jobs,total:Number(data?.totalHits)}}

async function cisco(site,{request,maxPages}){
 const jobs=[];let offset=0,page=0,total=Infinity;
 while(offset<total&&page<maxPages){
  const parsed=ciscoPage(await markup(request,`${site.listing}?from=${offset}&s=1`),site);
  if(!Number.isFinite(parsed.total))throw Error("parse_error: Structure changed: Cisco totalHits is missing");
  total=parsed.total;jobs.push(...parsed.jobs);if(!parsed.jobs.length)break;
  offset+=parsed.jobs.length;page+=1}
  return{jobs,pages:page,complete:offset>=total,total,totalExact:true,warnings:[]}}

// SK hynix publishes Korean/global openings on the shared SK Careers board (filtered to the
// SK hynix legal entity) and its U.S. openings on a separate Greenhouse board. Neither is a
// superset of the other, so both are read.
async function skHynix(site,{request}){
 const [globalPayload,usPayload]=await Promise.all([
  json(request,"https://www.skcareers.com/Recruit/GetRecruitList",{method:"POST",
   headers:{accept:"application/json","content-type":"application/x-www-form-urlencoded; charset=UTF-8","x-requested-with":"XMLHttpRequest"},
   rawBody:new URLSearchParams({sort:"1",searchText:"",corpCode:"",jobRole:"",recruitType:"",workingType:"",workingRegion:""}).toString()}),
  json(request,"https://boards-api.greenhouse.io/v1/boards/skhynixamerica/jobs?content=true",{headers:{accept:"application/json"}})]);
 const global=array(globalPayload?.list,"list").filter(item=>/^SK hynix$/i.test(String(item.corpName||""))).map(item=>({
  title:clean(item.title),location:clean(item.workingArea),url:`https://www.skcareers.com/Recruit/Detail/${item.noticeID}`,
  externalId:clean(item.noticeID||item.jobNoticeNo),postedAt:clean(item.start?.replace(/\([^)]*\)/g,"")),
  description:[item.jobRole,item.recruitType,item.workingType].filter(Boolean).join("; ")}));
 // Greenhouse returns the description as entity-escaped HTML, so it is decoded here; the shared
 // normalizer strips tags once and would otherwise leave the escaped markup in the stored text.
 const html=v=>v?cheerio.load(`<div>${v}</div>`).root().text():null;
 const us=array(usPayload?.jobs,"jobs").map(item=>({
  title:clean(item.title),location:clean(item.location?.name),url:item.absolute_url,
  externalId:clean(item.requisition_id||item.id),postedAt:item.first_published||item.updated_at,
  description:html(item.content),descriptionHtml:html(item.content)}));
  return{jobs:[...global,...us],pages:1,complete:true,total:global.length+us.length,totalExact:true,
  warnings:["Combined the global SK Careers board with the SK hynix America Greenhouse board."]}}

const scrapers={arm,amd,mediatek,google,cisco,"sk-hynix":skHynix,"u-blox":ublox};
export const companyNeedsCheerio=(adapterId,fetchDetail=true)=>
 ["arm","google","sk-hynix"].includes(adapterId)||(adapterId==="u-blox"&&fetchDetail);

export async function enrichCompany(adapterId,item,request){
 const enrichers={arm:armDetail,"u-blox":ubloxDetail};
 const detail=enrichers[adapterId];
 if(!detail)throw Error(`incomplete: ${adapterId} has no detail enricher`);
 await loadCheerio();
 return{...detail(await markup(request,item.detailUrl||item.url,{metricKind:"detail"}),item),detailUrl:item.detailUrl||item.url,descriptionStatus:"complete"}
}

export async function scrapeCompany(adapterId,context){
 const site=companyAdapters[adapterId];
 if(!site)throw Error(`incomplete: unsupported company adapter ${adapterId}`);
 if(companyNeedsCheerio(adapterId,context.fetchDetail!==false))await loadCheerio();
 // known/hashListing let an adapter recognise a listing it already stored; adapters that read
 // everything from the listing page have no detail fetch to skip and simply ignore them.
 const hashListing=context.hashListing||(()=>"");
 const result=await scrapers[adapterId](site,{known:new Set(),hashListing,...context,
  maxPages:Number(context.maxPages)>0?Number(context.maxPages):500});
 // Adapters that read everything from the listing page have no detail fetch to skip, but their
 // rows still need a hash: it is what tells the next run, and the change check, what it has seen.
 const jobs=result.jobs.map(item=>item.listingHash?item:
  {...item,listingHash:hashListing(item.url,item.title,item.location)});
 return{seen:[],total:null,...result,jobs,mode:"direct-company"}}
