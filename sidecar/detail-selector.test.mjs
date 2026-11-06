import assert from "node:assert/strict";import {createServer}from "node:http";import {spawn}from "node:child_process";import {test,before,after}from "node:test";
let server,base;
const listing=`<ul><li class="job"><h2>Data Analyst</h2><a href="/jobs/1">open</a><span class="where">Porto</span></li><li class="job"><h2>ML Engineer</h2><a href="/jobs/2">open</a><span class="where">Aveiro</span></li></ul>`;
const personio=`<?xml version="1.0" encoding="UTF-8"?><workzag-jobs><position><id>11</id><office>Aveiro</office><name>Data Engineer</name></position><position><id>12</id><office>Aveiro</office><name>Cloud Engineer</name></position></workzag-jobs>`;
before(async()=>{server=createServer((req,res)=>{
 if(req.url==="/robots.txt")return res.end("User-agent: *\nAllow: /\n");
 if(req.url==="/list")return res.setHeader("content-type","text/html").end(listing);
 if(req.url==="/jobs/1")return res.setHeader("content-type","text/html").end(`<main><div class="desc"><p>Build dashboards in <b>SQL</b>.</p></div></main>`);
 if(req.url==="/jobs/2")return res.setHeader("content-type","text/html").end(`<main><div class="other">no description here</div></main>`);
 if(req.url==="/feed.xml")return res.setHeader("content-type","application/xml").end(personio);
 res.statusCode=404;res.end("")});
 await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));base=`http://127.0.0.1:${server.address().port}`});
after(()=>server.close());
function send(command,path,configJson,extra={}){return new Promise((resolve,reject)=>{
 const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),out=[];let err="";
 child.stdout.on("data",d=>out.push(...String(d).trim().split("\n").filter(Boolean).map(JSON.parse)));child.stderr.on("data",d=>err+=d);
 child.on("close",()=>resolve(out));child.on("error",reject);
 child.stdin.end(JSON.stringify({protocolVersion:1,command,runId:"r",source:{name:"Fixture",baseUrl:base+path,adapterId:configJson.itemXPath?"static-xpath":"static-css",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson,...(extra.source||{})},...Object.fromEntries(Object.entries(extra).filter(([key])=>key!=="source"))})+"\n")})}
const css={itemSelector:".job",titleSelector:"h2",urlSelector:"a[href]",locationSelector:".where",detailSelector:".desc"};
test("a detailSelector source defers each description to its own vacancy page",async()=>{
 const events=await send("scrape_source","/list",css,{deferDetails:true,known:[]}),jobs=events.filter(e=>e.event==="job").map(e=>e.payload);
 assert.equal(jobs.length,2);assert.equal(events.at(-1).event,"completed");
 for(const job of jobs){assert.equal(job.descriptionStatus,"pending");assert.equal(job.detailUrl,job.applyUrl)}
 assert.deepEqual(jobs.map(job=>job.location),["Porto","Aveiro"]);
});
test("enrichment reads the vacancy page through the selector",async()=>{
 const [first]=(await send("scrape_source","/list",css,{deferDetails:true,known:[]})).filter(e=>e.event==="job").map(e=>e.payload);
 const events=await send("enrich_source","/list",css,{source:{enrichmentJobs:[{...first,jobId:"j1"}]}}),enriched=events.find(e=>e.event==="enriched_job").payload;
 assert.equal(enriched.descriptionStatus,"complete");assert.match(enriched.descriptionText,/Build dashboards in SQL/);
});
test("a selector that matches nothing fails that job instead of storing an empty description",async()=>{
 const jobs=(await send("scrape_source","/list",css,{deferDetails:true,known:[]})).filter(e=>e.event==="job").map(e=>e.payload);
 const events=await send("enrich_source","/list",css,{source:{enrichmentJobs:[{...jobs[1],jobId:"j2"}]}});
 assert.equal(events.some(e=>e.event==="enriched_job"),false);
 assert.equal(events.at(-1).payload.failed,1);
});
test("without a detailSelector nothing is deferred",async()=>{
 const {detailSelector,...plain}=css,jobs=(await send("scrape_source","/list",plain,{known:[]})).filter(e=>e.event==="job").map(e=>e.payload);
 assert.equal(jobs.length,2);assert.equal(jobs[0].detailUrl,null);
});
test("an XPath mapping reads an XML job feed instead of being taken for RSS",async()=>{
 const events=await send("scrape_source","/feed.xml",{itemXPath:"//position",titleXPath:"./name",locationXPath:"./office",urlXPath:`concat('${base}/jobs/', ./id)`,detailSelector:".desc"},{deferDetails:true,known:[]}),
  jobs=events.filter(e=>e.event==="job").map(e=>e.payload);
 assert.equal(jobs.length,2);assert.deepEqual(jobs.map(job=>job.title),["Data Engineer","Cloud Engineer"]);
 assert.equal(jobs[0].applyUrl,`${base}/jobs/11`);assert.equal(jobs[0].location,"Aveiro");
});
