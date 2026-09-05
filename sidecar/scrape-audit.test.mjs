import assert from "node:assert/strict";
import {createServer} from "node:http";
import {test} from "node:test";
import {runWorker} from "../scripts/worker-proof.mjs";
import {decodeEntities, toIsoDate, toLocation, listingHash, workdayRequisition} from "./worker.mjs";

async function fixture(handler, run) {
 const server=createServer(async(req,res)=>{
  let body="";for await(const part of req)body+=part;
  handler(req,res,JSON.parse(body||"{}"));
 });

 await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
 const source={id:"audit",name:"Fixture",baseUrl:`http://127.0.0.1:${server.address().port}`,
  adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:true,
  configJson:{testNoDelay:true,pageSize:2,maxPages:20,listingPath:"/jobs",tenant:"fixture",site:"External"}};
 try{await run(source)}finally{await new Promise(resolve=>server.close(resolve))}
}
const item=id=>({jobReqId:String(id),title:`Engineer ${id}`,externalPath:`/job/City/Engineer_${id}`});
const json=(res,data)=>res.setHeader("content-type","application/json").end(JSON.stringify(data));

test("Workday URL versions use a corroborated requisition without stripping genuine suffixes",()=>{
 assert.equal(workdayRequisition({externalPath:"/job/City/Engineer_JR2001099-1",bulletFields:["Spotlight Job","JR2001099"]}),"JR2001099");
 assert.equal(workdayRequisition({externalPath:"/job/City/Engineer_R-1",bulletFields:["Spotlight Job"]}),"R-1");
 assert.equal(workdayRequisition({externalPath:"/job/City/Engineer_JR2001099-1",bulletFields:["999"]}),"JR2001099-1");
 assert.equal(workdayRequisition({externalPath:"/job/City/Engineer_JR2001099-1",bulletFields:["JR2001099","JR2001099-1"]}),"JR2001099-1");
});

test("malformed publisher dates and HTML entities do not corrupt dates or crash scraping",()=>{
 assert.equal(toIsoDate("2026-02-30"),null);
 assert.equal(toIsoDate("2026-13-01T00:00:00Z"),null);
 assert.equal(toIsoDate("2024-02-29T12:00:00Z"),"2024-02-29");
 assert.doesNotThrow(()=>decodeEntities("Engineer &#999999999999; &#x110000;"));
 assert.equal(decodeEntities("R&amp;D &#39;chips&#39;"),"R&D 'chips'");
});

test("Workday capped counts use published disjoint categories even on a warm check",async()=>{
 await fixture((req,res)=>json(res,{total:2,jobPostings:[item(1),item(2)],
  facets:[{facetParameter:"jobFamilyGroup",values:[{id:"engineering",count:2},{id:"sales",count:1}]}]}),async source=>{
  source.configJson.splitFacet="jobFamilyGroup";
  const result=await runWorker(source,"check_source");
  assert.equal(result.terminal.payload.boardTotal,3);
  assert.equal(result.terminal.payload.requests,1,"no facet traversal during a warm check");
 });
});

test("category splitting cannot hide jobs omitted from the published categories",async()=>{
 await fixture((req,res,{appliedFacets={}})=>json(res,appliedFacets.jobFamilyGroup?
  {total:2,jobPostings:[item(1),item(2)]}:
  {total:3,jobPostings:[item(1)],facets:[{facetParameter:"jobFamilyGroup",values:[{id:"engineering",count:2}]}]}),async source=>{
  source.configJson={...source.configJson,splitFacet:"jobFamilyGroup",splitThreshold:2};
  const result=await runWorker(source,"scrape_source",{deferDetails:true});
  assert.equal(result.terminal.payload.complete,false);
  assert.equal(result.terminal.payload.boardTotal,3);
  assert.equal(result.terminal.payload.discovered,2);
 });
});

test("short, repeated and drifting Workday pages cannot certify a complete board",async()=>{
 for(const mode of ["short","repeated","drifting"]){
  let requests=0;
  await fixture((req,res,{offset=0})=>{
   requests++;
   json(res,{total:mode==="drifting"&&offset?5:4,
    jobPostings:mode==="short"?[item(1)]:mode==="repeated"?[item(1),item(2)]:[item(offset+1),item(offset+2)]});
  },async source=>{
   const result=await runWorker(source,"scrape_source",{deferDetails:true});
   assert.equal(result.terminal.event,"completed");
   assert.equal(result.terminal.payload.complete,false,mode);
   assert.match(result.terminal.payload.warnings.join(" "),/unfinished|total/i);
   if(mode==="repeated")assert.equal(requests,2,"stop once a page adds no identities");
  });
 }
});

test("facet completeness counts unique sightings including known and filtered jobs",async()=>{
 for(const missing of [true,false]){
  await fixture((req,res,{appliedFacets={}})=>{
   assert.deepEqual(appliedFacets.country,["PT"],"splitting preserves existing source filters");
   const facet=appliedFacets.locations?.[0];
   json(res,facet?{total:2,jobPostings:(facet==="a"||missing?[1,2]:[2,3]).map(item)}:
    {total:3,jobPostings:[item(1)],facets:[{facetParameter:"locations",values:[{id:"a"},{id:"b"}]}]});
  },async original=>{
   const source={...original,configJson:{...original.configJson,splitFacet:"locations",splitThreshold:2,appliedFacets:{country:["PT"]}}};
   const cold=await runWorker(source,"scrape_source",{deferDetails:true});
   assert.equal(cold.terminal.payload.complete,!missing);
   assert.equal(cold.terminal.payload.discovered,missing?2:3,"never substitute the published total for observed rows");
   assert.equal(cold.jobs.length,missing?2:3,"overlapping facets emit each listing once");
   const warm=await runWorker(source,"scrape_source",{known:cold.jobs.map(j=>j.listingHash),deferDetails:true,titleTerms:["engineer 1"]});
   assert.equal(warm.terminal.payload.complete,!missing);
   assert.equal(warm.terminal.payload.discovered,missing?2:3);
   assert.equal(warm.jobs.length,0);
  });
 }
});

test("permanent HTTP errors are not retried; transient HTTP errors still recover",async()=>{
 for(const status of [404,410,422,503]){
  let requests=0;
  await fixture((req,res)=>{
   requests++;
   if(status===503&&requests===2)return json(res,{total:1,jobPostings:[item(1)]});
   res.writeHead(status).end("unavailable");
  },async source=>{
   const result=await runWorker(source,"scrape_source",{deferDetails:true});
   assert.equal(requests,status===503?2:1,String(status));
   assert.equal(result.terminal.event,status===503?"completed":"failed");
  });
 }
});

 test("all locations survive and location reordering preserves listing identity",()=>{
  const places=["USA","France","Germany","Italy","Spain","Japan","Portugal"];
  assert.match(toLocation(places),/Portugal/);
  assert.equal(listingHash("id","Engineer",places),listingHash("id","Engineer",places.toReversed()));
  assert.notEqual(listingHash("id","Engineer",[{name:"Lisbon"}]),listingHash("id","Engineer",[{name:"Porto"}]));
 });

test("empty publisher detail is a failed refresh, never a successful empty replacement",async()=>{
 await fixture((_req,res)=>json(res,{jobPostingInfo:{title:"Engineer",jobDescription:""}}),async source=>{
  const result=await runWorker({...source,enrichmentJobs:[{jobId:"stored",title:"Engineer",company:"Fixture",listingHash:"same",detailUrl:source.baseUrl+"/detail"}]},"enrich_source");
  assert.equal(result.enrichedJobs.length,0);
  assert.ok(result.warnings.some(warning=>/empty description/.test(warning.message)));
 });
});
