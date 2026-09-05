import assert from "node:assert/strict";import * as hashModule from "./worker.mjs";import {createServer}from "node:http";import {spawn}from "node:child_process";import {after,before,test}from "node:test";
let server,base;const seen=[];
before(async()=>{server=createServer(async(req,res)=>{let body="";for await(const part of req)body+=part;seen.push({url:req.url,method:req.method,cookie:req.headers.cookie,body});const parsed=new URL(req.url,"http://local"),adapter=parsed.pathname.split("/")[1];if(req.url.startsWith("/detail/"))return res.setHeader("content-type","application/json").end(JSON.stringify({description:"detail text",applyUrl:"https://apply.example.test/job"}));const query=parsed.searchParams,page=adapter==="workday"?JSON.parse(body||"{}").offset:Number(query.get("page")||0),item={id:`${adapter}-1`,jobReqId:`${adapter}-1`,title:`${adapter} engineer`,company:"Fixture",detailUrl:`/detail/${adapter}`,...(adapter==="workday"?{compensation:{min:120000,max:150000,currency:"USD"}}:adapter==="eightfold"?{salaryMin:90000,salaryMax:110000,salaryCurrency:"USD",seniority:"senior"}:{})};const first=adapter==="eightfold"?!query.get("cursor"):["icims","talentbrew-jibe"].includes(adapter)?page===1:page===0;const list=first?[item]:[];const payload=adapter==="workday"?{total:1,facets:[{facetParameter:"locations",values:[{id:"pt"}]}],jobPostings:list}:adapter==="eightfold"?{positions:list,nextCursor:first?"next":null}:adapter==="phenom"?{jobs:list}:{jobs:list};res.setHeader("content-type","application/json").end(JSON.stringify(payload))});await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));base=`http://127.0.0.1:${server.address().port}`});after(()=>new Promise(resolve=>server.close(resolve)));
function execute(input){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));child.on("error",reject);child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));child.stdin.end(JSON.stringify(input)+"\n")})}
function run(adapter,config){return execute({protocolVersion:1,command:"scrape_source",runId:adapter,source:{name:"Fixture",baseUrl:base,adapterId:adapter,kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:3,pageSize:1,listingPath:`/${adapter}`,query:"rf",sessionCookies:[{name:"session",value:"allowed"}],...config}}})}
test("platform requests execute listing, paging, detail, and cookie contracts",async()=>{for(const adapter of["workday","eightfold","icims","talentbrew-jibe","phenom"]){seen.length=0;const events=await run(adapter,adapter==="workday"?{listingPath:"/workday",tenant:"fixture"}:{}),final=events.at(-1);assert.equal(final.event,"completed",JSON.stringify(events));assert.equal(final.payload.mode,"direct-platform");assert.equal(events.filter(e=>e.event==="job").length,1);const jobPayload=events.find(e=>e.event==="job").payload;if(adapter==="workday"){assert.equal(jobPayload.salaryMin,120000);assert.equal(jobPayload.salaryMax,150000);assert.equal(jobPayload.salaryCurrency,"USD");assert.equal(jobPayload.salaryConfidence,"structured")}else if(adapter==="eightfold"){assert.equal(jobPayload.salaryMin,90000);assert.equal(jobPayload.salaryMax,110000);assert.equal(jobPayload.seniority,"senior")}else{assert.equal(jobPayload.salaryMin,null);assert.equal(jobPayload.salaryMax,null);assert.equal(jobPayload.salaryConfidence,null)}assert.ok(seen.some(x=>x.url===`/detail/${adapter}`&&x.cookie==="session=allowed"));assert.ok(seen.filter(x=>x.url.startsWith(`/${adapter}`)).length>=(adapter==="workday"?1:2),`${adapter}: listing requests`);
  // Workday is the exception: the fixture publishes total 1 and serves 1 row, so the traversal is
  // already over. Asking for the page after the end is what PTC answers with page one again, so
  // not issuing that request is the point of the total-based stop rather than an accident.
  if(adapter==="workday")assert.equal(seen.filter(x=>x.url.startsWith("/workday")).length,1);if(adapter==="workday"){const first=seen.find(x=>x.url==="/workday");assert.equal(first.method,"POST");assert.deepEqual(JSON.parse(first.body),{appliedFacets:{},limit:1,offset:0,searchText:"rf"})}else assert.equal(seen.find(x=>x.url.startsWith(`/${adapter}`)).method,"GET")}});
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

// Hosted APIs cannot be redirected with listingPath. Replace only the transport in a child
// process: the real worker still parses the protocol, enforces robots and normalizes each row.
function hostedRun(adapter,config,routes,extra={}){
 const preload=`const routes=${JSON.stringify(routes)};globalThis.fetch=async raw=>{const url=String(raw);if(url.endsWith('/robots.txt'))return new Response(routes[url]??'User-agent: *\\nAllow: /\\n');if(!(url in routes))throw Error('Unexpected fixture request: '+url);return new Response(JSON.stringify(routes[url]),{headers:{'content-type':'application/json'}})};`;
 return new Promise((resolve,reject)=>{const child=spawn(process.execPath,["--import",`data:text/javascript,${encodeURIComponent(preload)}`,"sidecar/worker.mjs"]);let stdout="",stderr="";
  child.stdout.on("data",data=>stdout+=data);child.stderr.on("data",data=>stderr+=data);child.on("error",reject);child.on("close",code=>code?reject(Error(stderr)):resolve(stdout.trim().split("\n").map(JSON.parse)));
  const {enrichmentJobs,...options}=extra;
  child.stdin.end(JSON.stringify({protocolVersion:1,command:"scrape_source",runId:"hosted",source:{name:"Fixture",adapterId:adapter,baseUrl:"https://careers.example.test/site/",kind:"active",allowPrivateNetwork:true,robotsOverride:false,enrichmentJobs,configJson:{testNoDelay:true,token:"fixture",maxPages:2,...config}},...options})+"\n")});
}
test("Ashby whole-board totals include known and filtered jobs, with empty reads still unfinished",async()=>{
 const url="https://api.ashbyhq.com/posting-api/job-board/fixture",jobs=[{id:"1",title:"Engineer",jobUrl:"https://jobs.example.test/1",location:"Lisbon",descriptionPlain:"Build chips."},{id:"2",title:"Designer",jobUrl:"https://jobs.example.test/2",location:"Porto"}];
 const first=await hostedRun("ashby",{},{[url]:{jobs}}),stored=first.find(event=>event.event==="job")?.payload;
 assert.equal(first.at(-1).event,"completed",JSON.stringify(first.at(-1)));assert.equal(stored.descriptionText,"Build chips.");
 const warm=await hostedRun("ashby",{},{[url]:{jobs}},{known:[stored.listingHash],titleTerms:["Engineer"]});
 assert.equal(warm.at(-1).payload.boardTotal,2);assert.equal(warm.at(-1).payload.boardTotalExact,true);assert.equal(warm.at(-1).payload.complete,true);
 assert.equal(warm.some(event=>event.event==="job"),false);assert.ok(warm.some(event=>event.event==="seen_batch"));
 const empty=await hostedRun("ashby",{},{[url]:{jobs:[]}});assert.equal(empty.at(-1).payload.boardTotal,0);assert.equal(empty.at(-1).payload.complete,false);
 const denied=await hostedRun("ashby",{},{[url]:{jobs},"https://api.ashbyhq.com/robots.txt":"User-agent: *\nDisallow: /\n"});assert.equal(denied.at(-1).payload.code,"robots_denied");
});
test("Lever pages without inventing an exact total from one page",async()=>{
 const item={id:"lever-1",text:"Engineer",hostedUrl:"https://jobs.example.test/1",createdAt:1788307200000,categories:{location:"Toronto"},descriptionPlain:"Build tools."};
 const routes={"https://api.lever.co/v0/postings/fixture?mode=json&limit=1&skip=0":[item],"https://api.lever.co/v0/postings/fixture?mode=json&limit=1&skip=1":[]};
 const events=await hostedRun("lever",{pageSize:1},routes),job=events.find(event=>event.event==="job")?.payload;
 assert.equal(events.at(-1).event,"completed",JSON.stringify(events.at(-1)));assert.equal(job.title,"Engineer");assert.equal(job.externalId,"lever-1");assert.equal(job.location,"Toronto");
 assert.equal(events.at(-1).payload.pages,2);assert.equal(events.at(-1).payload.boardTotalExact,false);assert.equal(events.at(-1).payload.complete,true);
 const partial=await hostedRun("lever",{pageSize:1,maxPages:1},routes);assert.equal(partial.at(-1).payload.complete,false);assert.equal(partial.at(-1).payload.boardTotalExact,false);
});
test("Greenhouse and Oracle keep IDs and normalize inline and deferred descriptions",async()=>{
 const cases=[{adapter:"greenhouse",config:{},listing:"https://boards-api.greenhouse.io/v1/boards/fixture/jobs",detail:"https://boards-api.greenhouse.io/v1/boards/fixture/jobs/7",
  rows:{meta:{total:1},jobs:[{id:7,title:"Engineer",absolute_url:"https://jobs.example.test/7",location:{name:"Berlin"}}]},body:{id:7,title:"Engineer",absolute_url:"https://jobs.example.test/7",content:"<p>Design chips.</p>"}},
 {adapter:"oracle",config:{apiHost:"pod.example.test",siteNumber:"CX",pageSize:200},listing:"https://pod.example.test/hcmRestApi/resources/latest/recruitingCEJobRequisitions?onlyData=true&expand=requisitionList.secondaryLocations&finder=findReqs;siteNumber=CX,limit=200,offset=0,sortBy=POSTING_DATES_DESC",
  detail:"https://pod.example.test/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?onlyData=true&expand=all&finder=ById;Id=%227%22,siteNumber=CX",
  rows:{items:[{TotalJobsCount:1,requisitionList:[{Id:"7",Title:"Engineer",PrimaryLocation:"Paris"}]}]},body:{items:[{Id:"7",Title:"Engineer",ExternalDescriptionStr:"<p>Design chips.</p>"}]}}];
 for(const fixture of cases){const routes={[fixture.listing]:fixture.rows,[fixture.detail]:fixture.body};
  const listed=await hostedRun(fixture.adapter,fixture.config,routes,{deferDetails:true}),job=listed.find(event=>event.event==="job")?.payload;
  assert.equal(listed.at(-1).event,"completed",JSON.stringify(listed.at(-1)));assert.equal(job.externalId,"7",fixture.adapter);assert.equal(job.descriptionStatus,"pending");
  assert.equal(listed.at(-1).payload.boardTotal,1);assert.equal(listed.at(-1).payload.boardTotalExact,true);
  const detail=await hostedRun(fixture.adapter,fixture.config,routes,{command:"enrich_source",enrichmentJobs:[{...job,jobId:"stored"}]}),enriched=detail.find(event=>event.event==="enriched_job")?.payload;
  assert.equal(enriched?.jobId,"stored",JSON.stringify(detail));assert.equal(enriched.descriptionText,"Design chips.",fixture.adapter);
  const inline=await hostedRun(fixture.adapter,fixture.config,routes);assert.equal(inline.find(event=>event.event==="job")?.payload.descriptionText,"Design chips.",fixture.adapter);
 }
});

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
 assert.equal(toLocation(platformLocation({location:"Sibiu",additionalLocations:["Caen"]},"/job/Sibiu/x_R-1")),"Caen, Sibiu");
 // Listing-only read: the count is all the row carries, so the city comes from its own URL and the
 // remaining offices are counted rather than lost.
 assert.equal(platformLocation({locationsText:"2 Locations"},"/job/Sibiu/Crypto_R-1"),"Sibiu (+1 more)");
 // Nothing recoverable: the count is kept, because inventing a place would be worse.
 assert.equal(platformLocation({locationsText:"2 Locations"},"/careers/opening/5"),"2 Locations");
 // Every listed place survives, de-duplicated, with a long list trimmed and the rest counted.
 assert.equal(toLocation(["Austin","Austin","Munich"]),"Austin, Munich");
 assert.equal(toLocation(["a","b","c","d","e","f","g","h"]),"a, b, c, d, e, f, g, h")});

test("Eightfold PCSX defers real detail payloads and refuses repeated or drifting pagination",async()=>{
 let mode="normal";const requested=[];
 const fixture=createServer((req,res)=>{
  const url=new URL(req.url,"http://fixture");requested.push(url.pathname);
  res.setHeader("content-type","application/json");
  if(url.pathname==="/api/pcsx/position_details")return res.end(JSON.stringify({data:{id:37,name:"Process Engineer",jobDescription:"<p>Develop deposition equipment.</p>"}}));
  const start=Number(url.searchParams.get("start"));
  res.end(JSON.stringify({data:{count:mode==="normal"?1:mode==="drifting"&&start?3:2,
   positions:start>1?[]:[{id:mode==="drifting"?37+start:37,name:"Process Engineer",positionUrl:`/careers/job/${mode==="drifting"?37+start:37}`,locations:["Phoenix"]}]}}));
 });
 await new Promise(resolve=>fixture.listen(0,"127.0.0.1",resolve));
 try{
  const baseUrl=`http://127.0.0.1:${fixture.address().port}`;
  const source={name:"Fixture",baseUrl,adapterId:"eightfold",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{domain:"fixture.com",eightfoldApi:"pcsx",testNoDelay:true,maxPages:5}};
  const events=await execute({protocolVersion:1,command:"scrape_source",runId:"pcsx",source,deferDetails:true});
  const listing=events.find(event=>event.event==="job").payload;
  assert.equal(events.at(-1).payload.complete,true);assert.equal(listing.descriptionStatus,"pending");
  assert.match(listing.detailUrl,/position_details\?position_id=37&domain=fixture.com/);
  assert.ok(!requested.includes("/api/pcsx/position_details"));
  const details=await execute({protocolVersion:1,command:"enrich_source",runId:"pcsx-detail",source:{...source,enrichmentJobs:[{...listing,jobId:"stored"}]}});
  const detail=details.find(event=>event.event==="enriched_job").payload;
  assert.equal(detail.jobId,"stored");assert.equal(detail.externalId,"37");assert.equal(detail.descriptionStatus,"complete");
  assert.equal(detail.descriptionText,"Develop deposition equipment.");assert.equal(detail.listingHash,listing.listingHash);
  for(mode of["repeated","drifting"]){
   const partial=await execute({protocolVersion:1,command:"scrape_source",runId:mode,source,deferDetails:true});
   assert.equal(partial.at(-1).payload.complete,false,mode);
   assert.match(partial.at(-1).payload.warnings.join(" "),/unfinished/,mode);
  }
 }finally{await new Promise(resolve=>fixture.close(resolve))}
});

// TEKEVER's careers page counts 123 openings; its RSS feed carries the most recent 100. Reaching
// the end of that feed used to be reported as a complete board, which would have reconciled the 23
// it never mentioned to closed. A feed states no total, so it cannot certify a whole board.
test("a feed read is not a complete board unless the source says its feed is the whole board",async()=>{
 const item=n=>`<item><title>Engineer ${n}</title><link>https://example.test/jobs/${n}</link><guid>${n}</guid><description>Work.</description></item>`;
 const board=createServer((req,res)=>res.setHeader("content-type","application/rss+xml")
  .end(`<?xml version="1.0"?><rss version="2.0"><channel><title>Feed</title>${[1,2,3].map(item).join("")}</channel></rss>`));
 await new Promise(resolve=>board.listen(0,"127.0.0.1",resolve));
 const host=`http://127.0.0.1:${board.address().port}/jobs.rss`;
 const read=configJson=>execute({protocolVersion:1,command:"scrape_source",runId:"feed",
  source:{name:"Fixture",baseUrl:host,adapterId:"rss",kind:"active",allowPrivateNetwork:true,robotsOverride:true,
   configJson:{testNoDelay:true,...configJson}}});
 try{
  const capped=(await read({})).at(-1).payload;
  assert.equal(capped.complete,false,"the end of a feed is not the end of a board");
  assert.equal(capped.boardTotalExact,false);
  assert.match(capped.warnings.join(" "),/feed lists recent items/);
  const declared=(await read({feedIsWholeBoard:true})).at(-1).payload;
  assert.equal(declared.complete,true,"a source may declare that its feed carries every opening");
  assert.equal(declared.discovered,3);
 }finally{await new Promise(resolve=>board.close(resolve))}
});
