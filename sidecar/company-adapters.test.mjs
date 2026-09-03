// The company adapters address their employer's own hosts, so they are exercised here through
// the same injected request function the worker hands them rather than through a fixture host.
// Each case is the shape that host actually returns; the three assertions that look pedantic
// (Cisco's location objects, Google's pre-separated location spans, MediaTek's swapped
// label/code) are the three fields that came back as noise from the live boards.
import assert from "node:assert/strict";import {test}from "node:test";
import {companyAdapterIds,companyDatesNeedDetail,companyNeedsCheerio,scrapeCompany,enrichCompany}from "./company-adapters.mjs";

test("Siemens Software uses the official tenant, full count and unique location postings",async()=>{
 const posting=id=>({guid:id,reqid:"same-requisition",title_exact:"VIP Verification Engineer",title_slug:"vip-verification-engineer",company_exact:"Siemens",location_exact:"Taipei, TWN",description:"Verify EDA tools.",date_added:"2026-09-02"});
 const run=(change={})=>scrapeCompany("siemens",{maxPages:3,hashListing:key=>key,request:async(url,options)=>{
  assert.equal(options.headers["x-origin"],"jobs.sw.siemens.com");const page=Number(new URL(url).searchParams.get("page"));
  return{text:JSON.stringify({featured_jobs:[posting("1")],jobs:[posting(String(page))],pagination:{total:2,page,offset:page-1,has_more_pages:page<2},...(typeof change==="function"?change(page):change)})};
 }});
 const result=await run();assert.equal(result.complete,true);assert.equal(result.total,2);assert.equal(result.jobs.length,2);
 assert.equal(result.jobs[0].externalId,"1");assert.equal(result.jobs[1].externalId,"2","distinct location postings keep distinct identities");
 assert.equal(result.jobs[0].url,"https://jobs.sw.siemens.com/taipei-twn/vip-verification-engineer/1/job/");
 assert.equal((await run({featured_jobs:[posting("missing")]})).complete,false);
 assert.equal((await run(page=>({pagination:{total:page===1?3:2,page,offset:page-1,has_more_pages:page<2}}))).complete,false);
 assert.equal((await run({jobs:[posting("1")]})).complete,false);
 assert.equal((await run(page=>({pagination:{total:3,page,offset:page-1,has_more_pages:false}}))).complete,false);
 await assert.rejects(run({pagination:{total:2,page:1,offset:0}}),/pagination changed/);
});

test("SmartRecruiters reconciles all postings and defers the real description endpoint",async()=>{
 const posting=id=>({id,name:`Verification ${id}`,company:{identifier:"RenesasElectronics",name:"Renesas Electronics"},visibility:"PUBLIC",location:{city:"Raanana",country:"il"},releasedDate:"2026-09-02"});
 const calls=[],request=async url=>{calls.push(url);const offset=Number(new URL(url).searchParams.get("offset"));return{text:JSON.stringify({offset,totalFound:3,content:offset===0?[posting("1"),posting("2")]:[posting("3")]})}};
 const context={request,maxPages:3,fetchDetail:false,hashListing:key=>key};
 const result=await scrapeCompany("renesas",context);
 assert.equal(result.complete,true);assert.equal(result.total,3);assert.equal(result.pages,2);
 assert.equal(result.jobs[0].location,"Raanana, IL");assert.equal(result.jobs[0].descriptionStatus,"pending");
 assert.match(calls[1],/offset=2$/);assert.match(result.jobs[0].detailUrl,/postings\/1$/);
 const enriched=await enrichCompany("renesas",result.jobs[0],async url=>{
  assert.equal(url,result.jobs[0].detailUrl);return{text:JSON.stringify({id:"1",active:true,jobAd:{sections:{jobDescription:{text:"<p>Verify ASICs.</p>"},qualifications:{text:"<p>SystemVerilog.</p>"}}}})};
 });
 assert.match(enriched.description,/Verify ASICs/);assert.match(enriched.description,/SystemVerilog/);
 const cap=await scrapeCompany("renesas",{...context,maxPages:1});assert.equal(cap.complete,false);
 const changed=await scrapeCompany("renesas",{...context,request:async url=>{const offset=Number(new URL(url).searchParams.get("offset"));return{text:JSON.stringify({offset,totalFound:offset?2:3,content:[posting(offset?"2":"1")]})}}});
 assert.equal(changed.complete,false);
 const repeated=await scrapeCompany("renesas",{...context,request:async url=>({text:JSON.stringify({offset:Number(new URL(url).searchParams.get("offset")),totalFound:2,content:[posting("1")]})})});assert.equal(repeated.complete,false);
 await assert.rejects(scrapeCompany("renesas",{...context,request:async()=>({text:JSON.stringify({offset:0,totalFound:1,content:[{...posting("1"),company:{identifier:"wrong"}}]})})}),/wrong-company/);
});

const respond=routes=>{const seen=[];const request=async(url,options={})=>{seen.push({url,options});
 const hit=Object.entries(routes).find(([fragment])=>url.includes(fragment));
 if(!hit)throw Error(`network_error HTTP 404 for ${url}`);
 return{text:typeof hit[1]==="string"?hit[1]:JSON.stringify(hit[1]),url,type:"application/json"}};
 return{request,seen}};
// "requests" is this harness's log of what was fetched; "seen" belongs to the adapter and lists
// the listings it recognised and skipped, so the two must not share a name.
const scrape=(id,routes,config={})=>{const{request,seen}=respond(routes);
 return scrapeCompany(id,{request,maxPages:1,known:new Set(),hashListing:()=>"listing-hash",...config})
  .then(result=>({...result,requests:seen}))};

test("Arm listing cards are enriched from their detail pages and editorial cards are dropped",async()=>{
 const listing=`<ul>
  <li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/lisbon/soc-engineer/1/7">SoC Engineer</a><span class="location">Lisbon</span><span class="category">Hardware</span></li>
  <li class="job-card"><a class="job-card__title" href="/insights/life-at-arm">Life at Arm</a></li>
 </ul><select id="pagination-current-bottom"><option value="1">1</option></select>`;
 const detail=`<div class="job-id">Job ID 2026-17397</div><div class="job-location">Location Lisbon, Portugal</div><div class="job-date">Date posted 2026-08-14</div><div class="ats-description"><p>Design SoCs.</p></div>`;
 const result=await scrape("arm",{"/search-jobs":listing,"/job/lisbon":detail});
 assert.equal(result.jobs.length,1,"the editorial card is not a vacancy");
 assert.deepEqual({...result.jobs[0],descriptionHtml:undefined,listingHash:undefined,detailUrl:undefined,descriptionStatus:undefined},{title:"SoC Engineer",location:"Lisbon, Portugal",url:"https://careers.arm.com/job/lisbon/soc-engineer/1/7",externalId:"2026-17397",postedAt:"2026-08-14",description:"Design SoCs.",descriptionHtml:undefined,listingHash:undefined,detailUrl:undefined,descriptionStatus:undefined});
 assert.equal(result.jobs[0].descriptionStatus,"complete");
 assert.equal(result.jobs[0].listingHash,"listing-hash","the stored row carries the hash the next run compares against");
 assert.equal(result.complete,true)});

// The detail fetch is the entire cost of an Arm or Workday run. A listing row identical to the
// one already stored must cost zero requests and still be reported as seen, or the availability
// sweep would mark every unchanged job closed.
test("a listing row already stored skips its detail page and is reported as seen",async()=>{
 const listing=`<ul>
  <li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/known/1/7">Known Engineer</a><span class="location">Lisbon</span></li>
  <li class="job-card" data-job-id="8"><a class="job-card__title" href="/job/fresh/1/8">Fresh Engineer</a><span class="location">Porto</span></li>
 </ul><select id="pagination-current-bottom"><option value="1">1</option></select>`;
 const detail=`<div class="job-id">Job ID NEW-1</div><div class="ats-description"><p>Fresh.</p></div>`;
 const fetched=[];
 const request=async url=>{fetched.push(url);
  return{text:url.includes("/search-jobs")?listing:detail,url,type:"text/html"}};
 const hashListing=(key)=>`hash:${key}`;
 const result=await scrapeCompany("arm",{request,maxPages:1,hashListing,
  known:new Set(["hash:https://careers.arm.com/job/known/1/7"])});
 assert.deepEqual(result.seen,["hash:https://careers.arm.com/job/known/1/7"]);
 assert.equal(result.jobs.length,1,"only the unrecognised listing is enriched");
 assert.equal(result.jobs[0].externalId,"NEW-1");
 assert.ok(!fetched.some(url=>url.includes("/job/known/")),"the known job cost no request");
 assert.ok(fetched.some(url=>url.includes("/job/fresh/")),"the new job was still read in full")});

test("Arm keeps listing data when a detail page cannot be read",async()=>{
 const listing=`<ul><li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/x/1/7">SoC Engineer</a><span class="location">Lisbon</span></li></ul>`;
 const result=await scrape("arm",{"/search-jobs":listing});
 assert.equal(result.jobs.length,1);
 assert.equal(result.jobs[0].externalId,"7");
 assert.match(result.warnings.join(" "),/1 Arm detail pages could not be read/)});

test("AMD reads the paged jobs API and stops at its reported total",async()=>{
 const result=await scrape("amd",{"/api/jobs":{totalCount:1,jobs:[{data:{slug:"88885",req_id:"88885",title:"Mechanical Engineer",full_location:"Austin, Texas",posted_date:"2026-08-29",description:"Build things."}}]}});
 assert.equal(result.complete,true);
 assert.match(result.requests[0].url,/limit=100/,"AMD uses the largest verified page size");
 assert.deepEqual(result.jobs[0],{title:"Mechanical Engineer",location:"Austin, Texas",url:"https://careers.amd.com/careers-home/jobs/88885",externalId:"88885",postedAt:"2026-08-29",description:"Build things.",descriptionHtml:"Build things.",listingHash:"listing-hash"});
 assert.equal(result.total,1,"the board's own count is what the change check compares against")});

const asmlItem=id=>({id,job_id:id,type:"job_detail_page",name:`Engineer ${id}`,url:`https://www.asml.com/en/careers/find-your-job/${id}`,job_location:"Veldhoven, Netherlands",job_date_posted:"2026-09-02T00:00:00",description:"<p>Design lithography systems.</p>"});
const asmlPage=(offset,total,ids)=>({widgets:[{rfk_id:"asml_job_search",offset,total_item:total,content:ids.map(asmlItem),errors:[{type:"uri_not_found"}]}]});
test("ASML traverses the published Sitecore widget and keeps inline descriptions and stable IDs",async()=>{
 const offsets=[];
 const result=await scrapeCompany("asml",{maxPages:3,request:async(url,options)=>{
  assert.equal(url,"https://discover-euc1.sitecorecloud.io/discover/v2/126200477");
  const search=options.body.widget.items[0].search;offsets.push(search.offset);assert.equal(search.limit,100);
  return{text:JSON.stringify(search.offset===0?asmlPage(0,3,["J-1","J-2"]):asmlPage(2,3,["J-3"]))};}});
 assert.deepEqual(offsets,[0,2]);assert.equal(result.complete,true);assert.equal(result.total,3);
 assert.equal(result.jobs[0].externalId,"J-1");assert.equal(result.jobs[0].location,"Veldhoven, Netherlands");
 assert.equal(result.jobs[0].descriptionHtml,"<p>Design lithography systems.</p>");
});
test("ASML capped, early-empty and changing-total reads cannot report completion",async()=>{
 const capped=await scrape("asml",{"/discover/v2/":asmlPage(0,3,["J-1"])});assert.equal(capped.complete,false);
 for(const changing of[false,true]){
  const result=await scrapeCompany("asml",{request:async(_url,options)=>{
   const offset=options.body.widget.items[0].search.offset;
   return{text:JSON.stringify(offset===0?asmlPage(0,3,["J-1"]):asmlPage(offset,changing?2:3,changing?["J-2"]:[]))};}});
  assert.equal(result.complete,false);if(changing)assert.equal(result.totalExact,false);
 }
});
test("ASML rejects repeated pages, invalid totals, non-job records and search errors",async()=>{
 for(const alter of[page=>delete page.widgets[0].total_item,page=>page.widgets[0].content[0].type="editorial",page=>page.widgets[0].errors=[{type:"invalid_query"}]]){
  const page=asmlPage(0,1,["J-1"]);alter(page);await assert.rejects(scrape("asml",{"/discover/v2/":page}),/parse_error/);
 }
 for(const repeatOffset of[false,true])await assert.rejects(scrapeCompany("asml",{request:async(_url,options)=>{
  const offset=options.body.widget.items[0].search.offset;
  return{text:JSON.stringify(asmlPage(repeatOffset?0:offset,2,["J-1"]))};}}),/parse_error/);
});

test("MediaTek sends the locale cookie and reads the location out of the swapped label/code pair",async()=>{
 const item={id:"MTK1",title:"Linux Software Engineer",location:null,publishedDate:"2026-08-30T16:00:00.000+00:00",summary:"Kernel work.",properties:{location:{label:"0000009255",code:"Taipei"}}};
 const result=await scrape("mediatek",{"job.getJobs":{result:{data:{json:{pagination:{total_pages:1},jobs:[item]}}}}});
 assert.equal(result.jobs[0].location,"Taipei");
 assert.equal(result.jobs[0].url,"https://careers.mediatek.com/en/jobs/MTK1");
 assert.equal(result.requests[0].options.headers.cookie,"NEXT_LOCALE=en")});

test("Google joins its already-separated location spans once and drops the search query from URLs",async()=>{
 const card=`<li class="lLd3Je"><h3 class="QJPWVe">Software Engineer</h3>
  <a href="/about/careers/applications/jobs/results/100-software-engineer?location=US&amp;page=3">Learn more</a>
  <span class="r0wTof">Mountain View, CA, USA</span><span class="r0wTof">; Cambridge, MA, USA</span>
  <span class="r0wTof">Mountain View, CA, USA</span><span class="r0wTof">; Cambridge, MA, USA</span></li>`;
 const result=await scrape("google",{"/jobs/results/":`<div class="SWhIm">1</div><ul>${card}</ul>`});
 assert.equal(result.jobs[0].location,"Mountain View, CA, USA; Cambridge, MA, USA");
 assert.equal(result.jobs[0].url,"https://www.google.com/about/careers/applications/jobs/results/100-software-engineer");
 assert.equal(result.jobs[0].externalId,"100");
 assert.equal(result.complete,true)});

test("Google reports the robots policy that stops it paging instead of stopping silently",async()=>{
 const page=id=>`<div class="SWhIm">99</div><ul><li class="lLd3Je"><h3>Engineer ${id}</h3><a href="/about/careers/applications/jobs/results/${id}-engineer">x</a></li></ul>`;
 const request=async url=>{if(url.includes("page=2"))throw Error("robots_denied");return{text:page("100"),url,type:"text/html"}};
 const result=await scrapeCompany("google",{request,maxPages:5});
 assert.equal(result.jobs.length,1);
 assert.equal(result.complete,false);
 assert.match(result.warnings.join(" "),/robots\.txt disallows paged careers results/)});

test("Cisco reads the payload embedded in its search page and flattens its location objects",async()=>{
 const payload={eagerLoadRefineSearch:{totalHits:1,data:{jobs:[{jobSeqNo:"CISC1",jobId:"1",reqId:"2002268",title:"EMEA Cloud Compliance Leader",postedDate:"2026-01-19",descriptionTeaser:"Lead compliance.",location:"",multi_location_array:[{location:"Milan, Milano, Italy"},{location:"Porto, Porto, Portugal"}]}]}}};
 const result=await scrape("cisco",{"/search-results":`<script>phApp.ddo = ${JSON.stringify(payload)}; phApp.experimentData = {};</script>`});
 assert.equal(result.jobs[0].location,"Milan, Milano, Italy; Porto, Porto, Portugal");
 assert.equal(result.jobs[0].externalId,"2002268");
 assert.equal(result.jobs[0].url,"https://careers.cisco.com/global/en/job/CISC1/emea-cloud-compliance-leader");
 assert.equal(result.complete,true)});

test("Cisco says the page structure changed rather than reporting an empty board",async()=>{
 await assert.rejects(scrape("cisco",{"/search-results":"<html>no payload</html>"}),/^Error: parse_error: Structure changed/)});

test("SK hynix combines its two boards and keeps only its own entity on the shared one",async()=>{
 const result=await scrape("sk-hynix",{
  "skcareers.com":{list:[{noticeID:"R261845",title:"DRAM Design Engineer",corpName:"SK hynix",workingArea:"Gyeonggi/Incheon",start:"August 21(Fri)",jobRole:"Design"},
                         {noticeID:"R261881",title:"Plasma Operator",corpName:"SK plasma",workingArea:"Gyeongsang",start:"August 22(Sat)"}]},
  "greenhouse.io":{jobs:[{id:169,requisition_id:"169",title:"3D Stacked DRAM Design Engineer",location:{name:"San Jose, CA"},absolute_url:"https://job-boards.greenhouse.io/skhynixamerica/jobs/5362335008",first_published:"2026-07-16",content:"&lt;p&gt;Stack DRAM.&lt;/p&gt;"}]}});
 assert.equal(result.jobs.length,2,"the SK plasma posting belongs to a different company");
 assert.equal(result.jobs[0].externalId,"R261845");
 assert.equal(result.jobs[0].postedAt,"August 21");
 assert.equal(result.jobs[1].description,"<p>Stack DRAM.</p>","Greenhouse escapes its HTML once more than the shared normalizer decodes")});

test("an unknown company adapter is refused as incomplete, not guessed at",async()=>{
 assert.ok(companyAdapterIds.includes("asml"));
 await assert.rejects(scrapeCompany("nope",{request:async()=>({text:"{}"})}),/^Error: incomplete/)});

test("JSON-only company paths do not load Cheerio",()=>{
 assert.equal(companyNeedsCheerio("cisco"),false);
 assert.equal(companyNeedsCheerio("u-blox",false),false);
 assert.equal(companyNeedsCheerio("u-blox",true),true);
 assert.equal(companyNeedsCheerio("arm",false),true)});

test("a run stopped by its own page budget says so instead of looking finished",async()=>{
 const page=n=>`<div class="SWhIm">999</div><ul><li class="lLd3Je"><h3>Engineer ${n}</h3><a href="/about/careers/applications/jobs/results/${n}-engineer">x</a></li></ul>`;
 let seen=0;
 const request=async url=>{seen+=1;return{text:page(seen),url,type:"text/html"}};
 const result=await scrapeCompany("google",{request,maxPages:2});
 assert.equal(result.jobs.length,2);
 assert.equal(result.complete,false,"the board has 999 openings and only 2 were read")});

// The change check reads page one and asks whether anything on it is new. Enriching those rows
// would cost one request each for a description the check never stores.
test("the change check reads Arm's listing page without opening any detail page",async()=>{
 const listing=`<ul>
  <li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/a/1/7">Engineer A</a><span class="location">Lisbon</span></li>
  <li class="job-card" data-job-id="8"><a class="job-card__title" href="/job/b/1/8">Engineer B</a><span class="location">Porto</span></li>
 </ul><select id="pagination-current-bottom"><option value="1">1</option></select>`;
 const fetched=[];
 const request=async url=>{fetched.push(url);return{text:listing,url,type:"text/html"}};
 const result=await scrapeCompany("arm",{request,maxPages:1,fetchDetail:false,
  known:new Set(),hashListing:(key)=>`hash:${key}`});
 assert.equal(fetched.length,1,`page one only, got ${fetched.length} requests`);
 assert.equal(result.jobs.length,2,"both listings are still reported");
 assert.ok(result.jobs.every(job=>job.listingHash),"each carries the hash the check compares");
});

// u-blox publishes nothing in its job-openings HTML: the page ships an empty widget and fills it
// from an Algolia index, which is why this source read as "blocked" and stored zero vacancies.
// The shape below is the live index's, captured 2026-08-31.
const ubloxHit = (suffix, extra = {}) => ({
 id: `a2SVl00000${suffix}`, name: `Senior Hardware Engineer ${suffix}`, vacancy_name: `Senior Hardware Engineer ${suffix}`,
 department: "IC Design", locations_legacy: ["Headquarters (Thalwil), Switzerland"], also_available_in: ["Thalwil, Switzerland"],
 link_https: `https://fs-4627.my.salesforce-sites.com/recruit/fRecruit__ApplyJob?vacancyNo=VN${suffix}`,
 percentage: "Full Time", objectID: suffix, ...extra,
});
const ubloxIndex = (hits, nbPages = 1, hitsPerPage = 100) => ({ results: [{ hits, nbHits: hits.length, nbPages, hitsPerPage }] });
const vacancyPage = `<div class="pbSubsection">Vacancy NameSenior Hardware Engineer</div>
 <div class="sfdc_richtext"><p>About the role</p><p>Design radios.</p></div>
 <div class="sfdc_richtext"><p>u-blox is a global leader.</p></div>`;

test("u-blox vacancies come from its Algolia index and are enriched from Salesforce Recruit",async()=>{
 const calls=[];
 const request=async(url,options={})=>{calls.push({url,method:options.method??"GET",body:options.body});
  if(url.includes("algolia.net"))return{text:JSON.stringify(ubloxIndex([ubloxHit("2656"),ubloxHit("2693")])),url,type:"application/json"};
  return{text:vacancyPage,url,type:"text/html"}};
 const result=await scrapeCompany("u-blox",{request,maxPages:5,known:new Set(),hashListing:(key)=>`hash:${key}`});
 assert.equal(result.jobs.length,2);
 assert.equal(result.complete,true);
 assert.equal(result.total,2);
 assert.equal(result.mode,"direct-company");
 const [first]=result.jobs;
 assert.equal(first.title,"Senior Hardware Engineer 2656");
 assert.equal(first.location,"Headquarters (Thalwil), Switzerland; Thalwil, Switzerland","both the office and the alternates are kept");
 assert.equal(first.externalId,"VN2656","the vacancy number is the stable key, not the Salesforce record id");
 assert.equal(first.url,"https://fs-4627.my.salesforce-sites.com/recruit/fRecruit__ApplyJob?vacancyNo=VN2656");
 assert.equal(first.postedAt,null,"the index publishes no posting date and none is invented");
 assert.equal(first.descriptionStatus,"complete");
 assert.match(first.description,/About the role[\s\S]*Design radios\.[\s\S]*global leader/,"both rich-text blocks are the description");
 // One search plus one vacancy page each, and the search is a POST carrying the index name.
 const search=calls.find(call=>call.url.includes("algolia.net"));
 assert.equal(search.method,"POST");
 assert.equal(search.body.requests[0].indexName,"open_positions_en-US");
 assert.equal(calls.filter(call=>call.url.includes("ApplyJob")).length,2);
});

test("a u-blox vacancy already stored costs no request, and a listings-only read defers descriptions",async()=>{
 const fetched=[];
 const request=async(url)=>{fetched.push(url);
  if(url.includes("algolia.net"))return{text:JSON.stringify(ubloxIndex([ubloxHit("2656"),ubloxHit("2693")])),url,type:"application/json"};
  return{text:vacancyPage,url,type:"text/html"}};
 const known=new Set(["hash:https://fs-4627.my.salesforce-sites.com/recruit/fRecruit__ApplyJob?vacancyNo=VN2656"]);
 const warm=await scrapeCompany("u-blox",{request,maxPages:5,known,hashListing:(key)=>`hash:${key}`});
 assert.deepEqual(warm.seen,[...known],"the stored vacancy is reported as still open");
 assert.equal(warm.jobs.length,1);
 assert.equal(fetched.filter(url=>url.includes("VN2656")).length,0,"its vacancy page was never opened");
 fetched.length=0;
 const listingsOnly=await scrapeCompany("u-blox",{request,maxPages:5,known:new Set(),fetchDetail:false,hashListing:(key)=>`hash:${key}`});
 assert.equal(listingsOnly.jobs.length,2);
 assert.deepEqual([...new Set(listingsOnly.jobs.map(job=>job.descriptionStatus))],["pending"]);
 assert.equal(fetched.filter(url=>url.includes("ApplyJob")).length,0,"a listings-only read opens no vacancy page");
});

test("a u-blox index that changes shape fails loudly instead of reporting an empty board",async()=>{
 const request=async(url)=>({text:JSON.stringify({results:[{}]}),url,type:"application/json"});
 await assert.rejects(scrapeCompany("u-blox",{request,maxPages:1,known:new Set(),hashListing:()=>""}),/parse_error/);
});

// Both adapters used to invent the answer to "how many pages are there?" when the board stopped
// publishing one: Arm collapsed to Math.max(1,...[])===1 and u-blox to page+1. Either way one
// page reported itself as the whole board, and a whole board that reads as complete tells the
// database every opening it did not see has closed.
test("Arm without a page selector reports an unfinished read rather than a one-page board",async()=>{
 const card=`<li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/lisbon/soc-engineer/1/7">SoC Engineer</a><span class="location">Lisbon</span></li>`;
 const withSelector=await scrape("arm",{"/search-jobs":`<ul>${card}</ul><select id="pagination-current-bottom"><option value="1">1</option></select>`,"/job/lisbon":"<div class='ats-description'>x</div>"});
 assert.equal(withSelector.complete,true,"a board that names its last page and reaches it is complete");
 const broken=await scrape("arm",{"/search-jobs":`<ul>${card}</ul>`,"/job/lisbon":"<div class='ats-description'>x</div>"});
 assert.equal(broken.complete,false,"an unreadable page count is not a one-page board");
 assert.equal(broken.totalExact,false);
 assert.equal(broken.jobs.length,1,"the listings that were read are still returned");
 assert.ok(broken.warnings.some(warning=>warning.includes("page selector")),broken.warnings.join(" | "));
});

test("Arm stops at the page count published by its current input paginator",async()=>{
 const pages=[];
 const request=async url=>{const page=Number(new URL(url).searchParams.get("p"));pages.push(page);
  return{text:`<ul><li class="job-card" data-job-id="${page}"><a class="job-card__title" href="/job/x/1/${page}">Engineer ${page}</a></li></ul><nav data-pagination-for="search-results-jobs"><input id="pagination-current-bottom" max="2" value="${page}"></nav>`,url,type:"text/html"}};
 const result=await scrapeCompany("arm",{request,maxPages:500,fetchDetail:false,known:new Set(),hashListing:key=>key});
 assert.deepEqual(pages,[1,2],"the adapter must not request out-of-range pages");
 assert.equal(result.complete,true);
});

test("u-blox falls back to the short-page signal, and reports an unfinished read without either",async()=>{
 const routes=index=>({"algolia.net":index,"salesforce-sites.com":vacancyPage});
 const whole=await scrape("u-blox",{...routes(ubloxIndex([ubloxHit("2656")],1))},{maxPages:5});
 assert.equal(whole.complete,true,"a board that reports one page and delivers it is complete");
 // No usable nbPages, but one hit against a requested 100 is Algolia saying this was the last page.
 const short=await scrape("u-blox",{...routes(ubloxIndex([ubloxHit("2656")],"not a number"))},{maxPages:5});
 assert.equal(short.complete,true,"a page shorter than the one requested ends the index");
 assert.equal(short.requests.filter(entry=>entry.url.includes("algolia.net")).length,1,"and it stops there");
 // Full pages with no page count: the end is never reached, so the read is unfinished.
 const unknown=await scrape("u-blox",{...routes(ubloxIndex([ubloxHit("2656"),ubloxHit("2693")],"not a number",2))},{maxPages:2});
 assert.equal(unknown.complete,false,"a missing page count is not proof the first page was the last");
 assert.equal(unknown.jobs.length,4,"the vacancies that were read are still returned");
 assert.ok(unknown.warnings.some(warning=>warning.includes("page count")),unknown.warnings.join(" | "));
});


// Arm's search results carry no date; its vacancy pages do ("Date posted Jun. 09, 2026"). A scrape
// defers detail pages for speed, so before this every Arm listing was stored undated and no date
// filter could ever see one.
test("a board that only dates its detail pages says so, and the date survives the markup it ships",async()=>{
 assert.equal(companyDatesNeedDetail("arm"),true);
 assert.equal(companyDatesNeedDetail("u-blox"),false,"u-blox publishes no date anywhere, so a detail fetch buys nothing");
 assert.equal(companyDatesNeedDetail("mediatek"),false);
 const listing='<ul><li class="job-card" data-job-id="7"><a class="job-card__title" href="/job/austin/soc-engineer/1/7">SoC Engineer</a><span class="location">Austin</span></li></ul><select id="pagination-current-bottom"><option value="1">1</option></select>';
 // Exactly what careers.arm.com serves: the label is a <b> inside the same span as the date.
 const detail='<span class="job-id job-info"><b>Job ID</b> 2026-19305</span><span class="job-date job-info"><b>Date posted</b> Jun. 09, 2026</span><span class="job-location job-info"><b>Location</b> Austin, Texas</span><div class="ats-description">Work</div>';
 const result=await scrape("arm",{"/search-jobs":listing,"/job/austin":detail});
 assert.equal(result.jobs[0].postedAt,"Jun. 09, 2026");
 assert.equal(result.jobs[0].externalId,"2026-19305")});

// Synopsys is the second Radancy board here. Its own page states how many vacancies matched and
// how many pages they fill, so both the card mapping and that reconciliation are what these cover:
// reading every page the board named is not proof of having read every vacancy it counted.
const radancyPage=(page,pages,total,cards)=>`<section id="search-results" data-total-job-results="${total}" data-total-pages="${pages}" data-current-page="${page}">
<section id="search-results-list"><ul>${cards}</ul></section></section>`;
const radancyCard=(id,title,place,posted)=>`<li class="search-results-list__list-item">
<a class="sr-job-link" href="/job/lisbon/${id}/44408/${id}" data-job-id="${id}"><h2>${title}<img alt="circle arrow"></h2>
<div class="sr-wrapper"><span class="job-location"><img alt="pin icon">${place}</span>
<span class="category"><strong>Category: </strong>Engineering</span>
<span class="job-date-posted"><strong>Posted: </strong>${posted}</span></div></a></li>`;

test("a Radancy board is read from the page's own card markup and dated from the listing",async()=>{
 const listing=radancyPage(1,1,1,radancyCard("100063255168","Digital Design Engineer","Lisbon, Portugal","09/01/2026"));
 const result=await scrape("synopsys",{"/search-jobs":listing,"/job/lisbon":`<div class="ats-description"><p>Verify RTL.</p></div>`});
 assert.equal(result.complete,true);
 assert.equal(result.total,1);
 assert.equal(result.totalExact,true);
 assert.equal(result.jobs.length,1);
 const [job]=result.jobs;
 assert.equal(job.externalId,"100063255168");
 assert.equal(job.title,"Digital Design Engineer");
 assert.equal(job.location,"Lisbon, Portugal");
 assert.equal(job.postedAt,"09/01/2026","the label is stripped and the board's own date kept");
 assert.equal(job.url,"https://careers.synopsys.com/job/lisbon/100063255168/44408/100063255168");
 assert.match(job.description,/Verify RTL/)});

test("a Radancy page count reached with fewer vacancies than the board counted is unfinished",async()=>{
 const listing=radancyPage(1,1,3,radancyCard("1","Analog Design Engineer","Porto, Portugal","09/01/2026"));
 const result=await scrape("synopsys",{"/search-jobs":listing,"/job/lisbon":`<div class="ats-description">Text.</div>`});
 assert.equal(result.complete,false,"one of three vacancies is not a finished board");
 assert.match(result.warnings.join(" "),/listed 3 vacancies but 1 were read/)});

test("a Radancy page that stops publishing its extent is treated as unfinished",async()=>{
 const listing=`<section id="search-results-list"><ul>${radancyCard("2","Verification Engineer","Aveiro, Portugal","09/01/2026")}</ul></section>`;
 const result=await scrape("synopsys",{"/search-jobs":listing,"/job/lisbon":`<div class="ats-description">Text.</div>`});
 assert.equal(result.complete,false);
 assert.match(result.warnings.join(" "),/published no page count/)});
