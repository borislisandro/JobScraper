/* eslint-env browser */
// The "human" half of the audit: opens a company's real careers page in Chrome, searches a term in the page's own search box, loads the
// whole result list (more/next/scroll), and counts the vacancy links it shows. Nothing here goes through the scraper.
//   node scripts/audit-site.mjs [--concurrency 3] [--only "Name,Name"] [--terms engineer,verification]
import {readFile,writeFile,mkdir}from "node:fs/promises";
import {createRequire}from "node:module";
const {chromium}=createRequire(new URL("../sidecar/package.json",import.meta.url))("playwright-core");
const args=process.argv.slice(2),opt=name=>{const i=args.indexOf(`--${name}`);return i>=0?args[i+1]:null};
const concurrency=Number(opt("concurrency")||3),only=opt("only")?.split(",").map(name=>name.trim()),terms=(opt("terms")||"engineer,verification").split(",");
const outDir=new URL("../docs/scrape-audit/site/",import.meta.url);await mkdir(outDir,{recursive:true});
const catalog=JSON.parse(await readFile(new URL("../sidecar/company-catalog.json",import.meta.url),"utf8"));
const companies=catalog.companies.filter(company=>company.kind!=="reference"&&(!only||only.includes(company.name)));
const slug=name=>name.toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"");
const UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));

// Runs inside the page: every link that looks like a vacancy, with the text a visitor reads.
const collectLinks=()=>{
 const JOB=/\/careers\/\d+$|find-your-job\/J-|\/(jobs?|positions?|postings?|openings?|vacanc\w*|requisitions?|JobDetail|job-?details?|careers?\/(?:job|position|opening)s?|opportunit\w*|roles?)\/|[?&](?:gh_jid|jobId|job_id|jid|vacancyNo|req)=|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-|\/job_posting\/|\/j\/[A-Z0-9]{8,}|\/\d{5,}/i;
 const NAV=/^(careers?|jobs?|open (positions|roles)|all jobs|search( jobs)?|apply( now)?|view (all )?jobs?|learn more|read more|home|sign in|log in|back|next|previous|load more|show more|see all.*|filter|clear.*|english|privacy.*|cookies?.*|terms.*|join.*talent.*|saved jobs?|view job|details?|more)$/i;
 const seen=new Map;
 for(const a of document.querySelectorAll("a[href]")){
  const href=a.href;if(!href||href.startsWith("javascript:")||href.startsWith("mailto:"))continue;
  const own=a.getAttribute("data-automation-id")==="jobTitle"||a.matches(".posting-title, [class*=job-title] a, a[class*=job-title], a[class*=jobTitle], a[class*=job-card], a[class*=JobCard]");
  if(!own&&!JOB.test(href))continue;
  const heading=a.querySelector("h1,h2,h3,h4,h5,[class*=title],[class*=Title]");
  let text=(heading?.innerText||a.innerText||a.getAttribute("aria-label")||"").split("\n").map(s=>s.trim()).filter(Boolean)[0]||"";
  text=text.replace(/\s+/g," ").trim();
  // Several boards (Factorial, some Personio themes) make the whole row the link and label it "Apply now"; the vacancy title is then in the row.
  if(text.length<4||NAV.test(text)||/^apply/i.test(text)){const row=a.closest("li, article, tr, [data-ui=job-opening], [class*=job-offer], [class*=jobOffer], [class*=posting]");
   const line=(row?.querySelector("h1,h2,h3,h4,h5,[class*=title],[class*=heading]")?.innerText||row?.innerText||"").split("\n").map(s=>s.trim()).filter(t=>t&&!NAV.test(t)&&!/^apply/i.test(t))[0];
   if(line)text=line.replace(/\s+/g," ").trim()}
  if(text.length<4||text.length>200||NAV.test(text))continue;
  const key=href.replace(/#.*$/,"");
  if(!seen.has(key))seen.set(key,text);
 }
 return[...seen.entries()].map(([href,title])=>({href,title}));
};
// "123 jobs", "Showing 1-20 of 123 results", "123 open positions": the page's own claim about its size.
const reportedCount=text=>{
 const patterns=[/\bof\s+([\d,]{1,7})\s*(?:results?|jobs?|positions?|openings?|vacancies|roles?|matches)/i,/\b([\d,]{1,7})\s*(?:results?|jobs?|open positions?|openings?|vacancies|roles?|positions?|matches)\b/i,/(?:results?|jobs?|openings?|positions?)\s*[:(]\s*([\d,]{1,7})\b/i];
 for(const pattern of patterns){const m=text.match(pattern);if(m){const n=Number(m[1].replace(/,/g,""));if(n>=0&&n<200000)return n}}return null};

async function dismissBanners(page){
 for(const name of[/^(accept|agree|allow)( all)?( cookies)?$/i,/^(accept all|i accept|got it|ok|okay|allow all|alle akzeptieren|accepter|aceitar|accept & close|accept cookies)$/i]){
  const button=page.getByRole("button",{name}).first();
  if(await button.count().catch(()=>0)){await button.click({timeout:1500}).catch(()=>{});await sleep(300);return}}}
async function loadAll(page,acc,maxRounds=60){
 // Pages hide vacancies behind "load more", numbered pages or lazy scrolling; keep going while the number of links grows.
 let best=0,stale=0;
 for(let round=0;round<maxRounds&&stale<3;round++){
  await gather(page,acc);
  const count=acc.size;
  if(count>best){best=count;stale=0}else stale++;
  const more=page.getByRole("button",{name:/^(load|show|view|see)\s*more|more (jobs|results|roles)/i}).first();
  const next=page.locator("a[rel=next], button[aria-label*=next i], a[aria-label*=next i], [data-automation-id=next], li.next a, a:has-text('Next'):not([aria-disabled=true])").first();
  if(await more.isVisible().catch(()=>false)){await more.click({timeout:3000}).catch(()=>{});await page.waitForLoadState("networkidle",{timeout:6000}).catch(()=>{});await sleep(400);continue}
  if(await next.isVisible().catch(()=>false)&&await next.isEnabled().catch(()=>false)){await next.click({timeout:3000}).catch(()=>{});await page.waitForLoadState("networkidle",{timeout:6000}).catch(()=>{});await sleep(500);
   // a numbered pager replaces the list, so remember everything seen so far
   continue}
  await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));await sleep(600);
 }
 await gather(page,acc)}
// Employer pages often embed the ATS board in an iframe; a visitor sees it as part of the page, so every frame is read.
async function allText(page){const parts=[];for(const frame of page.frames())parts.push(await frame.locator("body").innerText({timeout:3000}).catch(()=>""));return parts.join(" ").replace(/\s+/g," ")}
async function gather(page,acc){for(const frame of page.frames())for(const link of await frame.evaluate(collectLinks).catch(()=>[]))acc.set(link.href,link.title)}
async function search(page,term){
 const selector="input[type=search], input[placeholder*=earch i], input[aria-label*=earch i], input[name*=earch i], input[name=q], input[name=keyword], input[name=keywords], input[id*=earch i], input[data-automation-id*=earch i], input[type=text][placeholder*=ob i], input[type=text][placeholder*=ole i], input[type=text][placeholder*=eyword i]";
 for(const frame of page.frames()){
  const box=frame.locator(selector).filter({visible:true}).first();
  if(!await box.count().catch(()=>0))continue;
  await box.fill("").catch(()=>{});await box.fill(term,{timeout:4000}).catch(()=>{});await box.press("Enter").catch(()=>{});
  await page.waitForLoadState("networkidle",{timeout:10000}).catch(()=>{});await sleep(1200);return true}
 return false}
const tracker=JSON.parse(await readFile(new URL("../docs/company-support-tracker.json",import.meta.url),"utf8")).companies,official=new Map(tracker.map(row=>[row.name,row.officialCareersUrl]));
// The address a visitor uses, which for several ATS families is not the catalog's request address (a Workday tenant root is an error
// page, a JSON feed is not a page at all). Fallbacks are other server-rendered views of the same board.
export function humanUrls(company){
 const c=company.config||{},urls=[],base=company.baseUrl;
 if(company.adapterId==="workday"&&c.tenant&&c.site)urls.push(base.includes(`/${c.site}`)?base:`${new URL(base).origin}/${c.site}`);
 else if(["json","rss","static-css","static-xpath"].includes(company.adapterId)&&official.get(company.name))urls.push(official.get(company.name));
 else if(company.adapterId==="oracle")urls.push(base.replace(/\/$/,"")+"/jobs");
 else urls.push(base);
 if(company.adapterId==="greenhouse"&&c.token)urls.push(`https://boards.greenhouse.io/embed/job_board?for=${c.token}`);
 if(company.adapterId==="personio"&&c.token)urls.push(`https://${c.token}.jobs.personio.${c.domain==="com"?"com":"de"}/`);
 if(official.get(company.name))urls.push(official.get(company.name));
 return[...new Set(urls)]}
const HOSTED=new Set(["greenhouse","ashby","lever","workable","personio","bamboohr"]);
export async function checkSite(browser,company,termList){
 let last,best=null;
 for(const url of humanUrls(company)){last=await checkUrl(browser,{...company,baseUrl:url},termList);last.triedUrls=[...(last.triedUrls||[]),url];
  const links=last.baseline?.links??0,reported=last.baseline?.reported;
  if(!last.error&&!last.blockedHint&&links>=(HOSTED.has(company.adapterId)?2:3)&&!(reported&&reported>links*3))break;
  if(!best||links>(best.baseline?.links??0))best=last}
 return best&&(best.baseline?.links??0)>(last.baseline?.links??0)?best:last}
async function checkUrl(browser,company,termList){
 const context=await browser.newContext({userAgent:UA,viewport:{width:1366,height:900},locale:"en-GB"}),page=await context.newPage(),result={name:company.name,url:company.baseUrl,checkedAt:new Date().toISOString(),terms:{}};
 const harvested=new Map;
 try{
  const response=await page.goto(company.baseUrl,{waitUntil:"domcontentloaded",timeout:40000});result.status=response?.status()??0;
  await page.waitForLoadState("networkidle",{timeout:15000}).catch(()=>{});await sleep(company.adapterId==="workable"||company.adapterId==="oracle"||company.adapterId==="bamboohr"?5000:1500);await dismissBanners(page);
  const body=await allText(page);
  result.title=await page.title().catch(()=>"");result.blockedHint=/access denied|verify you are human|captcha|are you a robot|just a moment|unusual traffic|you have been blocked|attention required|request blocked/i.test(body.slice(0,2500));
  result.baseline={reported:reportedCount(body)};
  await loadAll(page,harvested,15);
  result.baseline.links=harvested.size;result.baseline.titles=[...harvested.values()];
  for(const term of termList){
   const t={};result.terms[term]=t;
   const found=await search(page,term);t.searchBox=found;
   if(!found){t.note="no search box found";continue}
   const text=await allText(page);t.reported=reportedCount(text);
   const acc=new Map;await loadAll(page,acc,120);const links=[...acc.entries()].map(([href,title])=>({href,title}));t.resultLinks=links.length;
   t.matchingTitles=links.filter(link=>link.title.toLowerCase().includes(term)).map(link=>link.title);
   t.matching=new Set(t.matchingTitles).size;
  }
  // Even when the page has no search box, the term can be counted over everything the page listed.
  for(const term of termList){const t=result.terms[term];t.onListMatching=[...harvested.values()].filter(title=>title.toLowerCase().includes(term)).length}
 }catch(error){result.error=String(error.message||error).split("\n")[0]}
 finally{await context.close()}
 return result}

if(import.meta.url===new URL(`file://${process.argv[1]}`).href){
 const browser=await chromium.launch({executablePath:process.env.JOBSCRAPER_BROWSER||"/usr/bin/google-chrome",headless:true,args:["--no-sandbox","--disable-blink-features=AutomationControlled"]});
 let next=0,done=0;
 async function work(){while(next<companies.length){const company=companies[next++],record=await checkSite(browser,company,terms);
  await writeFile(new URL(`${slug(company.name)}.json`,outDir),JSON.stringify(record));
  console.log(`${++done}/${companies.length} ${company.name}: status ${record.status} baseline ${record.baseline?.links} links (reports ${record.baseline?.reported}) ${terms.map(term=>`${term}=${record.terms[term]?.matching??"-"}/${record.terms[term]?.reported??"-"}`).join(" ")}${record.error?" ERR "+record.error.slice(0,80):""}`)}}
 await Promise.all(Array.from({length:concurrency},work));await browser.close()}
