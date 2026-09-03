import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { jitterMs, makeUrlGuard, parseRobots, privateAddress, requestSlotDelay, requiredFor, retryAfterMs, robotsAllows, workdaySite, workdayTenant } from "./worker.mjs";

test("jitter and Retry-After stay bounded and deterministic",()=>{
  assert.equal(jitterMs(()=>0),1500);
  assert.equal(jitterMs(()=>.999999),3000);
  assert.equal(retryAfterMs("2",0),2000);
  assert.equal(retryAfterMs("Wed, 21 Oct 2015 07:28:00 GMT",Date.parse("Wed, 21 Oct 2015 07:27:00 GMT")),30000);
  assert.equal(retryAfterMs("999",0),30000);
});
test("resolver guard rejects private IPv4, IPv6, mapped IPv6 and DNS rebinding",async()=>{
  for(const address of ["127.0.0.1","10.0.0.1","169.254.1.1","192.0.2.1","::1","fe80::1","fc00::1","::ffff:127.0.0.1"])assert.equal(privateAddress(address),true,address);
  const guard=makeUrlGuard(async host=>host==="safe.test"?[{address:"8.8.8.8"}]:[{address:"127.0.0.1"}]);
  await assert.rejects(()=>guard("https://rebound.test/",{}),/resolved private/);
  await assert.rejects(()=>guard("file:///tmp/x",{}),/HTTP/);
  await assert.doesNotReject(()=>guard("https://safe.test/",{}));
});

test("robots parsing follows RFC 9309 group, wildcard and longest-match rules",()=>{
  // Verbatim shape published by careers.micron.com / careers.qualcomm.com. The previous
  // Disallow-only parser denied these outright; the sites explicitly permit their jobs paths.
  const eightfold=`User-agent: *
Disallow: /
Allow: /$
Allow: /careers
Allow: /api/career_hub
User-agent: IndeedJobBot
Disallow:
`;
  const rules=parseRobots(eightfold);
  assert.equal(robotsAllows(rules,"/"),true,"Allow: /$ must beat Disallow: / on the bare root");
  assert.equal(robotsAllows(rules,"/careers"),true);
  assert.equal(robotsAllows(rules,"/api/career_hub?page=0"),true);
  assert.equal(robotsAllows(rules,"/internal/admin"),false,"unlisted paths stay denied");
  // The bare "/" root allowance is end-anchored, so deeper paths do not inherit it.
  assert.equal(robotsAllows(rules,"/nope"),false);
  // Group selection: a named group wins over the wildcard group for that agent only.
  const named=parseRobots("User-agent: *\nDisallow: /\nUser-agent: jobscraper\nDisallow:\n");
  assert.equal(robotsAllows(named,"/anything"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /\n"),"/anything"),false);
  // Longest match wins, and Allow breaks an exact-length tie.
  const nested=parseRobots("User-agent: *\nDisallow: /a/\nAllow: /a/b/\n");
  assert.equal(robotsAllows(nested,"/a/x"),false);
  assert.equal(robotsAllows(nested,"/a/b/x"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /x\nAllow: /x\n"),"/x"),true);
  // Wildcards, end anchors, comments and empty Disallow.
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /*.pdf$\n"),"/docs/a.pdf"),false);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /*.pdf$\n"),"/docs/a.pdf?x=1"),true);
  assert.equal(robotsAllows(parseRobots("# comment\nUser-agent: *\nDisallow:\n"),"/anything"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /search-jobs/\n"),"/search-jobs"),true,"prefix match is literal, not fuzzy");
});

test("Workday tenant and site detection reads real tenant shapes",()=>{
  // Tenant is the subdomain before the .wdN datacentre segment.
  assert.equal(workdayTenant("intel.wd1.myworkdayjobs.com"),"intel");
  assert.equal(workdayTenant("nvidia.wd5.myworkdayjobs.com"),"nvidia");
  assert.equal(workdayTenant("careers.micron.com"),null);
  assert.equal(workdayTenant("evil-myworkdayjobs.com"),null);
  // Site segment comes from the tenant's own robots.txt: Sitemap line first, else first Allow.
  assert.equal(workdaySite("Sitemap: https://x.wd1.myworkdayjobs.com/External_Career/siteMap.xml\n"),"External_Career");
  assert.equal(workdaySite("User-agent: *\nAllow: /NVIDIAExternalCareerSite/\nDisallow: /refreshFacet/\n"),"NVIDIAExternalCareerSite");
  assert.equal(workdaySite("User-agent: *\nDisallow:\n"),null);
  assert.equal(workdaySite(""),null);
  // Workday requires both halves; a tenant alone cannot build a CXS URL.
  assert.deepEqual(requiredFor("workday"),["tenant","site"]);
});

// Every request paid a 1.5-3 s sleep regardless of what it was fetching, which is most of what a
// detail-heavy run cost. A JSON search endpoint is paced for its own kind; an HTML listing page,
// which is far heavier for a server to render, keeps the original spacing.
test("request pacing follows what is being fetched, not one blanket delay",async()=>{
 const {paceMs}=await import("./worker.mjs");
 const lowest=()=>0,highest=()=>0.999999;
 for(const adapter of ["workday","eightfold","apple","amd"]){
  assert.equal(paceMs(adapter,lowest),250);
  assert.ok(paceMs(adapter,highest)<=500,`${adapter} should stay well under the old floor`);
 }
 for(const adapter of ["static-css","arm","cisco","google"]){
  assert.equal(paceMs(adapter,lowest),1500,`${adapter} keeps the wider spacing an HTML listing deserves`);
  // A detail page is one static document, not a search: Arm needs one per job for its posting date,
  // and listing pace made that read seven minutes long.
  assert.equal(paceMs(adapter,lowest,"detail"),500,`${adapter} detail pages are paced for what they are`);
  assert.ok(paceMs(adapter,highest,"detail")<=1000);
  assert.equal(paceMs(adapter,lowest,"listing"),1500,`${adapter} listing pace is untouched`);
 }
 // A JSON endpoint is paced by its kind whatever it is fetching.
 assert.equal(paceMs("workday",lowest,"detail"),250);
});

test("request pacing starts immediately and couples only requests to the same origin",()=>{
 const slots=new Map;
 assert.equal(requestSlotDelay("https://a.test",250,1_000,slots),0);
 assert.equal(requestSlotDelay("https://a.test",250,1_000,slots),250);
 assert.equal(requestSlotDelay("https://b.test",250,1_000,slots),0);
 assert.equal(requestSlotDelay("https://a.test",250,1_125,slots),375);
});

// Node rejects a response whose headers exceed 16KB with an opaque UND_ERR_HEADERS_OVERFLOW, and
// real boards do send more than that: www.u-blox.com ships a 19KB content-security-policy header,
// which made every one of its pages unreadable for a reason no adapter could report. The app
// spawns the worker with --max-http-header-size=65536 (see sidecar.rs); this proves the worker
// reads such a response under that flag, and records why the flag has to be there.
test("a response with headers larger than Node's default limit is still readable",async()=>{
 const oversized="x".repeat(19_000);
 const server=createServer((req,res)=>{
  res.setHeader("content-security-policy",`default-src 'self'; report-to ${oversized}`);
  if(req.url==="/robots.txt")return res.setHeader("content-type","text/plain").end("User-agent: *\nAllow: /\n");
  res.setHeader("content-type","application/json").end(JSON.stringify({total:1,jobPostings:[{title:"Firmware Engineer",jobReqId:"JR1",locationsText:"Thalwil"}]}));
 });
 await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
 const base=`http://127.0.0.1:${server.address().port}`;
 const run=args=>new Promise(resolve=>{
  const child=spawn(process.execPath,[...args,"sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];
  child.stdout.on("data",data=>lines.push(...String(data).trim().split("\n").filter(Boolean)));
  child.on("close",()=>resolve(lines.map(line=>{try{return JSON.parse(line)}catch{return null}}).filter(Boolean)));
  child.stdin.end(JSON.stringify({protocolVersion:1,command:"scrape_source",runId:"headers",known:[],
   source:{name:"Fixture",baseUrl:base,adapterId:"workday",kind:"active",allowPrivateNetwork:true,robotsOverride:false,
    configJson:{testNoDelay:true,maxPages:1,pageSize:1,listingPath:"/workday",tenant:"fixture",site:"External"}}})+"\n");
 });
 try{
  const withFlag=await run(["--max-http-header-size=65536"]);
  assert.equal(withFlag.at(-1).event,"completed",JSON.stringify(withFlag.at(-1)));
  assert.equal(withFlag.filter(event=>event.event==="job").length,1);
  // Without it the same board is simply unreachable — the failure this flag exists to prevent.
  const withoutFlag=await run([]);
  assert.equal(withoutFlag.at(-1).event,"failed");
  assert.match(String(withoutFlag.at(-1).payload.message),/fetch failed|network_error/);
 }finally{await new Promise(resolve=>server.close(resolve))}
});
