import assert from "node:assert/strict";import {createServer}from "node:http";import {existsSync}from "node:fs";import {spawn}from "node:child_process";import {after,before,test}from "node:test";
import {inferJsonMapping,jsonArrays} from "./worker.mjs";
// A JS-only board is read from the JSON endpoint its page loads. Inference is pure and runs on the
// captured payload; these fixtures are the shapes that decide whether a payload is a job list at
// all, and the end-to-end case proves the generated config actually scrapes.
const jobsPayload={meta:{count:3},data:{jobs:[
 {title:"Compiler Engineer",absolute_url:"/careers/job/1",id:8801,location:{name:"Austin, TX"}},
 {title:"Verification Lead",absolute_url:"/careers/job/2",id:8802,location:{name:"Cambridge, UK"}},
 {title:"Systems Architect",absolute_url:"/careers/job/3",id:8803,location:{name:"Remote"}}]},
 nav:[{name:"Home page",href:"/"},{name:"About us",href:"/about"},{name:"Contact us",href:"/contact"}]};

test("the job array is found under its own path, not the nav menu beside it",()=>{
 const inferred=inferJsonMapping(jobsPayload);
 assert.equal(inferred.itemsPath,"data.jobs");
 assert.deepEqual(inferred.fieldMap,{title:"title",url:"absolute_url",externalId:"id",location:"location"})});

test("a title and a link alone are a menu, not a board",()=>{
 assert.equal(inferJsonMapping({nav:jobsPayload.nav}),null);
 // The same rows gain a posting date and become believable.
 const dated=jobsPayload.nav.map((item,i)=>({...item,posted_at:`2026-08-0${i+1}`}));
 assert.equal(inferJsonMapping({nav:dated})?.itemsPath,"nav")});

test("a payload that is itself the array maps to an empty items path",()=>{
 const inferred=inferJsonMapping([
  {name:"Staff Engineer",url:"https://example.invalid/1",requisitionId:"R1",postedDate:"2026-08-01"},
  {name:"Data Scientist",url:"https://example.invalid/2",requisitionId:"R2",postedDate:"2026-08-02"},
  {name:"Product Manager",url:"https://example.invalid/3",requisitionId:"R3",postedDate:"2026-08-03"}]);
 assert.equal(inferred.itemsPath,"");
 assert.equal(inferred.fieldMap.title,"name")});

test("a key whose values do not match its name is not believed",()=>{
 // "title" holding a URL is how a media payload advertises a thumbnail; nothing usable is offered.
 assert.equal(inferJsonMapping({items:[
  {title:"https://cdn.example.invalid/1.png",url:"/x/1",id:1},
  {title:"https://cdn.example.invalid/2.png",url:"/x/2",id:2},
  {title:"https://cdn.example.invalid/3.png",url:"/x/3",id:3}]}),null)});

test("array discovery is bounded and survives a self-referential payload",()=>{
 const loop={items:[{title:"a"},{title:"b"},{title:"c"}]};loop.self=loop;
 assert.equal(jsonArrays(loop).length,1)});

// End to end: the generated config is what a saved source runs, so it is run here.
let server,base;
const payload=JSON.stringify(jobsPayload);
before(async()=>{server=createServer((_,res)=>res.setHeader("content-type","application/json").end(payload));
 await new Promise(r=>server.listen(0,"127.0.0.1",r));base=`http://127.0.0.1:${server.address().port}/api/search`});
after(()=>new Promise(r=>server.close(r)));
// Probes a URL end to end, the way saving a new source does.
const probe=url=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
 child.stdout.on("data",d=>lines.push(...String(d).trim().split("\n").filter(Boolean)));child.on("error",reject);
 child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));
 child.stdin.end(`${JSON.stringify({protocolVersion:1,command:"probe_source",runId:"json-probe",source:{name:"Fixture",baseUrl:url,adapterId:"static-css",kind:"active",allowPrivateNetwork:true,configJson:{testNoDelay:true,maxPages:1}}})}\n`)});
const run=configJson=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
 child.stdout.on("data",d=>lines.push(...String(d).trim().split("\n").filter(Boolean)));child.on("error",reject);
 child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));
 child.stdin.end(`${JSON.stringify({protocolVersion:1,command:"test_source",runId:"json-inference",source:{name:"Fixture",baseUrl:base,adapterId:"json",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:1,...configJson}}})}\n`)});

test("the inferred config scrapes the endpoint it was inferred from",async()=>{
 const {itemsPath,fieldMap}=inferJsonMapping(jobsPayload);
 const jobs=(await run({itemsPath,fieldMap})).filter(e=>e.event==="job").map(e=>e.payload);
 assert.equal(jobs.length,3);
 assert.equal(jobs[0].title,"Compiler Engineer");
 assert.equal(jobs[0].location,"Austin, TX");
 assert.equal(jobs[0].externalId,"8801");
 // The endpoint's own address is the base a relative listing link resolves against.
 assert.equal(jobs[0].applyUrl,new URL("/careers/job/1",base).toString())});

// The capture path itself: an empty shell that builds its list from XHR is exactly the case
// selector inference cannot read, and the part most easily broken (response bodies have to be read
// before the browser closes). Skipped where Edge is not installed, which is the only browser the
// worker will drive.
const edge=[process.env.ProgramFiles&&`${process.env.ProgramFiles}/Microsoft/Edge/Application/msedge.exe`,process.env["ProgramFiles(x86)"]&&`${process.env["ProgramFiles(x86)"]}/Microsoft/Edge/Application/msedge.exe`].filter(Boolean).find(p=>existsSync(p));
test("a JS-only board is detected from the endpoint its page fetches",{skip:edge?false:"Microsoft Edge is not installed"},async()=>{
 const shell=`<html><body><main id="list">Loading…</main><script>fetch("/api/search?start=0").then(r=>r.json()).then(d=>{document.getElementById("list").textContent=d.data.jobs.length})</script></body></html>`;
 const site=createServer((req,res)=>{
  if(req.url.startsWith("/api/search"))return res.setHeader("content-type","application/json").end(payload);
  if(req.url==="/robots.txt")return res.setHeader("content-type","text/plain").end("User-agent: *\nAllow: /\n");
  res.setHeader("content-type","text/html").end(shell)});
 await new Promise(r=>site.listen(0,"127.0.0.1",r));
 const url=`http://127.0.0.1:${site.address().port}/careers`;
 try{
  const events=await probe(url),final=events.at(-1);
  assert.equal(final.event,"completed",JSON.stringify(final));
  assert.equal(final.payload.recommendedAdapter,"json");
  assert.equal(final.payload.detectedConfig.itemsPath,"data.jobs");
  assert.equal(final.payload.detectedConfig.fieldMap.title,"title");
  // The saved source points at the endpoint, not at the page, so later runs need no browser.
  assert.match(final.payload.finalUrl,/\/api\/search/);
  assert.equal(final.payload.sampleJobs.length,3)}
 finally{await new Promise(r=>site.close(r))}});

// A referral or a recruiter's link is one opening on a board nobody configured, and configuring a
// whole source for a single job is the wrong trade. Schema.org JobPosting is what almost every ATS
// already publishes for Google Jobs, so it is the one thing worth trusting on an unknown page.
test("one vacancy can be read from its own page",async()=>{
 const {captureFromHtml}=await import("./worker.mjs");
 const posting={"@context":"https://schema.org","@type":"JobPosting",title:"Verification Engineer",
  datePosted:"2026-08-21",validThrough:"2026-10-01",hiringOrganization:{name:"Critical Software"},
  jobLocation:{"@type":"Place",address:{addressLocality:"Coimbra",addressCountry:"Portugal"}},
  description:"<p>UVM and SystemVerilog</p>"};
 const page=`<html><head><title>Careers</title><script type="application/ld+json">${JSON.stringify(posting)}</script></head><body></body></html>`;
 const read=captureFromHtml(page,"https://criticalsoftware.test/jobs/9");
 assert.equal(read.title,"Verification Engineer");
 assert.equal(read.company,"Critical Software");
 assert.equal(read.location,"Coimbra, Portugal");
 assert.equal(read.postedAt,"2026-08-21");
 assert.equal(read.closingAt,"2026-10-01");
 assert.equal(read.descriptionText,"UVM and SystemVerilog");
 assert.equal(read.structured,true);
 // Graphs and arrays are both normal ways to ship it.
 const graph=`<script type="application/ld+json">${JSON.stringify({"@graph":[{"@type":"WebPage"},posting]})}</script>`;
 assert.equal(captureFromHtml(graph,"https://x.test/1").title,"Verification Engineer");
 // A page with no markup still yields something, and says it is not to be trusted — the person who
 // pasted the link is the one who can fix it before it is stored.
 const bare=captureFromHtml('<html><head><title>Staff Engineer | Foo</title><meta property="og:site_name" content="Foo"></head></html>',"https://foo.test/2");
 assert.equal(bare.structured,false);
 assert.equal(bare.title,"Staff Engineer | Foo");
 assert.equal(bare.company,"Foo");
 // A malformed block does not take the page down with it.
 const broken=`<script type="application/ld+json">{ not json </script><script type="application/ld+json">${JSON.stringify(posting)}</script>`;
 assert.equal(captureFromHtml(broken,"https://x.test/3").title,"Verification Engineer");
 // Remote-only postings say so in a field of their own.
 const remote=captureFromHtml(`<script type="application/ld+json">${JSON.stringify({...posting,jobLocation:undefined,jobLocationType:"TELECOMMUTE"})}</script>`,"https://x.test/4");
 assert.equal(remote.workMode,"remote");
 assert.equal(remote.location,"Remote")});
