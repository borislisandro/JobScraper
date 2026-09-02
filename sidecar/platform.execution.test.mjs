import assert from "node:assert/strict";import * as hashModule from "./worker.mjs";import {createServer}from "node:http";import {spawn}from "node:child_process";import {after,before,test}from "node:test";
let server,base;const seen=[];
before(async()=>{server=createServer(async(req,res)=>{let body="";for await(const part of req)body+=part;seen.push({url:req.url,method:req.method,cookie:req.headers.cookie,body});const parsed=new URL(req.url,"http://local"),adapter=parsed.pathname.split("/")[1];if(req.url.startsWith("/detail/"))return res.setHeader("content-type","application/json").end(JSON.stringify({description:"detail text",applyUrl:"https://apply.example.test/job"}));const query=parsed.searchParams,page=adapter==="workday"?JSON.parse(body||"{}").offset:Number(query.get("page")||0),item={id:`${adapter}-1`,jobReqId:`${adapter}-1`,title:`${adapter} engineer`,company:"Fixture",detailUrl:`/detail/${adapter}`,...(adapter==="workday"?{compensation:{min:120000,max:150000,currency:"USD"}}:adapter==="eightfold"?{salaryMin:90000,salaryMax:110000,salaryCurrency:"USD",seniority:"senior"}:{})};const first=adapter==="eightfold"?!query.get("cursor"):["icims","talentbrew-jibe"].includes(adapter)?page===1:page===0;const list=first?[item]:[];const payload=adapter==="workday"?{total:1,facets:[{facetParameter:"locations",values:[{id:"pt"}]}],jobPostings:list}:adapter==="eightfold"?{positions:list,nextCursor:first?"next":null}:adapter==="phenom"?{jobs:list}:{jobs:list};res.setHeader("content-type","application/json").end(JSON.stringify(payload))});await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));base=`http://127.0.0.1:${server.address().port}`});after(()=>new Promise(resolve=>server.close(resolve)));
function execute(input){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));child.on("error",reject);child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));child.stdin.end(JSON.stringify(input)+"\n")})}
function run(adapter,config){return execute({protocolVersion:1,command:"scrape_source",runId:adapter,source:{name:"Fixture",baseUrl:base,adapterId:adapter,kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:3,pageSize:1,listingPath:`/${adapter}`,query:"rf",sessionCookies:[{name:"session",value:"allowed"}],...config}}})}
test("platform requests execute listing, paging, detail, and cookie contracts",async()=>{for(const adapter of["workday","eightfold","icims","talentbrew-jibe","phenom"]){seen.length=0;const events=await run(adapter,adapter==="workday"?{listingPath:"/workday",tenant:"fixture"}:{}),final=events.at(-1);assert.equal(final.event,"completed",JSON.stringify(events));assert.equal(final.payload.mode,"direct-platform");assert.equal(events.filter(e=>e.event==="job").length,1);const jobPayload=events.find(e=>e.event==="job").payload;if(adapter==="workday"){assert.equal(jobPayload.salaryMin,120000);assert.equal(jobPayload.salaryMax,150000);assert.equal(jobPayload.salaryCurrency,"USD");assert.equal(jobPayload.salaryConfidence,"structured")}else if(adapter==="eightfold"){assert.equal(jobPayload.salaryMin,90000);assert.equal(jobPayload.salaryMax,110000);assert.equal(jobPayload.seniority,"senior")}else{assert.equal(jobPayload.salaryMin,null);assert.equal(jobPayload.salaryMax,null);assert.equal(jobPayload.salaryConfidence,null)}assert.ok(seen.some(x=>x.url===`/detail/${adapter}`&&x.cookie==="session=allowed"));assert.ok(seen.filter(x=>x.url.startsWith(`/${adapter}`)).length>=2);if(adapter==="workday"){const first=seen.find(x=>x.url==="/workday");assert.equal(first.method,"POST");assert.deepEqual(JSON.parse(first.body),{appliedFacets:{},limit:1,offset:0,searchText:"rf"})}else assert.equal(seen.find(x=>x.url.startsWith(`/${adapter}`)).method,"GET")}});
test("Workday listing reads defer detail enrichment until one job is requested",async()=>{
 seen.length=0;const source={id:"s",name:"Fixture",baseUrl:base,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:1,pageSize:1,listingPath:"/workday",tenant:"fixture",site:"External"}};
 const listingEvents=await execute({protocolVersion:1,command:"scrape_source",runId:"listing",deferDetails:true,source});
 const listing=listingEvents.find(event=>event.event==="job")?.payload;
 assert.equal(listing.descriptionStatus,"pending");assert.equal(listing.detailUrl,`${base}/detail/workday`);
 assert.equal(seen.some(request=>request.url==="/detail/workday"),false,"listing phase issues no detail request");
 seen.length=0;const enriched=await execute({protocolVersion:1,command:"enrich_source",runId:"detail",source:{...source,enrichmentJobs:[{...listing,jobId:"stored-id"}]}});
 const detail=enriched.find(event=>event.event==="enriched_job")?.payload;
 assert.equal(detail.jobId,"stored-id");assert.equal(detail.descriptionStatus,"complete");assert.equal(detail.descriptionText,"detail text");
 assert.equal(seen.filter(request=>request.url==="/detail/workday").length,1)
});
test("missing Workday tenant/path and partial traversal are explicit",async()=>{const events=await run("workday",{listingPath:undefined,tenant:undefined});assert.equal(events.at(-1).payload.code,"incomplete");const partial=await run("icims",{maxPages:1});assert.equal(partial.at(-1).payload.complete,false)});

// "Is there anything new?" has to be answerable without reading the whole board. The check reads
// page one, reports what the board says it holds, and counts how many of those listings are not
// already stored — so a source with nothing new costs one request instead of a full traversal.
test("the change check reads one page and reports the board total and what is new",async()=>{
  seen.length=0;
  const request=(known)=>new Promise((resolve,reject)=>{
    const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
    child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));
    child.on("error",reject);
    child.on("close",()=>resolve(lines.map(JSON.parse)));
    child.stdin.end(JSON.stringify({protocolVersion:1,command:"check_source",runId:"check",known,
      source:{name:"Fixture",baseUrl:base,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,
        configJson:{testNoDelay:true,listingPath:"/workday",tenant:"fixture",site:"External",pageSize:1}}})+"\n")});

  const cold=(await request([])).at(-1);
  assert.equal(cold.event,"completed");
  assert.equal(cold.payload.mode,"check");
  assert.equal(cold.payload.boardTotal,1);
  assert.equal(cold.payload.boardTotalExact,true);
  assert.equal(cold.payload.fresh,1,"nothing is stored yet, so page one is entirely new");
  assert.ok(cold.payload.requests>=1&&cold.payload.requests<=3,`one page, not a traversal: ${cold.payload.requests}`);
  const pageOneRequests=cold.payload.requests;

  // Re-run knowing exactly what page one holds: the board is unchanged and nothing is new.
  const hash=require_hash();
  const warm=(await request([hash])).at(-1);
  assert.equal(warm.payload.fresh,0,"a page of listings already stored reports nothing new");
  assert.equal(warm.payload.requests,pageOneRequests,"the check costs the same either way");
});

test("a Workday check ignores facet splitting and never fetches details",async()=>{
 seen.length=0;
 const events=await new Promise((resolve,reject)=>{
  const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
  child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));
  child.on("error",reject);child.on("close",()=>resolve(lines.map(JSON.parse)));
  child.stdin.end(JSON.stringify({protocolVersion:1,command:"check_source",runId:"facet-check",source:{name:"Fixture",baseUrl:base,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,listingPath:"/workday",tenant:"fixture",site:"External",pageSize:1,splitFacet:"locations",splitThreshold:0}}})+"\n");
 });
 assert.equal(events.at(-1).event,"completed");
 assert.equal(seen.filter(request=>request.url==="/workday").length,1);
 assert.equal(seen.filter(request=>request.url.startsWith("/detail/")).length,0);
});

test("title filtering and known hashes avoid Workday detail requests before normalization",async()=>{
 const execute=(known,titleTerms)=>new Promise((resolve,reject)=>{
  const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
  child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));
  child.on("error",reject);child.on("close",()=>resolve(lines.map(JSON.parse)));
  child.stdin.end(JSON.stringify({protocolVersion:1,command:"scrape_source",runId:"warm",known,titleTerms,source:{name:"Fixture",baseUrl:base,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:1,listingPath:"/workday",tenant:"fixture",site:"External",pageSize:1}}})+"\n");
 });
 seen.length=0;
 const filtered=await execute([], ["firmware"]);
 assert.equal(filtered.at(-1).payload.filtered,1);
 assert.equal(filtered.some(event=>event.event==="job"),false);
 assert.equal(seen.some(request=>request.url.startsWith("/detail/")),false);
 seen.length=0;
 const warm=await execute([require_hash()], []);
 assert.equal(warm.some(event=>event.event==="seen_batch"),true);
 assert.equal(warm.some(event=>event.event==="job"),false);
 assert.equal(seen.some(request=>request.url.startsWith("/detail/")),false);
});
// The fixture's listing row, hashed the way the worker hashes it.
function require_hash(){
  const {listingHash}=hashModule;
  return listingHash("/detail/workday","workday engineer","");
}

// Workday publishes the posting date as prose that changes every day ("Posted 5 Days Ago" becomes
// "Posted 6 Days Ago" tomorrow) while the vacancy itself is untouched. When that string was part
// of the listing hash, no run ever recognised a listing it had already stored: every update
// re-read the whole board, and because an unrecognised listing is persisted as
// description_status='pending', every description was downloaded again with it.
test("a listing whose only change is a relative posting date is still recognised the next day",async()=>{
 let postedOn="Posted 5 Days Ago";
 const board=createServer((req,res)=>{
  seen.push({url:req.url});
  if(req.url.startsWith("/detail/"))return res.setHeader("content-type","application/json").end(JSON.stringify({description:"detail text"}));
  res.setHeader("content-type","application/json").end(JSON.stringify({total:1,jobPostings:[
   {title:"Silicon Validation Engineer",externalPath:"/job/Ireland-Leixlip/Silicon-Validation-Engineer_JR0284458",
    locationsText:"Ireland, Leixlip",postedOn,detailUrl:"/detail/workday",jobReqId:"JR0284458"}]}));
 });
 await new Promise(resolve=>board.listen(0,"127.0.0.1",resolve));
 const host=`http://127.0.0.1:${board.address().port}`;
 const readBoard=known=>execute({protocolVersion:1,command:"scrape_source",runId:`posted-${postedOn}`,known,
  source:{name:"Fixture",baseUrl:host,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,
   configJson:{testNoDelay:true,maxPages:1,pageSize:1,listingPath:"/workday",tenant:"fixture",site:"External"}}});
 try{
  const today=await readBoard([]);
  assert.equal(today.at(-1).event,"completed",JSON.stringify(today.at(-1)));
  const stored=today.find(event=>event.event==="job")?.payload;
  assert.ok(stored?.listingHash,"the first run stores a hash for the listing");

  // The next day: same vacancy, same board, one day older in the only field Workday publishes.
  postedOn="Posted 6 Days Ago";
  seen.length=0;
  const tomorrow=await readBoard([stored.listingHash]);
  assert.equal(tomorrow.at(-1).event,"completed",JSON.stringify(tomorrow.at(-1)));
  assert.equal(tomorrow.some(event=>event.event==="job"),false,"the same vacancy is not re-read a day later");
  assert.equal(tomorrow.some(event=>event.event==="seen_batch"),true,"it is reported as still on the board");
  assert.equal(seen.some(request=>request.url.startsWith("/detail/")),false,"and its description is not downloaded again");
  assert.equal(tomorrow.at(-1).payload.skipped,1);
 }finally{await new Promise(resolve=>board.close(resolve))}
});

// A Workday listing row has no requisition id field: bulletFields carries it, but a promoted
// posting reads ["Spotlight Job","JR0284458"], so the old bulletFields[0] fallback gave every
// spotlight-flagged posting on a board the same external id. Rows are matched on
// (source_id, external_id), so those postings all collapsed onto one database row and overwrote
// each other. Only a listings-only update reaches this fallback — a detail fetch supplies jobReqId.
test("spotlight-flagged Workday postings keep their own requisition ids on a listings-only read",async()=>{
 const board=createServer((req,res)=>{
  seen.push({url:req.url});
  res.setHeader("content-type","application/json").end(JSON.stringify({total:3,jobPostings:[
   // Two promoted postings: the label sits in front of the requisition id in both.
   {title:"Manufacturing Technician",externalPath:"/job/Ireland-Leixlip/Manufacturing-Technician_JR0284458",
    locationsText:"Ireland, Leixlip",postedOn:"Posted 5 Days Ago",bulletFields:["Spotlight Job","JR0284458"]},
   {title:"Early Careers Technician",externalPath:"/job/Ireland-Leixlip/Early-Careers-Technician_JR0284021",
    locationsText:"Ireland, Leixlip",postedOn:"Posted 5 Days Ago",bulletFields:["Spotlight Job","JR0284021"]},
   // An unpromoted one, and a hyphenated requisition shape (Microchip publishes "R3629-26").
   {title:"Applications Engineer",externalPath:"/job/Japan---Tokyo/Applications-Engineer_R3629-26",
    locationsText:"Japan - Tokyo",postedOn:"Posted Today",bulletFields:["R3629-26"]},
  ]}));
 });
 await new Promise(resolve=>board.listen(0,"127.0.0.1",resolve));
 const host=`http://127.0.0.1:${board.address().port}`;
 try{
  const events=await execute({protocolVersion:1,command:"scrape_source",runId:"spotlight",known:[],deferDetails:true,
   source:{name:"Fixture",baseUrl:host,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,
    configJson:{testNoDelay:true,maxPages:1,pageSize:3,listingPath:"/workday",tenant:"fixture",site:"External"}}});
  assert.equal(events.at(-1).event,"completed",JSON.stringify(events.at(-1)));
  const jobs=events.filter(event=>event.event==="job").map(event=>event.payload);
  assert.equal(jobs.length,3);
  assert.equal(jobs.filter(job=>job.externalId==="Spotlight Job").length,0,"the promoted label is never an identity");
  assert.deepEqual(jobs.map(job=>job.externalId),["JR0284458","JR0284021","R3629-26"]);
  assert.equal(new Set(jobs.map(job=>job.externalId)).size,3,"two spotlight postings do not share one row");
 }finally{await new Promise(resolve=>board.close(resolve))}
});

// A Workday posting open in more than one office publishes the count where the place belongs, and
// keeps the real offices in the detail payload. Storing the count verbatim put nearly a fifth of a
// board beyond the reach of any location filter.
test("a multi-office posting stores places, not a count",()=>{
 const {platformLocation,pathCity,locationCount,toLocation}=hashModule;
 assert.equal(locationCount("2 Locations"),"2");
 assert.equal(locationCount("11 locations"),"11");
 assert.equal(locationCount("Sibiu"),null);
 assert.equal(pathCity("/job/Sibiu/Crypto-Quality-SW-Process-Internship_R-10066282"),"Sibiu");
 assert.equal(pathCity("/job/Sophia-Antipolis/Analog-Designer_R-1"),"Sophia Antipolis");
 // Some tenants put the requisition in that slot; a number is not an office.
 assert.equal(pathCity("/job/R-10066282/Something_R-1"),null);
 // Detail payload: the primary office and its siblings, and never the count beside them.
 assert.deepEqual(
  platformLocation({location:"Sibiu",additionalLocations:["Caen","Bucharest"],locationsText:"3 Locations"},"/job/Sibiu/x_R-1"),
  ["Sibiu","Caen","Bucharest"]);
 assert.equal(toLocation(platformLocation({location:"Sibiu",additionalLocations:["Caen"]},"/job/Sibiu/x_R-1")),"Sibiu, Caen");
 // Listing-only read: the count is all the row carries, so the city comes from its own URL and the
 // remaining offices are counted rather than lost.
 assert.equal(platformLocation({locationsText:"2 Locations"},"/job/Sibiu/Crypto_R-1"),"Sibiu (+1 more)");
 // Nothing recoverable: the count is kept, because inventing a place would be worse.
 assert.equal(platformLocation({locationsText:"2 Locations"},"/careers/opening/5"),"2 Locations");
 // Every listed place survives, de-duplicated, with a long list trimmed and the rest counted.
 assert.equal(toLocation(["Austin","Austin","Munich"]),"Austin, Munich");
 assert.equal(toLocation(["a","b","c","d","e","f","g","h"]),"a, b, c, d, e, f (+2 more)")});
