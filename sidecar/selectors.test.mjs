import assert from "node:assert/strict";import {createServer}from "node:http";import {spawn}from "node:child_process";import {after,before,test}from "node:test";
// The CSS picker preferred an element's href over its text for every selector, so a title
// matched by an <a> (the default titleSelector is "h1,h2,h3,a") was stored as a URL, and
// relative listing links were persisted unresolved.
let html=`<ul><li class="job-card"><a class="job-card__title" href="/job/austin/theorem-prover/1">Theorem Proving Engineer</a><span class="location">Austin, Texas</span></li></ul>`;
let server,base;
before(async()=>{server=createServer((req,res)=>res.setHeader("content-type","text/html").end(html));await new Promise(r=>server.listen(0,"127.0.0.1",r));base=`http://127.0.0.1:${server.address().port}/search-jobs/`});
after(()=>new Promise(r=>server.close(r)));
const serve=markup=>{html=markup};
// Infers a config from markup, then scrapes that same markup through the worker end to end.
const parse=async(markup,config)=>{serve(markup);return (await run(config)).filter(e=>e.event==="job").map(e=>e.payload)};
// Probes an arbitrary URL end to end, the way saving a new source does.
const probe=url=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];child.stdout.on("data",d=>lines.push(...String(d).trim().split("\n").filter(Boolean)));child.on("error",reject);child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));child.stdin.end(JSON.stringify({protocolVersion:1,command:"probe_source",runId:"probe",source:{name:"Fixture",baseUrl:url,adapterId:"static-css",kind:"active",allowPrivateNetwork:true,configJson:{testNoDelay:true,maxPages:1}}})+"\n")});
const run=config=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,["sidecar/worker.mjs"],{cwd:process.cwd()}),lines=[];child.stdout.on("data",d=>lines.push(...String(d).trim().split("\n").filter(Boolean)));child.on("error",reject);child.on("close",code=>code?reject(Error(`worker ${code}`)):resolve(lines.map(JSON.parse)));child.stdin.end(JSON.stringify({protocolVersion:1,command:"test_source",runId:"selectors",source:{name:"Fixture",baseUrl:base,adapterId:"static-css",kind:"active",allowPrivateNetwork:true,robotsOverride:true,configJson:{testNoDelay:true,maxPages:1,...config}}})+"\n")});
test("linked titles keep their text and relative job links resolve against the page",async()=>{
 const events=await run({itemSelector:"li.job-card",titleSelector:"a.job-card__title",locationSelector:"span.location",urlSelector:"a.job-card__title"});
 const job=events.find(e=>e.event==="job")?.payload;assert.ok(job,JSON.stringify(events));
 assert.equal(job.title,"Theorem Proving Engineer");assert.equal(job.location,"Austin, Texas");
 assert.equal(job.applyUrl,new URL("/job/austin/theorem-prover/1",base).toString())});

// Selector inference has to survive the noise a real careers page carries: nav links, a footer,
// and utility classes on the job container itself.
test("selectors are inferred from repeated job markup and ignore navigation chrome",async()=>{
 const {inferSelectors}=await import("./worker.mjs");
 const page=`<nav><a href="/about">About us</a><a href="/contact">Contact</a></nav><ul><li class="job-card fs-start fs-middle"><a class="job-card__title" href="/job/austin/role-0/0">Theorem Proving Engineer 0</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start fs-middle"><a class="job-card__title" href="/job/lund/role-1/1">Theorem Proving Engineer 1</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start fs-middle"><a class="job-card__title" href="/job/cambridge/role-2/2">Theorem Proving Engineer 2</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start fs-middle"><a class="job-card__title" href="/job/budapest/role-3/3">Theorem Proving Engineer 3</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start fs-middle"><a class="job-card__title" href="/job/tokyo/role-4/4">Theorem Proving Engineer 4</a><span class="location">Austin, Texas</span></li></ul><footer><a href="/legal">Legal</a></footer>`;
 const config=await inferSelectors(page);
 assert.equal(config.itemSelector,"li.job-card.fs-start");
 assert.equal(config.titleSelector,"a.job-card__title");
 assert.equal(config.urlSelector,"a.job-card__title");
 assert.equal(config.locationSelector,"span.location")});
test("a page with no repeating job list infers nothing",async()=>{
 const {inferSelectors}=await import("./worker.mjs");
 assert.equal(await inferSelectors("<h1>About us</h1><p>We are a company.</p><a href=\"/contact\">Contact us today</a>"),null)});
// The suggested name lands on every source card, so page-title boilerplate is trimmed — but a
// title that is nothing but boilerplate must still yield a name rather than an empty field.
test("suggested names drop page-title boilerplate without ever emptying",async()=>{
 const {siteName}=await import("./worker.mjs");
 assert.equal(siteName("<title>Arm Job Search Results"),"Arm");
 assert.equal(siteName("<title>Figma | Careers"),"Figma");
 assert.equal(siteName("<title>Anthropic"),"Anthropic");
 assert.equal(siteName("<title>Careers"),"Careers");
 assert.equal(siteName("<p>no title here</p>"),null)});

// The footer is the trap: it repeats more links than the job list does, so scoring on repetition
// alone returned "Cookies Policy" and "SEC Filings" as jobs. A job list is links landing in ONE
// directory that names itself; footer links scatter and share no prefix past the root.
test("navigation and footer links never win over the job list",async()=>{
 const {inferSelectors}=await import("./worker.mjs");
 const page=`<nav><a href="/about-us">about us</a><a href="/contact-us">contact us</a><a href="/privacy-policy">privacy policy</a><a href="/cookies-policy">cookies policy</a><a href="/sec-filings">sec filings</a></nav><ul><li class="job-card fs-start"><a class="job-card__title" href="/job/austin/role-0/0">Firmware Engineer 0</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/lund/role-1/1">Firmware Engineer 1</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/cambridge/role-2/2">Firmware Engineer 2</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/budapest/role-3/3">Firmware Engineer 3</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/tokyo/role-4/4">Firmware Engineer 4</a><span class="location">Austin, Texas</span></li></ul><footer><a href="/about-us">about us</a><a href="/contact-us">contact us</a><a href="/privacy-policy">privacy policy</a><a href="/cookies-policy">cookies policy</a><a href="/sec-filings">sec filings</a></footer>`;
 const config=await inferSelectors(page,"https://careers.example.test/search-jobs/");
 assert.equal(config.itemSelector,"li.job-card.fs-start");
 assert.equal(config.urlPrefix,"/job");
 assert.equal(config.locationSelector,"span.location")});
// Arm's content cards ("Audrey's Story") carry the same classes as its job cards, so the selector
// matches both. The directory that identified the list is what separates them.
test("cards sharing the job class but not the job directory are dropped",async()=>{
 const {inferSelectors}=await import("./worker.mjs");
 const page=`<ul><li class="job-card fs-start"><a class="job-card__title" href="/job/austin/role-0/0">Firmware Engineer 0</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/lund/role-1/1">Firmware Engineer 1</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/cambridge/role-2/2">Firmware Engineer 2</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/budapest/role-3/3">Firmware Engineer 3</a><span class="location">Austin, Texas</span></li><li class="job-card fs-start"><a class="job-card__title" href="/job/tokyo/role-4/4">Firmware Engineer 4</a><span class="location">Austin, Texas</span></li><li class="job-card job-card--content fs-start"><a class="job-card__title" href="/story-0">Life at the company 0</a></li><li class="job-card job-card--content fs-start"><a class="job-card__title" href="/story-1">Life at the company 1</a></li><li class="job-card job-card--content fs-start"><a class="job-card__title" href="/story-2">Life at the company 2</a></li><li class="job-card job-card--content fs-start"><a class="job-card__title" href="/story-3">Life at the company 3</a></li></ul>`;
 const config=await inferSelectors(page,"https://careers.example.test/search-jobs/");
 assert.equal(config.urlPrefix,"/job");
 const rows=await parse(page,config);
 assert.equal(rows.length,5,JSON.stringify(rows.map(r=>r.title)));
 assert.ok(rows.every(r=>r.applyUrl.includes("/job/")),JSON.stringify(rows.map(r=>r.applyUrl)))});
// Greenhouse puts the title AND the location inside one anchor, so the anchor's own text reads
// "Anthropic Fellows ProgramLondon, UK; …".
test("a title wrapped alongside other text in one anchor is isolated",async()=>{
 const {inferSelectors}=await import("./worker.mjs");
 const cells=Array.from({length:4},(_,i)=>`<td class="cell"><a href="/acme/jobs/${i}"><p class="body body--medium">Staff Engineer ${i}</p><p class="body body--metadata">London, UK</p></a></td>`).join("");
 const config=await inferSelectors(`<table><tr>${cells}</tr></table>`,"https://boards.example.test/acme");
 assert.equal(config.titleSelector,"a p.body.body--medium");
 const rows=await parse(`<table><tr>${cells}</tr></table>`,config);
 assert.equal(rows[0].title,"Staff Engineer 0")});

// Location and posting date were the two fields that stayed empty on nearly every source: static
// pages carry them without a configured selector, and JSON boards hand them over as objects or as
// relative prose that the database dropped or could not sort.
test("locations and posting dates are read even when no selector names them",async()=>{
 const cards=Array.from({length:4},(_,i)=>`<li class="card"><a class="t" href="/job/role-${i}/${i}">Verification Engineer ${i}</a><span class="job-location">Cambridge, UK</span><time datetime="2026-03-0${i+1}">${i+1} March</time></li>`).join("");
 const {inferSelectors}=await import("./worker.mjs");
 const config=await inferSelectors(`<ul>${cards}</ul>`,"https://careers.example.test/search-jobs/");
 assert.equal(config.dateSelector,"time[datetime]");
 const rows=await parse(`<ul>${cards}</ul>`,config);
 assert.equal(rows[0].location,"Cambridge, UK");
 assert.equal(rows[0].postedAt,"2026-03-01");
 // Even with the inferred config thrown away, the card still yields both fields.
 const bare=await parse(`<ul>${cards}</ul>`,{itemSelector:"li.card",titleSelector:"a.t",urlSelector:"a.t"});
 assert.equal(bare[0].location,"Cambridge, UK");
 assert.equal(bare[0].postedAt,"2026-03-01")});
test("posting dates from every shape a board publishes become one sortable day",async()=>{
 const {toIsoDate,toLocation}=await import("./worker.mjs");
 const now=Date.parse("2026-03-10T12:00:00Z");
 assert.equal(toIsoDate("2026-02-01T09:30:00Z",now),"2026-02-01");
 assert.equal(toIsoDate("Posted 3 Days Ago",now),"2026-03-07");
 assert.equal(toIsoDate("Posted Today",now),"2026-03-10");
 assert.equal(toIsoDate("yesterday",now),"2026-03-09");
 assert.equal(toIsoDate("30+ days ago",now),"2026-02-08");
 assert.equal(toIsoDate("March 2, 2026",now),"2026-03-02");
 assert.equal(toIsoDate("whenever",now),null);
 assert.equal(toIsoDate("",now),null);
 assert.equal(toIsoDate(null,now),null);
 // Structured locations used to reach the database as an object and be discarded.
 assert.equal(toLocation({city:"Lisbon",country:"Portugal"}),"Lisbon, Portugal");
 assert.equal(toLocation([{name:"Remote - US"}]),"Remote - US");
 assert.equal(toLocation("  Austin,  Texas "),"Austin, Texas");
 assert.equal(toLocation({}),null)});

// jobs.apple.com/pt-pt/search 301s to /pt-pt/search?location=portugal-PRTC, and any query at all
// suppresses that redirect. Saving the redirect target pinned the source to one country for ever.
test("a redirect that only adds the site's own default filter never becomes the saved source",async()=>{
 const cards=Array.from({length:4},(_,i)=>`<li class="job-card"><a class="job-card__title" href="/search/job-${i}/${i}">Verification Engineer ${i}</a><span class="location">Anywhere</span></li>`).join("");
 const filtered=createServer((req,res)=>{
  const url=new URL(req.url,"http://127.0.0.1");
  if(url.pathname==="/robots.txt")return res.setHeader("content-type","text/plain").end("User-agent: *\nAllow: /\n");
  // The bare search path is redirected to the site's own location default; anything with a query
  // is served directly, exactly as Apple behaves.
  if(url.pathname==="/search"&&!url.search){res.writeHead(301,{location:"/search?location=portugal-PRTC"});return res.end()}
  res.setHeader("content-type","text/html").end(`<ul>${cards}</ul>`)});
 await new Promise(r=>filtered.listen(0,"127.0.0.1",r));
 const origin=`http://127.0.0.1:${filtered.address().port}`;
 try{
  const events=await probe(`${origin}/search`);
  const done=events.find(e=>e.event==="completed");
  assert.ok(done,JSON.stringify(events));
  assert.equal(done.payload.finalUrl,`${origin}/search?location=`,"the saved URL must clear the site's default filter, not inherit it");
  const warned=events.find(e=>e.event==="warning"&&e.payload.code==="default_filter_cleared");
  assert.ok(warned,"the user has to be told the site tried to narrow the search");
  assert.match(warned.payload.message,/location=portugal-PRTC/);
 }finally{await new Promise(r=>filtered.close(r))}});
test("a redirect that corrects the address is still followed",async()=>{
 const {widenedUrl}=await import("./worker.mjs");
 const entered=new URL("https://jobs.example.test/pt-pt/search");
 // Same page, filter added: widen it.
 assert.equal(widenedUrl(entered,new URL("https://jobs.example.test/pt-pt/search?location=portugal-PRTC")),"https://jobs.example.test/pt-pt/search?location=");
 // Moved host (jobs.intel.com to its Workday tenant) or moved path: a real correction, left alone.
 assert.equal(widenedUrl(entered,new URL("https://acme.wd1.myworkdayjobs.com/pt-pt/search?x=1")),null);
 assert.equal(widenedUrl(entered,new URL("https://jobs.example.test/en-us/search?x=1")),null);
 // A filter the user typed themselves is theirs to keep.
 assert.equal(widenedUrl(new URL("https://jobs.example.test/search?team=silicon"),new URL("https://jobs.example.test/search?team=silicon&location=x")),null);
 // No redirect at all.
 assert.equal(widenedUrl(entered,new URL("https://jobs.example.test/pt-pt/search")),null)});

// Date.parse always reads a numeric date month-first, so a European board publishing 04/03/2026
// for 4 March was stored as 3 April — a month of error, and an age filter built on it is wrong by
// that much. Whenever one component settles the question, both orderings must agree.
test("a numeric posting date is read correctly whichever way round day and month are", async () => {
  const {toIsoDate} = await import("./worker.mjs");
  const now = Date.parse("2026-08-31T12:00:00Z");
  // 25 cannot be a month, so both spellings describe the same day.
  assert.equal(toIsoDate("25/03/2026", now), "2026-03-25", "day-first");
  assert.equal(toIsoDate("03/25/2026", now), "2026-03-25", "month-first");
  assert.equal(toIsoDate("25.03.2026", now), "2026-03-25", "dots read the same as slashes");
  assert.equal(toIsoDate("25/03/26", now), "2026-03-25", "two-digit year");
  assert.equal(toIsoDate("Posted 25/03/2026", now), "2026-03-25", "the label is ignored");
  // Year-first is never ambiguous.
  assert.equal(toIsoDate("2026/03/04", now), "2026-03-04");
  // A date that cannot exist either way round is no date at all.
  assert.equal(toIsoDate("30/02/2026", now), null);
  assert.equal(toIsoDate("13/13/2026", now), null);
  // Genuinely ambiguous: a future reading is discarded, because nothing is posted tomorrow.
  assert.equal(toIsoDate("05/10/2026", now), "2026-05-10", "10 May is past; 5 Oct is not");
  // Still ambiguous after that: the more recent reading wins, so an age filter keeps the listing
  // rather than dropping it as too old on a guess.
  assert.equal(toIsoDate("04/03/2026", now), "2026-04-03");
});
