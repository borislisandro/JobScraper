import assert from "node:assert/strict";import {createServer}from "node:http";import {toIsoDate}from "./worker.mjs";import {spawn}from "node:child_process";import {test,before,after}from "node:test";
let server,base;
before(async()=>{server=createServer((req,res)=>{if(req.url==="/robots.txt")return res.end("User-agent: *\nAllow: /\n");if(req.url==="/eightfold-robots.txt")return res.end("User-agent: *\nDisallow: /\nAllow: /$\nAllow: /careers\nAllow: /api/career_hub\n");if(req.url==="/blocked/robots.txt")return res.end("User-agent: *\nDisallow: /blocked\n");if(req.url==="/board")return res.end('<nav><a href="/about">About us</a></nav><ul><li class="job-card"><a class="job-card__title" href="/job/0">Firmware Engineer 0</a><span class="location">Lisbon</span></li><li class="job-card"><a class="job-card__title" href="/job/1">Firmware Engineer 1</a><span class="location">Lisbon</span></li><li class="job-card"><a class="job-card__title" href="/job/2">Firmware Engineer 2</a><span class="location">Lisbon</span></li><li class="job-card"><a class="job-card__title" href="/job/3">Firmware Engineer 3</a><span class="location">Lisbon</span></li></ul>');if(req.url==="/static")return res.end('<article class="job"><h2>Firmware Engineer</h2><a href="/apply">Apply</a></article>');if(req.url==="/xpath")return res.end('<article class="job"><h2>XPath Engineer</h2><a href="/apply">Apply</a></article>');if(req.url==="/page1")return res.end('<article class="job"><h2>Page One</h2></article><a class="next" href="/page2">Next</a>');if(req.url==="/page2")return res.end('<article class="job"><h2>Page Two</h2></article>');if(req.url==="/json")return res.setHeader("content-type","application/json").end(JSON.stringify({jobs:[{id:"j1",title:"Systems Engineer",company:"Example"}]}));if(req.url==="/workday")return res.setHeader("content-type","application/json").end(JSON.stringify({jobPostings:[{jobReqId:"wd1",title:"Workday Engineer",externalPath:"/wd"}]}));if(req.url==="/rss")return res.setHeader("content-type","application/rss+xml").end('<rss><channel><item><guid>r1</guid><title>Embedded Engineer</title><link>https://example.test/apply</link><description>Rust</description></item></channel></rss>');res.writeHead(429,{"retry-after":"1"}).end("slow down")});await new Promise(r=>server.listen(0,"127.0.0.1",r));base=`http://127.0.0.1:${server.address().port}`});after(()=>new Promise(r=>server.close(r)));
function run(path,configJson={},adapterId="static-css",extra={}){return new Promise((resolve,reject)=>{const c=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),out=[];let err="";c.stdout.on("data",d=>out.push(...String(d).trim().split("\n").filter(Boolean)));c.stderr.on("data",d=>err+=d);c.on("error",reject);c.on("close",code=>code?reject(Error(err)):resolve(out.map(JSON.parse)));c.stdin.end(JSON.stringify({protocolVersion:1,command:"test_source",runId:"test",source:{name:"Fixture",baseUrl:base+path,adapterId,kind:"active",allowPrivateNetwork:true,configJson:{testNoDelay:true,...configJson},...extra}})+"\n")})}
const event=(events,name)=>events.find(e=>e.event===name);
test("CSS and XPath mappings emit normalized jobs",async()=>{const css=await run("/static",{itemSelector:".job",titleSelector:"h2",urlSelector:"a[href]"}),xpath=await run("/xpath",{itemXPath:"//article[@class='job']",titleXPath:"//h2",urlXPath:"//a"},"static-xpath");assert.equal(event(css,"job").payload.title,"Firmware Engineer");assert.equal(event(xpath,"job")?.payload.title,"XPath Engineer",JSON.stringify(xpath))});
test("preview reports mode, timing, and missing selector warnings without persistence",async()=>{const preview=await run("/static",{},"static-css"),final=preview.at(-1);assert.equal(final.payload.persisted,0);assert.equal(final.payload.mode,"direct");assert.equal(typeof final.payload.timingMs,"number");assert.match(final.payload.warnings.join(" "),/itemSelector/)});
test("JSON, RSS, and platform direct adapters parse deterministic shapes",async()=>{const json=await run("/json",{mode:"json"},"json"),rss=await run("/rss",{mode:"rss"},"rss"),wd=await run("/workday",{listingPath:"/workday"},"workday");assert.equal(event(json,"job").payload.company,"Example");assert.equal(event(rss,"job").payload.title,"Embedded Engineer");assert.equal(event(wd,"job").payload.externalId,"wd1")});
test("test preview stops after one page instead of scraping the whole board",async()=>{const x=await run("/page1",{itemSelector:".job",titleSelector:"h2",nextSelector:".next"});assert.equal(x.filter(e=>e.event==="job").length,1);assert.equal(x.at(-1).payload.complete,false);assert.equal(x.at(-1).payload.pages,1)});
test("robots and rate limits are explicit failures, never empty success",async()=>{const blocked=await run("/blocked/jobs",{robotsUrl:base+"/blocked/robots.txt"}),limited=await run("/rate-limited",{});assert.equal(blocked.at(-1).payload.code,"robots_denied");assert.equal(limited.at(-1).payload.code,"rate_limited")});
test("override warning and unsupported custom sources are explicit",async()=>{const override=await run("/static",{itemSelector:".job",titleSelector:"h2"},"static-css",{robotsOverride:true}),custom=await run("/static",{},"custom-api");assert.equal(event(override,"warning").payload.code,"robots_override");assert.equal(custom.at(-1).payload.code,"incomplete")});
test("private targets and non HTTP schemes fail before worker navigation",async()=>{const privateTarget=await run("/static",{},"static-css",{allowPrivateNetwork:false}),badScheme=await run("/static",{urlTemplate:"file:///etc/passwd"});assert.equal(privateTarget.at(-1).payload.code,"network_error");assert.equal(badScheme.at(-1).payload.code,"network_error")});
test("expectedHost drift from the final response host is a warning, not silent",async()=>{const drifted=await run("/json",{mode:"json",expectedHost:"nope.example.test"}),matched=await run("/json",{mode:"json",expectedHost:"127.0.0.1"});assert.equal(event(drifted,"warning").payload.code,"host_drift");assert.equal(event(matched,"warning"),undefined)});
test("description entity decoding handles numeric and named refs",async()=>{const{decodeEntities}=await import("./worker.mjs");assert.equal(decodeEntities("what&#39;s new &amp; improved&mdash;now"),"what's new & improved—now")});
test("a known listing-only job emits one batched sighting and no persistence event",async()=>{
  const {listingHash}=await import("./worker.mjs"),known=[listingHash(`${base}/apply`,"Firmware Engineer",null)];
  const events=await new Promise((resolve,reject)=>{const c=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),out=[];let err="";c.stdout.on("data",d=>out.push(...String(d).trim().split("\n").filter(Boolean)));c.stderr.on("data",d=>err+=d);c.on("error",reject);c.on("close",code=>code?reject(Error(err)):resolve(out.map(JSON.parse)));c.stdin.end(JSON.stringify({protocolVersion:1,command:"scrape_source",runId:"known-static",known,source:{id:"s",name:"Fixture",baseUrl:base+"/static",adapterId:"static-css",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,itemSelector:".job",titleSelector:"h2",urlSelector:"a[href]"}}})+"\n")});
  assert.equal(events.filter(event=>event.event==="seen_batch").length,1);
  assert.equal(events.some(event=>event.event==="job"),false);
  assert.equal(events.at(-1).payload.skipped,1);
  assert.equal(events.at(-1).payload.persisted,0);
});
function sendRaw(line){return new Promise((resolve,reject)=>{const c=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),out=[];let err="";c.stdout.on("data",d=>out.push(...String(d).trim().split("\n").filter(Boolean)));c.stderr.on("data",d=>err+=d);c.on("error",reject);c.on("close",code=>code?reject(Error(err)):resolve(out.map(JSON.parse)));c.stdin.end(line+"\n")})}
test("malformed input and rejected commands emit a failed event instead of crashing on an out-of-scope runId",async()=>{const garbage=await sendRaw("not json");assert.equal(garbage.at(-1).event,"failed");assert.equal(garbage.at(-1).payload.code,"parse_error");assert.equal(garbage.at(-1).runId,"unknown");const unknownCommand=await sendRaw(JSON.stringify({protocolVersion:1,command:"bogus",runId:"r1"}));assert.equal(unknownCommand.at(-1).event,"failed");assert.equal(unknownCommand.at(-1).runId,"r1")});

function probe(path,configJson={}){return new Promise((resolve,reject)=>{const c=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),out=[];let err="";c.stdout.on("data",d=>out.push(...String(d).trim().split("\n").filter(Boolean)));c.stderr.on("data",d=>err+=d);c.on("error",reject);c.on("close",code=>code?reject(Error(err)):resolve(out.map(JSON.parse).at(-1)));c.stdin.end(JSON.stringify({protocolVersion:1,command:"probe_source",runId:"probe",source:{name:"Probe",baseUrl:base+path,adapterId:"static-css",kind:"active",robotsOverride:false,allowPrivateNetwork:true,configJson:{testNoDelay:true,...configJson}}})+"\n")})}
test("probe detects platform from the live response instead of echoing the chosen adapter",async()=>{
  // Eightfold: identified from the robots allow-list. This fixture's robots.txt names no
  // sitemap, so the tenant domain the search endpoint requires cannot be read from it and
  // the probe says so rather than recommending a config that would 422 on first use.
  const eightfold=await probe("/static",{robotsUrl:base+"/eightfold-robots.txt"});
  assert.equal(eightfold.payload.recommendedAdapter,"eightfold");
  assert.equal(eightfold.payload.detectedConfig.eightfoldApi,"pcsx");
  assert.equal(eightfold.payload.detectedConfig.domain,undefined);
  assert.match(eightfold.payload.warnings.join(" "),/domain/);
  // Feed adapters keep the full path the user pointed at; platform adapters use the origin.
  const rss=await probe("/rss");
  assert.equal(rss.payload.recommendedAdapter,"rss");
  assert.equal(rss.payload.finalUrl,base+"/rss");
  // An unknown board configures itself: selectors are inferred and then PROVED by parsing,
  // so what comes back is a config that already produced listings, not empty fields to fill in.
  const board=await probe("/board");
  assert.equal(board.payload.recommendedAdapter,"static-css");
  assert.equal(board.payload.confidence,"high");
  assert.equal(board.payload.detectedConfig.itemSelector,"li.job-card");
  assert.equal(board.payload.detectedConfig.titleSelector,"a.job-card__title");
  assert.equal(board.payload.sampleJobs[0].title,"Firmware Engineer 0");
  // A page with no repeating job list is reported unsupported rather than half-configured.
  const unknown=await probe("/static");
  assert.equal(unknown.payload.recommendedAdapter,null);
  assert.equal(unknown.payload.unsupported,true);
  // Probe never echoes the adapter it was handed: every case above was sent "static-css".
  assert.notEqual(eightfold.payload.recommendedAdapter,"static-css");
});

// The app holds the worker's stdin open for the whole run so it can send resume/cancel, then
// reads stdout until EOF. A worker that lingers after its terminal event therefore never yields
// EOF, and the command that started it waits forever. Every other test here closes stdin, which
// hid this: the run must finish AND the process must exit while stdin is still open.
test("a finished run releases the worker even while its stdin stays open",async()=>{
  const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()});
  const events=[];
  child.stdout.on("data",d=>String(d).trim().split("\n").filter(Boolean).forEach(l=>events.push(JSON.parse(l))));
  child.stdin.write(JSON.stringify({protocolVersion:1,command:"probe_source",runId:"open-stdin",source:{name:"Fixture",baseUrl:base+"/board",adapterId:"static-css",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true}}})+"\n");
  const exit=await Promise.race([
    new Promise(resolve=>child.on("close",resolve)),
    new Promise(resolve=>setTimeout(()=>{child.kill();resolve("timeout")},15000)),
  ]);
  assert.notEqual(exit,"timeout","worker still running after its terminal event");
  assert.equal(events.at(-1).event,"completed",JSON.stringify(events));
});

// A listing row carries a placeholder description — Arm stores the job category, u-blox the
// department — and the normalizer reads descriptionText before description. Enrichment used to
// spread the stored row over the freshly read page, so the placeholder survived and the real
// description was thrown away: every enriched Arm and u-blox job kept a one-word "description".
test("enrichment replaces the listing's placeholder description with the page it just read",async()=>{
 const server=createServer((req,res)=>{
  res.setHeader("content-type","text/html").end(`<div class="ats-description"><p>Design the SoC. Own the RTL. Ship it.</p></div>`);
 });
 await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
 const base=`http://127.0.0.1:${server.address().port}`;
 try{
  const events=await new Promise(resolve=>{
   const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
   child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));
   child.on("close",()=>resolve(lines.map(line=>{try{return JSON.parse(line)}catch{return null}}).filter(Boolean)));
   child.stdin.end(JSON.stringify({protocolVersion:1,command:"enrich_source",runId:"placeholder",
    source:{id:"s",name:"Arm",baseUrl:base,adapterId:"arm",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true},
     enrichmentJobs:[{jobId:"stored",title:"SoC Engineer",company:"Arm",listingHash:"h1",detailUrl:`${base}/job/1`,
      descriptionText:"Hardware",descriptionStatus:"pending"}]}})+"\n");
  });
  assert.equal(events.at(-1).event,"completed",JSON.stringify(events.at(-1)));
  const enriched=events.find(event=>event.event==="enriched_job")?.payload;
  assert.equal(enriched.descriptionStatus,"complete");
  assert.match(enriched.descriptionText,/Design the SoC\. Own the RTL\./);
  assert.doesNotMatch(enriched.descriptionText,/^Hardware$/,"the listing's category is no longer the description");
 }finally{await new Promise(resolve=>server.close(resolve))}
});


// A board writing "Jun. 09, 2026" was stored as the 8th for anyone east of Greenwich: the string
// parses to local midnight and toISOString then moved it back a day. Dates are days, not instants.
test("a written posting date keeps the day it was written",()=>{
 assert.equal(toIsoDate("Jun. 09, 2026"),"2026-06-09","the format careers.arm.com publishes");
 assert.equal(toIsoDate("Sep. 01, 2026"),"2026-09-01");
 assert.equal(toIsoDate("September 1, 2026"),"2026-09-01");
 // JSON-LD on the same page writes it unpadded.
 assert.equal(toIsoDate("2026-9-1"),"2026-09-01");
 // An instant with a zone is already a day and is taken as written.
 assert.equal(toIsoDate("2026-06-09T22:00:00Z"),"2026-06-09");
 // Nothing usable stays nothing: a board that publishes no date must not invent one.
 assert.equal(toIsoDate(""),null);
 assert.equal(toIsoDate(null),null);
 assert.equal(toIsoDate("Various"),null)});
