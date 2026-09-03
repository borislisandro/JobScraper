// Company-specific boards that no generic ATS adapter reads. Each one is the live-verified
// contract for that employer, ported from the standalone scraper: a generic "static-css" or
// "icims" guess against these hosts returns navigation links or 404s, not vacancies.
let cheerio,cheerioPromise;
const loadCheerio=()=>cheerioPromise??=import("cheerio").then(module=>cheerio=module);

export const companyAdapters={
 siemens:{name:"Siemens",host:"jobs.sw.siemens.com",base:"https://jobs.sw.siemens.com"},
 renesas:{name:"Renesas",host:"jobs.renesas.com",base:"https://jobs.renesas.com",smartRecruitersToken:"RenesasElectronics"},
 "arista-networks":{name:"Arista Networks",host:"www.arista.com",base:"https://www.arista.com",smartRecruitersToken:"AristaNetworks"},
 rambus:{name:"Rambus",host:"careers-rambus.icims.com",base:"https://careers-rambus.icims.com",listing:"https://careers-rambus.icims.com/jobs/search?in_iframe=1",datesOnDetailOnly:true},
 ceva:{name:"CEVA",host:"www.ceva-ip.com",base:"https://www.ceva-ip.com",listing:"https://www.comeet.co/careers-api/2.0/company/76.005/positions?token=67526BE6752D332D33026BE135F033A8&details=true"},
 // Arm's search results carry no posting date at all; the vacancy page does ("Date posted
 // Jun. 09, 2026"), so for this board the detail fetch is what a date costs and skipping it
 // leaves every Arm listing undated.
 arm:{name:"Arm",host:"careers.arm.com",base:"https://careers.arm.com",listing:"https://careers.arm.com/search-jobs",datesOnDetailOnly:true},
 amd:{name:"AMD",host:"careers.amd.com",base:"https://careers.amd.com"},
 // Public search credentials and widget ID published by ASML's careers page JavaScript.
 asml:{name:"ASML",host:"www.asml.com",base:"https://www.asml.com",listing:"https://www.asml.com/en/careers/find-your-job",
  searchUrl:"https://discover-euc1.sitecorecloud.io/discover/v2/126200477",searchKey:"01-967712c8-5a349c1760436ea6dccfd7bb02bfbe4dc2ccc36c"},
 mediatek:{name:"MediaTek",host:"careers.mediatek.com",base:"https://careers.mediatek.com"},
 google:{name:"Google",host:"www.google.com",base:"https://www.google.com/about/careers/applications/",listing:"https://www.google.com/about/careers/applications/jobs/results/"},
 cisco:{name:"Cisco",host:"careers.cisco.com",base:"https://careers.cisco.com",listing:"https://careers.cisco.com/global/en/search-results"},
 // The same Phenom payload, different tenants.
 "bae-systems":{name:"BAE Systems",host:"jobs.baesystems.com",base:"https://jobs.baesystems.com",listing:"https://jobs.baesystems.com/global/en/search-results"},
 thales:{name:"Thales",host:"careers.thalesgroup.com",base:"https://careers.thalesgroup.com",listing:"https://careers.thalesgroup.com/global/en/search-results"},
 "sk-hynix":{name:"SK hynix",host:"talent.skhynix.com",base:"https://talent.skhynix.com"},
 // Radancy (TalentBrew), same platform as Arm. Its cards carry the posting date, so unlike Arm's
 // this board does not need a detail page just to date a listing.
 synopsys:{name:"Synopsys",host:"careers.synopsys.com",base:"https://careers.synopsys.com",listing:"https://careers.synopsys.com/search-jobs"},
 // Same platform, an older theme: bare <li> cards and no posting date anywhere — not on the card
 // and not on the vacancy page either — so unlike Arm there is nothing a detail fetch would date.
 l3harris:{name:"L3Harris",host:"careers.l3harris.com",base:"https://careers.l3harris.com",listing:"https://careers.l3harris.com/search-jobs"},
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

function talentBrewDetail(html,current){
 const $=cheerio.load(html),withoutLabel=(selector,label)=>clean($(selector).first().text()?.replace(new RegExp(`^${label}\\s*`,"i"),""));
 return{...current,externalId:withoutLabel(".job-id","Job ID")||current.externalId,location:withoutLabel(".job-location","Location")||current.location,
  postedAt:withoutLabel(".job-date","Date posted")||current.postedAt,descriptionHtml:$(".ats-description").first().html()||undefined,
  description:clean($(".ats-description").first().text())||current.description}}

// Every board here whose listing page carries no description shares one shape: the vacancy page is
// the only place the text exists, a row identical to one already stored skips that page entirely,
// and a page that cannot be read degrades to listing data rather than failing the whole run. It
// was written out three times; the differences were the detail reader and the noun in the message.
async function enrichListings(jobs,{request,known,hashListing,acceptTitle=()=>true,fetchDetail=true},detail,label){
 const enriched=[],seen=[];let failures=0,filtered=0;
 for(const item of jobs){
  if(!item.url){enriched.push(item);continue}
  const hash=hashListing(item.url,item.title,item.location);
  if(!acceptTitle(item.title)){filtered+=1;if(known.has(hash))seen.push(hash);continue}
  if(known.has(hash)){seen.push(hash);continue}
  // The change check only asks what is on page one; enriching those rows would cost a request
  // each for information it is not going to store.
  const detailUrl=item.detailUrl||item.url;
  if(!fetchDetail){enriched.push({...item,listingHash:hash,detailUrl,descriptionStatus:"pending"});continue}
  try{enriched.push({...detail(await markup(request,detailUrl,{metricKind:"detail"}),item),listingHash:hash,detailUrl,descriptionStatus:"complete"})}
  catch{failures+=1;enriched.push({...item,listingHash:hash,detailUrl:item.url,descriptionStatus:"failed",descriptionError:`${label} could not be read.`})}}
 return{enriched,seen,filtered,warnings:failures?[`${failures} ${label}s could not be read; their listing data was kept.`]:[]}}

async function arm(site,context){
 const {request,maxPages}=context,jobs=[];let page=1,maxPage=Infinity;
 while(page<=maxPage&&page<=maxPages){
  const parsed=armPage(await markup(request,`${site.listing}?p=${page}`),site);
  jobs.push(...parsed.jobs);if(parsed.maxPage!==null)maxPage=parsed.maxPage;
  if(!parsed.jobs.length)break;page+=1}
 // Detail pages carry the requisition ID, the full location and the description; a listing-only
 // record is still a valid vacancy, so a failed detail fetch degrades instead of failing the run.
 const {enriched,seen,filtered,warnings}=await enrichListings(jobs,context,talentBrewDetail,"Arm detail page");
 // Complete means the last page the board itself named was read. Running out of listings is a
 // reason to stop, not proof of the end: a page that fails to render is indistinguishable from
 // one past the end, and treating it as the end reconciles every unread opening to closed.
 const extentKnown=Number.isFinite(maxPage),complete=extentKnown&&page>maxPage;
 return{jobs:enriched,seen,filtered,pages:Math.min(page-1,maxPage),complete,total:jobs.length,totalExact:complete,
  warnings:[...warnings,
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
async function ublox(site,context){
 const {request,maxPages}=context,jobs=[];let page=0,nbPages=Infinity,total=null,exhausted=false;
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
 const {enriched,seen,filtered,warnings:detailWarnings}=await enrichListings(jobs,context,ubloxDetail,"u-blox vacancy page");
 const extentKnown=Number.isFinite(nbPages)||exhausted,complete=extentKnown&&(exhausted||page>=nbPages);
 return{jobs:enriched,seen,filtered,pages:page,complete,total,totalExact:total!==null,
  warnings:[...detailWarnings,
   ...(extentKnown?[]:["u-blox reported no page count, so how much of the board was read is unknown. This run is treated as unfinished and nothing was marked closed."])]}}

// Synopsys runs on Radancy (TalentBrew), the same platform as Arm behind a different theme, so its
// vacancy pages are read by the same detail parser. Its robots.txt disallows "/search-jobs/", which
// is where the JSON results endpoint lives; the paged search page itself is not disallowed and is
// what this reads. That page states its own extent — how many vacancies matched and how many pages
// they occupy — so the read is reconciled against the board's own numbers rather than against
// running out of listings.
function radancyPage(html,site){
 const $=cheerio.load(html),results=$("#search-results").first();
 const declared=name=>{const value=Number(results.attr(name));return Number.isFinite(value)&&value>0?value:null};
 // Tenants theme their own cards — Synopsys names them, L3Harris ships a bare <li> — so the list
 // container and the job id the link carries are what identify a row, not a theme's class names.
 const jobs=$("#search-results-list li").map((_,el)=>{const card=$(el),link=card.find("a[data-job-id]").first();
  if(!link.length)return null;
  return{
  title:clean(link.find("h2").first().text()),location:clean(card.find("[class*=job-location]").first().text()),
  url:absolute(link.attr("href"),site.base),externalId:clean(link.attr("data-job-id")),
  // "Posted: 09/01/2026" — the label is part of the element's text, and the date behind it is the
  // board's own format, left for the shared date reader rather than parsed twice.
  postedAt:clean(card.find(".job-date-posted").first().text())?.replace(/^posted:?\s*/i,""),
  description:clean(card.find("[class*=category]").first().text())}}).get().filter(Boolean);
 return{jobs,total:declared("data-total-job-results"),maxPage:declared("data-total-pages")}}

async function radancy(site,context){
 const {request,maxPages}=context,jobs=[];let page=1,maxPage=Infinity,total=null;
 while(page<=maxPage&&page<=maxPages){
  const parsed=radancyPage(await markup(request,`${site.listing}?p=${page}`),site);
  jobs.push(...parsed.jobs);
  if(parsed.maxPage!==null)maxPage=parsed.maxPage;
  if(total===null)total=parsed.total;
  if(!parsed.jobs.length)break;page+=1}
 const {enriched,seen,filtered,warnings}=await enrichListings(jobs,context,talentBrewDetail,`${site.name} vacancy page`);
 // Reading every page the board named is not the same as having read every vacancy it counted: a
 // page that silently renders short would otherwise pass as a finished board and close the rest.
 const extentKnown=Number.isFinite(maxPage),read=page>maxPage,counted=total===null||jobs.length===total;
 return{jobs:enriched,seen,filtered,pages:Math.min(page-1,maxPage),complete:extentKnown&&read&&counted,
  total:total??jobs.length,totalExact:total!==null,
  warnings:[...warnings,
   ...(extentKnown?[]:[`${site.name} published no page count, so how much of the board was read is unknown. This run is treated as unfinished and nothing was marked closed.`]),
   ...(extentKnown&&read&&!counted?[`${site.name} listed ${total} vacancies but ${jobs.length} were read across its own page count; this run is treated as unfinished.`]:[])]}}

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

async function asml(site,{request,maxPages}){
 const jobs=[],ids=new Set();let offset=0,pages=0,total=null,changed=false;
 do{
  const payload=await json(request,site.searchUrl,{method:"POST",headers:{accept:"application/json","content-type":"application/json",authorization:site.searchKey},
   body:{context:{page:{uri:site.listing},locale:{country:"us",language:"en"}},widget:{items:[{entity:"content",rfk_id:"asml_job_search",search:{limit:100,offset,content:{}}}]}}});
  const widget=payload?.widgets?.find(item=>item.rfk_id==="asml_job_search");
  if(!Number.isInteger(widget?.total_item)||widget.total_item<0||widget.offset!==offset)
   throw Error("parse_error: Structure changed: ASML search total or offset is invalid");
  // Sitecore can report a page-context uri_not_found while returning valid job search data.
  if(widget.errors?.some(error=>error.type!=="uri_not_found"))throw Error("parse_error: ASML search reported an error");
  if(total!==null&&total!==widget.total_item)changed=true;
  total=widget.total_item;
  const items=array(widget.content,"ASML search content");
  for(const item of items){
   const id=clean(item.job_id||item.id),url=absolute(item.url,site.base);
   if(item.type!=="job_detail_page"||!id||!clean(item.name)||!url||new URL(url).host!==site.host)
    throw Error("parse_error: Structure changed: ASML search returned an invalid vacancy");
   if(ids.has(id))throw Error("parse_error: ASML search repeated a vacancy while paging");
   ids.add(id);jobs.push({externalId:id,title:clean(item.name),url,
    location:clean(item.job_location)||[item.job_city,item.job_country].filter(Boolean).join(", ")||null,
    postedAt:item.job_date_posted,description:item.description,descriptionHtml:item.description});
  }
  pages+=1;offset+=items.length;
  if(!items.length)break;
 }while(offset<total&&pages<maxPages);
 return{jobs,pages,total,totalExact:!changed,complete:!changed&&offset===total,
  warnings:changed?["ASML's total changed while paging; this read is unfinished and cannot close stored jobs."]:[]};
}

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

function phenomPage(html,site){
 // A Phenom tenant renders its result list client-side but ships the same payload inline as
 // phApp.ddo, which is the only readable listing index on these hosts. Cisco, BAE Systems and
 // Thales all serve it unchanged; the employer only differs in host and job-URL prefix.
 const startMarker="phApp.ddo = ",endMarker="; phApp.experimentData";
 const start=html.indexOf(startMarker),end=html.indexOf(endMarker,start+startMarker.length);
 if(start<0||end<0)throw Error(`parse_error: Structure changed: ${site.name} embedded search data is missing`);
 let data;try{data=JSON.parse(html.slice(start+startMarker.length,end))?.eagerLoadRefineSearch}
 catch(e){throw Error(`parse_error: Structure changed: ${site.name} embedded search data is unreadable: ${e.message}`,{cause:e})}
 const jobs=array(data?.data?.jobs,"eagerLoadRefineSearch.data.jobs").map(item=>({
  title:clean(item.title),
  location:(item.multi_location_array?.length?item.multi_location_array.map(x=>x?.location||x?.city||x?.name):item.multi_location?.length?item.multi_location:[item.location||item.cityStateCountry])
   .map(clean).filter(Boolean).filter((v,i,a)=>a.indexOf(v)===i).join("; ")||null,
  url:absolute(`${site.jobPath||"/global/en/job"}/${item.jobSeqNo||item.jobId}/${slug(item.title)}`,site.base),
  externalId:clean(item.reqId||item.jobId||item.jobSeqNo),postedAt:item.postedDate||item.posted_date||item.datePosted,
  description:item.descriptionTeaser}));
 return{jobs,total:Number(data?.totalHits)}}

async function phenom(site,{request,maxPages}){
 const jobs=[];let offset=0,page=0,total=Infinity;
 while(offset<total&&page<maxPages){
  const parsed=phenomPage(await markup(request,`${site.listing}?from=${offset}&s=1`),site);
  if(!Number.isFinite(parsed.total))throw Error(`parse_error: Structure changed: ${site.name} totalHits is missing`);
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

function icimsDetail(html,current){
 const $=cheerio.load(html);let posting;
 $("script[type='application/ld+json']").each((_,el)=>{
  const data=JSON.parse($(el).text());for(const item of Array.isArray(data)?data:data["@graph"]||[data])if(item["@type"]==="JobPosting")posting=item;
 });
 if(!posting?.description||!/^Rambus\b/i.test(posting.hiringOrganization?.name||""))throw Error("parse_error: Rambus JobPosting description or employer is missing");
 return{...current,description:posting.description,descriptionHtml:posting.description,postedAt:posting.datePosted,location:posting.jobLocation||current.location};
}

async function rambus(site,context){
 const jobs=[],ids=new Set();let url=site.listing,pages=0,maxPage=null,unstable=false;
 while(url&&pages<context.maxPages){
  const $=cheerio.load(await markup(context.request,url)),declared=Number(clean($(".iCIMS_Paging").text())?.match(/\bof\s+(\d+)/i)?.[1]);
  if(!Number.isInteger(declared)||declared<1)throw Error("parse_error: Rambus page count is missing");
  if(maxPage!==null&&maxPage!==declared)unstable=true;maxPage=declared;
  const items=$(".iCIMS_JobCardItem").map((_,el)=>{
   const card=$(el),link=card.find(".title a").first(),url=absolute(link.attr("href"),site.base),id=url?.match(/\/jobs\/(\d+)\//)?.[1];
   if(!id||!clean(link.find("h3").text()))throw Error("parse_error: Rambus vacancy card changed");
   if(ids.has(id))unstable=true;ids.add(id);
   return{externalId:id,title:clean(link.find("h3").text()),url,location:clean(card.find(".header.left span:not(.sr-only)").text())};
  }).get();
  if(!items.length){unstable=true;break}jobs.push(...items);pages++;
  url=absolute($("link[rel=next]").attr("href"),site.base);
  if(url&&(new URL(url).host!==site.host||Number(new URL(url).searchParams.get("pr"))!==pages))throw Error("parse_error: Rambus next page repeated or left its board");
 }
 const complete=!unstable&&!url&&pages===maxPage;
 const {enriched,seen,filtered,warnings}=await enrichListings(jobs,context,icimsDetail,"Rambus detail page");
 return{jobs:enriched,seen,filtered,pages,total:complete?jobs.length:null,totalExact:complete,complete,warnings};
}

async function ceva(site,{request}){
 const items=array(await json(request,site.listing),"CEVA positions"),ids=new Set();
 const jobs=items.map(item=>{
  if(!item.uid||ids.has(item.uid)||item.is_internal!==false||!/^ceva$/i.test(item.company_name||""))throw Error("parse_error: CEVA repeated a posting or returned a private/wrong-company row");
  ids.add(item.uid);
  const description=array(item.details,"CEVA description sections").map(section=>section.value).filter(Boolean).join("\n\n");
  if(!clean(item.name)||!description)throw Error("parse_error: CEVA title or description is missing");
  const url=absolute(item.url_active_page,site.base);if(!url||new URL(url).host!==site.host)throw Error("parse_error: CEVA posting left the official board");
  return{externalId:item.uid,title:clean(item.name),url,location:clean(item.location?.name),description,descriptionHtml:description,workMode:clean(item.workplace_type)?.toLowerCase()};
 });
 return{jobs,pages:1,total:jobs.length,totalExact:true,complete:true,warnings:[]};
}

async function siemens(site,{request,maxPages}){
 const jobs=[],ids=new Set(),featured=new Set();let page=1,total=null,unstable=false,ended=false;
 while(page<=maxPages){
  const data=await json(request,`https://prod-search-api.jobsyn.org/api/v1/solr/search?page=${page}&num_items=10`,{headers:{accept:"application/json","x-origin":site.host}});
  const pager=data.pagination,items=array(data.jobs,"Siemens Software jobs");
  if(!pager||!Number.isInteger(pager.total)||pager.total<0||pager.page!==page||pager.offset!==jobs.length||typeof pager.has_more_pages!=="boolean")throw Error("parse_error: Siemens Software pagination changed");
  if(total!==null&&total!==pager.total)unstable=true;total=pager.total;
  // Featured placements are not additional vacancies. Require them to occur in the counted list.
  for(const item of array(data.featured_jobs,"Siemens Software featured jobs"))featured.add(item.guid);
  for(const item of items){
   if(!clean(item.guid)||!clean(item.title_exact)||!/^(?:Siemens|Mendix)\b/.test(item.company_exact||"")||!clean(item.description))throw Error("parse_error: Siemens Software returned an invalid posting");
   if(ids.has(item.guid))unstable=true;ids.add(item.guid);
   jobs.push({externalId:item.guid,title:clean(item.title_exact),company:clean(item.company_exact),
    location:clean(item.location_exact),url:`${site.base}/${slug(item.location_exact)}/${item.title_slug}/${item.guid}/job/`,
    postedAt:item.date_added,description:item.description});
  }
  if(!pager.has_more_pages){ended=true;break}
  if(!items.length)break;page++;
 }
 return{jobs,pages:Math.min(page,maxPages),total,totalExact:true,complete:ended&&!unstable&&jobs.length===total&&[...featured].every(id=>ids.has(id)),
  warnings:unstable?["Siemens Software changed its total or repeated a posting while paging; this read cannot close stored jobs."]:[]};
}

function smartRecruitersDetail(text,current){
 const data=JSON.parse(text);
 if(data.active!==true||String(data.id)!==current.externalId)throw Error("parse_error: SmartRecruiters detail is inactive or belongs to another posting");
 const description=Object.values(data.jobAd?.sections||{}).map(section=>section.text).filter(Boolean).join("\n\n");
 if(!description)throw Error("parse_error: SmartRecruiters description is missing");
 return{...current,description,descriptionHtml:description};
}

async function smartRecruiters(site,context){
 const {request,maxPages}=context,token=encodeURIComponent(site.smartRecruitersToken),jobs=[],ids=new Set();
 let offset=0,pages=0,total=null,unstable=false;
 do{
  const data=await json(request,`https://api.smartrecruiters.com/v1/companies/${token}/postings?limit=100&offset=${offset}`);
  if(!Number.isInteger(data.totalFound)||data.totalFound<0||data.offset!==offset)throw Error("parse_error: SmartRecruiters pagination is missing or repeated");
  if(total!==null&&total!==data.totalFound)unstable=true;
  total=data.totalFound;
  const items=array(data.content,"SmartRecruiters content");
  for(const item of items){
   if(!item.id||!clean(item.name)||item.company?.identifier!==site.smartRecruitersToken||item.visibility!=="PUBLIC")throw Error("parse_error: SmartRecruiters returned an invalid or wrong-company posting");
   const id=String(item.id);if(ids.has(id))unstable=true;ids.add(id);
   jobs.push({externalId:id,title:clean(item.name),company:clean(item.company.name),
    url:`https://jobs.smartrecruiters.com/${token}/${encodeURIComponent(id)}`,
    detailUrl:`https://api.smartrecruiters.com/v1/companies/${token}/postings/${encodeURIComponent(id)}`,
    location:[item.location?.city,item.location?.region,item.location?.country?.toUpperCase()].map(clean).filter(Boolean).join(", ")||null,
    postedAt:item.releasedDate});
  }
  pages++;offset+=items.length;if(!items.length)break;
 }while(offset<total&&pages<maxPages);
 const {enriched,seen,filtered,warnings}=await enrichListings(jobs,context,smartRecruitersDetail,"SmartRecruiters detail");
 return{jobs:enriched,seen,filtered,pages,total,totalExact:true,complete:!unstable&&offset===total,
  warnings:[...warnings,...(unstable?["SmartRecruiters changed its total or repeated a posting while paging; this run cannot close stored jobs."]:[])]};
}

const scrapers={rambus,ceva,siemens,renesas:smartRecruiters,"arista-networks":smartRecruiters,arm,amd,asml,mediatek,google,cisco:phenom,"bae-systems":phenom,thales:phenom,l3harris:radancy,"sk-hynix":skHynix,synopsys:radancy,"u-blox":ublox};
export const companyNeedsCheerio=(adapterId,fetchDetail=true)=>
 ["rambus","arm","google","sk-hynix","synopsys","l3harris"].includes(adapterId)||(adapterId==="u-blox"&&fetchDetail);

export async function enrichCompany(adapterId,item,request){
 const enrichers={rambus:icimsDetail,renesas:smartRecruitersDetail,"arista-networks":smartRecruitersDetail,arm:talentBrewDetail,synopsys:talentBrewDetail,l3harris:talentBrewDetail,"u-blox":ubloxDetail};
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
