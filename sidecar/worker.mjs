#!/usr/bin/env node
import readline from "node:readline";import {createHash}from "node:crypto";import {existsSync}from "node:fs";import {readFile}from "node:fs/promises";import {performance as clock}from "node:perf_hooks";import {requestFor,normalizeItem} from "./adapters.mjs";import {companyAdapters,companyAdapterIds,companyDatesNeedDetail,enrichCompany,scrapeCompany} from "./company-adapters.mjs";
import {cookieHeader,scopedHeaders,allowed} from "./request-policy.mjs";
import {createBrowserProxy} from "./browser-proxy.mjs";
const V=1,R=2,emit=(event,runId,payload={})=>process.stdout.write(`${JSON.stringify({protocolVersion:V,event,runId,payload})}\n`),sleep=ms=>new Promise(r=>setTimeout(r,ms));
const edge=[process.env.ProgramFiles&&`${process.env.ProgramFiles}/Microsoft/Edge/Application/msedge.exe`,process.env["ProgramFiles(x86)"]&&`${process.env["ProgramFiles(x86)"]}/Microsoft/Edge/Application/msedge.exe`].filter(Boolean);
export const adapters={"static-css":{version:"1.1.0",capabilities:["direct","css","pagination"]},"static-xpath":{version:"1.1.0",capabilities:["direct","xpath","pagination"]},json:{version:"1.1.0",capabilities:["direct","pagination"]},rss:{version:"1.1.0",capabilities:["direct"]},playwright:{version:"1.1.0",capabilities:["browser","headed","session"]},apple:{version:"1.1.0",capabilities:["direct-json","pagination"]},workday:{version:"1.1.0",capabilities:["direct-json","pagination","detail","playwright-fallback"]},eightfold:{version:"1.1.0",capabilities:["direct-json","cursor","detail","playwright-fallback"]},icims:{version:"1.1.0",capabilities:["direct-json","pagination","detail","playwright-fallback"]},"talentbrew-jibe":{version:"1.1.0",capabilities:["direct-json","pagination","detail","playwright-fallback"]},phenom:{version:"1.1.0",capabilities:["direct-json","pagination","detail","playwright-fallback"]},greenhouse:{version:"1.1.0",capabilities:["direct-json","detail"]},ashby:{version:"1.1.0",capabilities:["direct-json"]},lever:{version:"1.1.0",capabilities:["direct-json","pagination"]},oracle:{version:"1.1.0",capabilities:["direct-json","pagination","detail"]},"custom-api":{version:"1.1.0",capabilities:[],unsupported:true},reference:{version:"1.1.0",capabilities:[],unsupported:true},
 // One adapter per employer whose board no generic ATS adapter reads; see company-adapters.mjs.
 ...Object.fromEntries(companyAdapterIds.map(id=>[id,{version:"1.1.0",capabilities:["direct-company","pagination",...(["rambus","renesas","arista-networks","arm","u-blox","synopsys","l3harris"].includes(id)?["detail"]:[])]}]))};
export {privateAddress,makeUrlGuard,allowed} from "./request-policy.mjs";
export const retryAfterMs=(header,nowMs=Date.now(),cap=30_000)=>{if(!header)return 0;const seconds=Number(header);const raw=Number.isFinite(seconds)?Math.ceil(seconds*1000):Math.max(0,Date.parse(header)-nowMs);return Math.min(cap,Math.max(0,raw||0))};
export const jitterMs=(random=Math.random)=>1500+Math.floor(random()*1501);
const urlFor=s=>String(s.configJson?.urlTemplate||s.baseUrl).replaceAll("{query}",encodeURIComponent(s.configJson?.query||"")).replaceAll("%QUERY%",encodeURIComponent(s.configJson?.query||""));
const num=v=>{const n=Number(v);return v!=null&&v!==""&&Number.isFinite(n)?n:null};
// ATS description HTML carries entities (NVIDIA's &#39; is the case that surfaced this);
// stripping tags alone leaves them raw. Numeric refs plus the handful of named entities
// job descriptions actually use — no full HTML5 entity table needed for prose text.
const namedEntities={amp:"&",lt:"<",gt:">",quot:'"',apos:"'",nbsp:" ",rsquo:"’",lsquo:"‘",rdquo:"”",ldquo:"“",mdash:"—",ndash:"–",hellip:"…"};
export const decodeEntities=s=>s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi,(m,e)=>{
 if(e[0]!=="#")return namedEntities[e.toLowerCase()]??m;
 const point=parseInt(e.slice(1).replace(/^x/i,""),e[1].toLowerCase()==="x"?16:10);
 return Number.isInteger(point)&&point>0&&point<=0x10ffff&&!(point>=0xd800&&point<=0xdfff)?String.fromCodePoint(point):"\ufffd";
});
// Locations arrive as a string on some boards, an object on others ({name}, {city,state}) and an
// array on the rest. Anything that was not a string used to reach the database as a JSON object
// and get dropped on the floor, so every such source stored "Unknown".
// A posting open in several offices used to store only the first of them, so filtering by any of
// the others could never find it. Keep every place; stable ordering also prevents needless
// re-reads when a publisher only rearranges its location array.
export const toLocation=v=>{if(v==null)return null;
 if(Array.isArray(v)){const places=[...new Set(v.map(toLocation).filter(Boolean))];
  if(!places.length)return null;
  return places.sort().join(", ")}
 // schema.org PostalAddress spells these addressLocality/addressRegion/addressCountry. Boards that
 // publish a JobPosting graph (TEKEVER's JSON Feed among them) hand the address straight through,
 // and without these names every one of their listings stored no place at all.
 if(typeof v==="object"){const parts=[v.name,v.displayName,v.city,v.locality,v.addressLocality,v.state,v.region,v.addressRegion,v.country,v.countryName,v.addressCountry,v.address&&toLocation(v.address)].filter(x=>typeof x==="string"&&x.trim());return parts.length?[...new Set(parts.map(p=>p.trim()))].join(", "):null}
 const text=String(v).replace(/\s+/g," ").trim();return text||null};
// Posting dates are sorted on, so they have to be one comparable format. Boards publish ISO
// timestamps, localized dates, and relative prose ("Posted 3 days ago", "Posted Today") in equal
// measure; whatever cannot be resolved to a real day is stored as nothing rather than as noise.
export const toIsoDate=(v,nowMs=Date.now())=>{if(v==null)return null;const raw=String(v).replace(/\s+/g," ").trim();if(!raw)return null;
 const iso=/^(\d{4}-\d{2}-\d{2})/.exec(raw);if(iso){const date=new Date(`${iso[1]}T00:00:00Z`);return !Number.isNaN(date.getTime())&&date.toISOString().slice(0,10)===iso[1]?iso[1]:null}
 const day=86_400_000,units={minute:0,hour:0,day,week:7*day,month:30*day,year:365*day};
 if(/\b(today|just posted|new)\b/i.test(raw))return new Date(nowMs).toISOString().slice(0,10);
 if(/\byesterday\b/i.test(raw))return new Date(nowMs-day).toISOString().slice(0,10);
 const ago=/(\d+)\s*\+?\s*(minute|hour|day|week|month|year)s?\s*(ago|old)/i.exec(raw);
 if(ago){const date=new Date(nowMs-Number(ago[1])*units[ago[2].toLowerCase()]);return Number.isNaN(date.getTime())?null:date.toISOString().slice(0,10)}
 const epoch=/^\d{10}$|^\d{13}$/.exec(raw);
 if(epoch)return new Date(Number(raw)*(raw.length===10?1000:1)).toISOString().slice(0,10);
 // Numeric dates are read here rather than by Date.parse, which always assumes month-first: a
 // European board publishing 04/03/2026 for 4 March would otherwise be stored as 3 April, and an
 // age filter built on that is a month wrong. The rules, in order of certainty:
 //   1. a four-digit part is the year, and a part above 12 can only be the day — no guessing;
 //   2. when both readings are possible, one in the future is discarded: a job cannot be posted
 //      tomorrow;
 //   3. if both survive, the more recent wins, so an age filter keeps a listing it cannot date
 //      confidently rather than silently dropping it as too old.
 const bare=raw.replace(/^posted(\s+on)?\s+/i,"");
 const numeric=/^(\d{1,4})[/.-](\d{1,2})[/.-](\d{2,4})$/.exec(bare);
 if(numeric){
  const parts=numeric.slice(1).map(Number),yearFirst=numeric[1].length===4;
  const year=yearFirst?parts[0]:parts[2]<100?2000+parts[2]:parts[2];
  const [left,right]=yearFirst?[parts[1],parts[2]]:[parts[0],parts[1]];
  const build=(month,dayOfMonth)=>{
   if(month<1||month>12||dayOfMonth<1||dayOfMonth>31)return null;
   const date=new Date(Date.UTC(year,month-1,dayOfMonth));
   return date.getUTCMonth()===month-1&&date.getUTCDate()===dayOfMonth?date:null};
  const candidates=(yearFirst?[build(left,right)]:[build(right,left),build(left,right)]).filter(Boolean);
  if(!candidates.length)return null;
  const notFuture=candidates.filter(date=>date.getTime()<=nowMs+day);
  return (notFuture.length?notFuture:candidates).sort((x,y)=>y-x)[0].toISOString().slice(0,10)}
 // A bare "March 1" parses to the year 2001, which would sort ahead of every real listing.
 if(!/\d{4}/.test(raw))return null;
 const parsed=Date.parse(bare);
 if(Number.isNaN(parsed))return null;
 // Date.parse reads "Jun. 09, 2026" as local midnight, and toISOString would then hand back the
 // 8th for everyone east of Greenwich. The day is read back in the frame it was written in.
 const date=new Date(parsed);
 return `${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,"0")}-${String(date.getDate()).padStart(2,"0")}`};
// ponytail: structured compensation fields only (salaryMin/Max/currency/period from ATS JSON); no
// free-text "$120k-$150k DOE" parser here — add one if a source only ever exposes salary as prose.
function job(x,s){const title=String(x.title||x.name||x.jobTitle||"").trim(),company=String(x.company||x.companyName||s.name||"").trim(),url=x.applyUrl||x.url||x.canonicalUrl||s.baseUrl,canonicalUrl=String(x.canonicalUrl||url),location=toLocation(x.location??x.locationName??x.locationsText??x.city),postedAt=toIsoDate(x.postedAt??x.postedDate??x.postedOn??x.datePosted??x.publishedAt),descriptionText=decodeEntities(String(x.descriptionText||x.description||x.summary||x.jobDescription||"").replace(/<[^>]*>/g," ")).replace(/\s+/g," ").trim(),externalId=x.externalId||x.id||x.requisitionId||x.jobReqId||x.jobId||createHash("sha256").update(`${s.baseUrl}|${title}|${company}|${url}`).digest("hex"),comp=x.compensation||x.payRange||x.salary||{},salaryMin=num(x.salaryMin??x.minSalary??x.payRangeMinimum??comp.min??comp.minimum),salaryMax=num(x.salaryMax??x.maxSalary??x.payRangeMaximum??comp.max??comp.maximum),detailUrl=x.detailUrl?String(x.detailUrl):null,descriptionStatus=["complete","pending","failed"].includes(x.descriptionStatus)?x.descriptionStatus:(detailUrl&&!descriptionText?"pending":"complete");return{jobId:x.jobId||null,externalId:String(externalId),canonicalUrl,applyUrl:String(x.applyUrl||url),title,company,location,workMode:x.workMode||null,descriptionText,descriptionHtml:String(x.descriptionHtml||x.description||"").slice(0,250000),detailUrl,descriptionStatus,descriptionError:x.descriptionError||null,postedAt,closingAt:x.closingAt||null,skills:Array.isArray(x.skills)?x.skills.map(String):[],seniority:x.seniority||x.seniorityLevel||x.jobLevel||null,salaryMin,salaryMax,salaryCurrency:x.salaryCurrency||x.currency||comp.currency||null,salaryPeriod:x.salaryPeriod||x.payPeriod||comp.period||null,salaryConfidence:salaryMin!=null||salaryMax!=null?"structured":null,adapterVersion:adapters[s.adapterId]?.version||"1.0.0",contentHash:createHash("sha256").update(`${title}\n${company}\n${descriptionText}`).digest("hex"),listingHash:x.listingHash||listingHash(canonicalUrl,title,location),provenance:{adapter:s.adapterId,adapterVersion:adapters[s.adapterId]?.version||"1.0.0",sourceUrl:s.baseUrl}}}
// A run that recognises a listing it already stored skips the detail fetch entirely, so the
// count of requests actually issued is the honest measure of what a run cost.
export const counters={requests:0,skipped:0,filtered:0};
// streamEmitMs is the part of emitMs spent publishing jobs from inside the adapter. It is tracked
// separately because that time sits inside adapterMs: without it, the same milliseconds would be
// counted once as "emit" and again as "adapterOther", and setup would go negative to pay for it.
const metricKinds=["robots","listing","detail","token","probe","other"],blankKind=()=>({count:0,pacingMs:0,backoffMs:0,responseMs:0,bodyMs:0}),blankMetrics=()=>({started:clock.now(),adapterMs:0,emitMs:0,streamEmitMs:0,browserMs:0,urlGuardMs:0,pacingMs:0,backoffMs:0,responseMs:0,bodyMs:0,retries:0,redirects:0,requestsByKind:Object.fromEntries(metricKinds.map(kind=>[kind,blankKind()]))});
let metrics=blankMetrics();
const rounded=value=>Math.max(0,Math.round(Number(value)||0)),kindFor=(s,request={})=>metricKinds.includes(request.metricKind)?request.metricKind:(s.__defaultRequestKind||"listing"),add=(key,started)=>{const elapsed=clock.now()-started;metrics[key]+=elapsed;return elapsed};
const timedAdapter=async action=>{const started=clock.now();try{return await action()}finally{add("adapterMs",started)}};
const workerPerformance=()=>{const totalMs=clock.now()-metrics.started,measured=metrics.urlGuardMs+metrics.pacingMs+metrics.backoffMs+metrics.responseMs+metrics.bodyMs+metrics.browserMs+metrics.streamEmitMs,adapterOtherMs=Math.max(0,metrics.adapterMs-measured),setupMs=Math.max(0,totalMs-metrics.adapterMs-(metrics.emitMs-metrics.streamEmitMs));return{version:1,worker:{totalMs:rounded(totalMs),bucketsMs:{setup:rounded(setupMs),urlGuard:rounded(metrics.urlGuardMs),pacing:rounded(metrics.pacingMs),backoff:rounded(metrics.backoffMs),response:rounded(metrics.responseMs),body:rounded(metrics.bodyMs),browser:rounded(metrics.browserMs),adapterOther:rounded(adapterOtherMs),emit:rounded(metrics.emitMs)},requestsByKind:Object.fromEntries(metricKinds.map(kind=>[kind,{...metrics.requestsByKind[kind],pacingMs:rounded(metrics.requestsByKind[kind].pacingMs),backoffMs:rounded(metrics.requestsByKind[kind].backoffMs),responseMs:rounded(metrics.requestsByKind[kind].responseMs),bodyMs:rounded(metrics.requestsByKind[kind].bodyMs)}])),retries:metrics.retries,redirects:metrics.redirects}}};
const terminalPayload=payload=>({...payload,performance:workerPerformance()});
// Everything a listing row can tell us before deciding whether its detail page is worth fetching.
// The key is the listing own path, so two openings that read alike still hash apart.
//
// The posting date is deliberately NOT part of this. Workday publishes it as "Posted 5 Days Ago"
// and Arm falls back to "3 days ago" text, so hashing it made the same unchanged listing hash
// differently every day: nothing was ever recognised, so every update re-read the whole board
// and, because an unrecognised listing is stored as description_status='pending', re-downloaded
// every description with it. Identity is the listing's own key, title and location; a posting
// whose date moved is the same posting, and a real content change is still caught by contentHash
// once the description has been read.
export const listingHash=(key,title,location)=>createHash("sha256")
 .update([key,title,toLocation(location)].map(v=>String(v??"").trim()).join("\u0000")).digest("hex");
const isKnown=(s,hash)=>Boolean(s.__known&&s.__known.has(hash));
const titlePasses=(s,title)=>!s.__titleTerms?.length||s.__titleTerms.some(term=>String(title||"").toLowerCase().includes(term));
const seenHashes=new Set;
const markSeen=hash=>{if(!hash||seenHashes.has(hash))return;seenHashes.add(hash);counters.skipped++};
// The one place that decides whether a normalized listing is kept, and the same place that hands
// it to the caller. Every adapter routes through it, so the title filter and the known-listing
// skip cannot drift apart between adapters — the generic CSS/XPath path used to apply neither.
//
// It publishes the moment a listing is accepted rather than after the whole board is read. A
// scrape used to buffer every job and emit them only once the last page was in, so a network
// error on page 480 of 500, or a cancellation nineteen minutes into a twenty-minute board, threw
// away everything already read and persisted nothing. What was read is now already delivered.
const published=new Set;
let publish=null;
const keep=(s,item)=>{
 if(!item?.title)return false;
 if(!titlePasses(s,item.title)){counters.filtered++;if(isKnown(s,item.listingHash))markSeen(item.listingHash);return false}
 if(isKnown(s,item.listingHash)){markSeen(item.listingHash);return false}
 // One board can list the same opening twice (Apple repeats a position per location) and a
 // faceted Workday read covers deliberately overlapping slices, so a listing can arrive more
 // than once. Publishing is one-way, so the duplicate has to be caught before it goes out.
 if(item.listingHash){if(published.has(item.listingHash))return false;published.add(item.listingHash)}
 if(publish){const started=clock.now();publish(item);metrics.streamEmitMs+=add("emitMs",started)}
 return true};
// Politeness is about the rate a host sees, not about sleeping for its own sake. These boards'
// JSON search endpoints answer in a few hundred milliseconds and exist to be paged, so spacing
// them by seconds bought nothing and cost hours — Apple has run at 250 ms all along with no
// complaint from Apple. An HTML listing page is far heavier for a server to render, so it keeps
// the original spacing. Any source can still override both with requestDelayMs.
const jsonAdapters=new Set(["ceva","siemens","renesas","arista-networks","apple","workday","eightfold","icims","talentbrew-jibe","phenom","greenhouse","ashby","lever","oracle","json","rss","amd","asml","mediatek","sk-hynix"]);
// A listing page is a search the site runs; a detail page is one static document it already has.
// Charging both the same 1.5-3s is most of what Arm's read now costs, because its posting dates
// only exist on those detail pages: 219 of them at listing pace is seven minutes of sleeping.
export const paceMs=(adapterId,random=Math.random,kind="listing")=>
 jsonAdapters.has(adapterId)?250+Math.floor(random()*251)
 :kind==="detail"?500+Math.floor(random()*501)
 :jitterMs(random);
const pace=(s,kind)=>s.configJson?.testJitterMs??paceMs(s.adapterId,s.configJson?.random||Math.random,kind);
const requestSlots=new Map;
export const requestSlotDelay=(origin,delay,now=Date.now(),slots=requestSlots)=>{const due=Math.max(now,slots.get(origin)??now);slots.set(origin,due+delay);return due-now};
const cancelled=new Set;
const cancelledError=runId=>{if(cancelled.has(runId))throw Error("cancelled")};
async function wait(ms,s){cancelledError(s.__runId);await sleep(ms);cancelledError(s.__runId)}
async function measuredWait(ms,s,key,kind){if(ms<=0)return;const started=clock.now();try{await wait(ms,s)}finally{const elapsed=add(key,started);metrics.requestsByKind[kind][key==="pacingMs"?"pacingMs":"backoffMs"]+=elapsed}}
async function waitForRequest(url,s,kind){if(s.configJson?.testNoDelay)return;const origin=new URL(url).origin,delay=s.configJson?.requestDelayMs??pace(s,kind);await measuredWait(requestSlotDelay(origin,delay),s,"pacingMs",kind)}
async function guarded(url,s){const started=clock.now();try{return await allowed(url,s)}finally{add("urlGuardMs",started)}}
async function retryWait(ms,s,kind){metrics.retries++;await measuredWait(ms,s,"backoffMs",kind)}
// RFC 9309 robots.txt. Group selection by user-agent, then longest-matching rule wins
// with Allow breaking ties, and * / $ wildcards. The previous parser only understood
// Disallow, so a site publishing "Disallow: /" plus an Allow list (the standard Eightfold
// careers pattern) was refused outright even though it explicitly permits its jobs paths.
export const robotsRegex=pattern=>{const anchored=pattern.endsWith("$"),body=anchored?pattern.slice(0,-1):pattern;return new RegExp(`^${body.replace(/[.+?^${}()|[\]\\]/g,"\\$&").replace(/\*/g,".*")}${anchored?"$":""}`)};
export const parseRobots=(text,agent="jobscraper")=>{const groups=[];let current=null,expectAgent=true;for(const raw of text.split(/\r?\n/)){const line=raw.replace(/#.*$/,"").trim();if(!line)continue;const idx=line.indexOf(":");if(idx<0)continue;const name=line.slice(0,idx).trim().toLowerCase(),value=line.slice(idx+1).trim();if(name==="user-agent"){if(!current||!expectAgent){groups.push(current={agents:[],rules:[]});expectAgent=true}current.agents.push(value.toLowerCase())}else if((name==="allow"||name==="disallow")&&current){expectAgent=false;current.rules.push({allow:name==="allow",pattern:value})}}
 const named=groups.filter(g=>g.agents.some(a=>a!=="*"&&a&&agent.toLowerCase().includes(a)));return (named.length?named:groups.filter(g=>g.agents.includes("*"))).flatMap(g=>g.rules)};
// An empty "Disallow:" is an explicit no-op, so only non-empty patterns constrain.
export const robotsAllows=(rules,path)=>{let best=null;for(const rule of rules){if(!rule.pattern||!robotsRegex(rule.pattern).test(path))continue;const len=rule.pattern.length;if(!best||len>best.len||(len===best.len&&rule.allow))best={len,allow:rule.allow}}return best?best.allow:true};
const robotsCache=new Map();
// expectedHost is seeded per source but was never checked against where responses
// actually come from, so a source that starts redirecting elsewhere (three of the
// starter pack already do) failed silently. One warning per run, on the first
// response — later same-host fetches (pagination, detail) would just repeat it.
const hostWarned=new Set;
const checkHostDrift=(finalUrl,s)=>{const expected=s.configJson?.expectedHost;if(!expected||hostWarned.has(s.__runId)||finalUrl.hostname===expected)return;hostWarned.add(s.__runId);emit("warning",s.__runId,{code:"host_drift",message:`Responses now come from ${finalUrl.hostname}, not the configured expectedHost ${expected}.`})};
async function robotsPolicy(s,origin){const key=`${s.__runId}|${origin}`;if(robotsCache.has(key))return robotsCache.get(key);let rules=[];try{const r=await get(s.configJson?.robotsUrl||`${origin}/robots.txt`,s,0,{metricKind:"robots"},true),type=(r.type||"").toLowerCase();
  // Several ATS hosts redirect /robots.txt to a careers page. Parsing that markup as a
  // policy silently yields "everything allowed", so report it instead of assuming.
  if(type.includes("html")||/^\s*<(!doctype|html)/i.test(r.text))emit("warning",s.__runId,{code:"robots_unparseable",message:`robots.txt at ${origin} returned ${type||"non-text content"}; continuing with no published policy.`});else rules=parseRobots(r.text);
 }catch(e){const m=String(e.message||e);
  // RFC 9309 2.3.1: 4xx (including 401/403) means no policy is published and access is
  // allowed; 5xx means unreachable and the crawler must assume disallow.
  if(/^auth_required/.test(m)||/HTTP 4\d\d/.test(m))rules=[];
  else if(/HTTP 5\d\d/.test(m))throw Error("robots_denied: robots.txt unreachable (server error); refusing to crawl",{cause:e});
  else throw e}
 robotsCache.set(key,rules);return rules}
async function get(url,s,redirects=0,request={},skipRobots=false){let last;const kind=kindFor(s,request),kindMetrics=metrics.requestsByKind[kind];for(let i=0;i<=R;i++)try{cancelledError(s.__runId);const safe=await guarded(url,s);if(!skipRobots&&!s.robotsOverride){const rules=await robotsPolicy(s,`${safe.protocol}//${safe.host}`);if(!robotsAllows(rules,`${safe.pathname}${safe.search}`))throw Error("robots_denied")}await waitForRequest(safe,s,kind);counters.requests++;kindMetrics.count++;const headers={"user-agent":"JobScraper/1.0 local personal use",...scopedHeaders(await requestHeaders(s),safe,s.baseUrl),...scopedHeaders(request.headers,safe,request.headerOrigin||safe)};const cookies=cookieHeader(s.__sessionCookies||[],safe,s.baseUrl);if(cookies&&!headers.cookie)headers.cookie=cookies;const fetchStarted=clock.now();let r;try{r=await fetch(safe,{redirect:"manual",method:request.method||"GET",headers,body:request.rawBody!==undefined?request.rawBody:request.body===undefined?undefined:JSON.stringify(request.body),signal:AbortSignal.timeout(30000)})}finally{const elapsed=add("responseMs",fetchStarted);kindMetrics.responseMs+=elapsed}if([301,302,303,307,308].includes(r.status)){metrics.redirects++;if(redirects>=10)throw Error("network_error: too many redirects");const next=r.headers.get("location");if(!next)throw Error("network_error: redirect missing location");await r.body?.cancel();return get(new URL(next,safe).toString(),s,redirects+1,{...([307,308].includes(r.status)?request:{headers:request.headers,metricKind:kind}),headerOrigin:request.headerOrigin||safe.origin},skipRobots)}
 // Eightfold answers a burst with 403 rather than 429, so for sources that declare it a throttle
 // signal a 403 is retried with backoff before it is reported as an authentication wall.
 if(r.status===403&&s.configJson?.retryForbidden&&i<R){await retryWait(2000*(i+1),s,kind);continue}
 if([401,403].includes(r.status))throw Error("auth_required");if(r.status===429){if(i===R)throw Error(`rate_limited retry-after=${r.headers.get("retry-after")||"unknown"}`);await retryWait(retryAfterMs(r.headers.get("retry-after"),Date.now(),30_000)||500*(i+1),s,kind);continue}if(!r.ok){if(r.status>=500&&i<R){await retryWait(500*(i+1),s,kind);continue}throw Object.assign(Error(`network_error HTTP ${r.status}`),{noRetry:r.status>=400&&r.status<500&&r.status!==408})}checkHostDrift(safe,s);const bodyStarted=clock.now();let text;try{text=await r.text()}finally{const elapsed=add("bodyMs",bodyStarted);kindMetrics.bodyMs+=elapsed}return{text,type:r.headers.get("content-type")||"",url:safe.toString(),headers:r.headers}}catch(e){last=e;if(e.noRetry||/^(auth_required|rate_limited|robots_denied|cancelled)/.test(String(e.message||e))||i===R)throw e;await retryWait(500*(i+1),s,kind)}throw last}
// Every fetched URL is now checked inside get(), including pagination and detail pages,
// which the base-URL-only check used to miss entirely. This stays as an up-front gate so
// a denied source fails before any listing request is issued.
async function robots(s){if(s.robotsOverride){emit("warning",s.__runId,{code:"robots_override",message:"Saved per-source robots override is active."});return}const u=await guarded(urlFor(s),s);const rules=await robotsPolicy(s,`${u.protocol}//${u.host}`);if(!robotsAllows(rules,`${u.pathname}${u.search}`))throw Error("robots_denied")}
const path=(v,p)=>p.split(".").filter(Boolean).reduce((x,k)=>x?.[k],v);
function collection(data,s){const a=s.adapterId,c=a==="workday"?data.jobPostings:a==="eightfold"?data.positions||data.data?.positions:a==="icims"||a==="talentbrew-jibe"?data.jobs||data.searchResults:a==="phenom"?data.jobs||data.data?.jobs:a==="greenhouse"||a==="ashby"?data.jobs:a==="oracle"?data.items?.[0]?.requisitionList:Array.isArray(data)?data:path(data,s.configJson?.itemsPath||"jobs")||data.results;if(!Array.isArray(c))throw Error("parse_error: adapter response has no job array");return c}
// A generated config names the keys of an arbitrary endpoint explicitly; platform and
// hand-written configs rely on the aliases below. Mapped values win, and a relative link is
// resolved against the endpoint that served it.
const mapFields=(x,fieldMap,base)=>{const out={};for(const [field,key] of Object.entries(fieldMap||{})){const v=path(x,key);if(v==null||v==="")continue;out[field]=field==="url"?absolute(String(v),base):v}return out};
function rows(data,s){return collection(data,s).map(x=>job({...x,externalId:x.jobReqId||x.requisitionId||x.id||x.jobId,title:x.title||x.jobTitle||x.name,company:x.company||x.companyName,location:x.location||x.locations?.[0]?.city,url:x.applyUrl||x.externalPath||x.url,description:x.description||x.jobDescription,postedAt:x.postedDate||x.postedAt,...mapFields(x,s.configJson?.fieldMap,s.baseUrl)},s)).filter(j=>j.title)}
// Listing markup links are relative ("/job/austin/..."); leaving them unresolved stored
// unusable apply URLs.
const absolute=(href,base)=>{if(!href)return href;try{return new URL(href,base).toString()}catch{return href}};
async function xpathPage(html,url,s,cfg){const[{DOMParser},{default:xp}]=await Promise.all([import("@xmldom/xmldom"),import("xpath")]),doc=new DOMParser().parseFromString(html,"application/xml"),value=(node,expr)=>{if(!expr)return"";const found=xp.select1(expr,node);return typeof found==="string"?found.trim():String(found?.value??found?.textContent??"").trim()},nodes=xp.select(cfg.itemXPath,doc);if(!Array.isArray(nodes)||!nodes.length)throw Error("selector_broken: XPath item mapping returned no nodes");const jobs=nodes.map(n=>job({title:value(n,cfg.titleXPath||".//h1|.//h2|.//h3|.//a"),company:value(n,cfg.companyXPath),location:value(n,cfg.locationXPath),url:absolute(value(n,cfg.urlXPath||".//a/@href"),url),descriptionText:value(n,cfg.descriptionXPath)},s)).filter(j=>j.title),next=value(doc,cfg.nextXPath);return{jobs,next:next?new URL(next,url).toString():null}}
// Location and posting date are what the Jobs page filters and sorts on, and a listing card
// nearly always carries both even when nobody configured a selector for them: the value sits in
// an element whose own class names it, or in a <time datetime>. Matching whole class tokens (not
// substrings) keeps "candidate-name" from being read as a date.
const LOCATION_TOKEN=/^(location|locations|city|cities|region|country|office|workplace|place|geo)$/i;
const DATE_TOKEN=/^(date|dates|posted|posting|postdate|posteddate|published|publishdate)$/i;
const sniff=($,node,token)=>{for(const el of $(node).find("[class]").toArray()){
 if(!String(el.attribs.class||"").split(/[\s\-_]+/).some(part=>token.test(part)))continue;
 const text=$(el).text().replace(/\s+/g," ").trim();if(text&&text.length<=120)return text}
 return""};
async function cssPage(html,url,s,cfg){const{load}=await import("cheerio"),$=load(html),pick=(n,sel,isUrl)=>{if(!sel)return"";const f=$(n).is(sel)?$(n):$(n).find(sel).first(),v=(isUrl?f.attr("href")??f.text():f.text()).trim();return isUrl&&v?absolute(v,url):v},scope=cfg.itemSelector||"[data-job-id], .job, .job-item, li",jobs=$(scope).toArray().map(n=>job({title:pick(n,cfg.titleSelector||"h1,h2,h3,a"),company:pick(n,cfg.companySelector),location:pick(n,cfg.locationSelector)||sniff($,n,LOCATION_TOKEN),url:pick(n,cfg.urlSelector||"a[href]",true),descriptionText:pick(n,cfg.descriptionSelector),postedAt:$(n).find("time[datetime]").first().attr("datetime")||pick(n,cfg.dateSelector)||sniff($,n,DATE_TOKEN)},s)).filter(j=>j.title&&inPrefix(j.applyUrl,cfg.urlPrefix,url)),next=cfg.nextSelector?$(cfg.nextSelector).first().attr("href"):null;
 // One page can show the same job twice — Arm repeats three of them in a "saved jobs" widget that
 // uses the same card markup. Same link on one page is never a second opening.
 return{jobs:[...new Map(jobs.map(j=>[j.applyUrl,j])).values()],next:next?new URL(next,url).toString():null}}
// Only constrains when a prefix was inferred; hand-written configs keep every matched row.
const inPrefix=(href,prefix,base)=>{if(!prefix)return true;try{return new URL(href,base).pathname.startsWith(prefix)}catch{return false}};
// Selector inference. Repetition alone does NOT identify a job list — nav menus, footers and
// blog carousels all repeat, and scoring on repetition happily returned AMD's footer ("Cookies
// Policy", "SEC Filings", …) as jobs. What actually marks a job list is that its links land in
// ONE directory whose name says so: /job/…, /careers/listing/…, /anthropic/jobs/…. Footer links
// scatter across unrelated paths and share no prefix past the root, so they drop out on their own.
const JOB_PATH=/\/(jobs?|careers?|positions?|openings?|opportunit\w*|roles?|listings?|vacanc\w*|requisitions?|reqs?|postings?)[/\-_]/i;
const classSig=n=>n.tagName+String(n.attribs?.class||"").split(/\s+/).filter(c=>c&&!/\d/.test(c)&&c.length<40).map(c=>`.${c}`).join("");
const trimSig=sig=>{const[tag,...classes]=sig.split(".");return tag+classes.slice(0,2).map(c=>`.${c}`).join("")};
// Longest path prefix every link shares, keeping at least one distinct segment each so a group of
// links to the SAME page cannot qualify.
export const sharedPrefix=(urls,base)=>{const paths=[...urls].map(href=>{try{return new URL(href,base).pathname.split("/").filter(Boolean)}catch{return[]}});
 if(paths.length<2)return"";let shared=0;while(paths.every(p=>p.length>shared+1&&p[shared]===paths[0][shared]))shared++;
 return shared?`/${paths[0].slice(0,shared).join("/")}`:""};
export async function inferSelectors(html,base="https://example.invalid/"){const{load}=await import("cheerio"),$=load(html||""),groups=new Map();
 $("a[href]").each((_,a)=>{const text=$(a).text().trim(),href=$(a).attr("href");
  if(text.length<3||text.length>120||!href||href.startsWith("#")||/^(javascript|mailto|tel):/i.test(href))return;
  let parent=a.parent;for(let depth=0;depth<4&&parent&&parent.tagName;depth++,parent=parent.parent){const item=classSig(parent),anchor=classSig(a),key=`${depth}|${item}|${anchor}`,group=groups.get(key)||{depth,item,anchor,node:parent,anchorNode:a,urls:new Set()};group.urls.add(href);groups.set(key,group)}});
 const best=[...groups.values()].filter(g=>g.urls.size>=3).map(g=>({...g,prefix:sharedPrefix(g.urls,base)}))
  .filter(g=>g.prefix&&JOB_PATH.test(`${g.prefix}/`))
  .map(g=>({...g,score:g.urls.size*(1+g.prefix.split("/").length)/(1+g.depth)}))
  .sort((a,b)=>b.score-a.score)[0];
 if(!best)return null;
 // Greenhouse wraps title AND location in one anchor (<a><p>Title</p><p>London, UK…</p></a>), so
 // the anchor's own text is "TitleLondon, UK…". When the anchor has element children carrying
 // text, the first one is the title.
 const parts=$(best.anchorNode).children().toArray().filter(n=>$(n).text().trim().length>2),
  titled=parts[0],anchor=trimSig(best.anchor),title=titled?`${anchor} ${trimSig(classSig(titled))}`:anchor,
  located=$(best.node).find("span[class],div[class]").toArray().find(n=>/location|city|region|place/i.test(String(n.attribs.class||""))),
  // Greenhouse names nothing: title and location are two anonymous <p>s in the same anchor. The
  // second is only read as a location when it reads like one ("London, UK", "Remote"), so a card
  // whose second line is a department is left alone rather than mislabelled.
  sibling=!located&&parts[1]&&/,|\b(remote|hybrid|on-?site)\b/i.test($(parts[1]).text())?parts[1]:null,
  // <time datetime> is the one posting-date marker that is standard across job boards.
  dated=$(best.node).find("time[datetime]").length>0;
 // Arm's job cards and its "Audrey's Story" content cards share a class, so the selector alone
 // scrapes both. The directory that identified the list also separates them: keep it and drop
 // rows whose link falls outside it.
 return{itemSelector:trimSig(best.item),titleSelector:title,urlSelector:anchor,urlPrefix:best.prefix,
  ...(located?{locationSelector:trimSig(classSig(located))}:sibling?{locationSelector:`${anchor} ${trimSig(classSig(sibling))}`}:{}),
  ...(dated?{dateSelector:"time[datetime]"}:{})}}
// JSON endpoint inference. A JS-only board ships an empty shell and fills it from its own search
// endpoint; that endpoint is the better source by far — no browser at scrape time, real fields
// rather than scraped text, and it survives a redesign of the page. Its shape is per-vendor, so
// it is inferred here rather than hand-written, and nothing is trusted: a mapping is offered only
// after the endpoint has answered a plain fetch of its own and parsed into real listings.
const CAPTURE_LIMIT=40,CAPTURE_MAX_BYTES=2_000_000;
const JSON_KEYS={
 title:/^(title|jobtitle|job_title|positiontitle|position_title|postingtitle|posting_title|name|displayname|display_name)$/i,
 url:/^(url|joburl|job_url|applyurl|apply_url|absolute_url|absoluteurl|externalpath|external_path|canonicalurl|canonical_url|detailurl|detail_url|link|href|permalink)$/i,
 externalId:/^(id|jobid|job_id|requisitionid|requisition_id|reqid|req_id|jobreqid|externalid|external_id|number|code|slug)$/i,
 location:/^(location|locations|city|cities|office|offices|workplace|region|country|locationname|location_name|primarylocation|primary_location|joblocation|job_location)$/i,
 postedAt:/^(posteddate|posted_date|posted|postedat|posted_at|postedon|posted_on|publisheddate|published_date|publishedat|published_at|dateposted|date_posted|createdat|created_at|publicationdate|publication_date|firstpublished|updatedat|updated_at)$/i,
 description:/^(description|jobdescription|job_description|descriptionhtml|summary|content|body)$/i};
// What a field's value has to look like before its key name is believed. A "title" holding a URL,
// or an "id" holding an object, means the name lied and the mapping would produce unreadable rows.
const JSON_VALUES={
 title:v=>typeof v==="string"&&v.trim().length>=2&&v.trim().length<=200&&!/^https?:\/\//i.test(v),
 url:v=>typeof v==="string"&&(/^https?:\/\//i.test(v)||v.startsWith("/")),
 externalId:v=>(typeof v==="string"&&v.trim().length>0&&v.trim().length<=120)||typeof v==="number",
 location:v=>v!=null&&(typeof v==="string"?v.trim().length>0:typeof v==="object"),
 postedAt:v=>(typeof v==="string"&&v.trim().length>0)||typeof v==="number",
 description:v=>typeof v==="string"&&v.trim().length>0};
const jsonRow=v=>v!=null&&typeof v==="object"&&!Array.isArray(v);
// Every array of objects in a payload, with the dot path that reaches it. Bounded, so a huge or
// deeply nested response cannot make detection expensive.
export const jsonArrays=(data,maxDepth=6,budget=4000)=>{const found=[],seen=new Set();let visited=0;
 const walk=(node,trail,depth)=>{if(depth>maxDepth||visited++>budget||!node||typeof node!=="object"||seen.has(node))return;seen.add(node);
  if(Array.isArray(node)){const items=node.filter(jsonRow);
   if(items.length>=3&&items.length>=node.length*0.6)found.push({path:trail.join("."),items});
   for(const [i,child] of node.slice(0,3).entries())walk(child,[...trail,i],depth+1);return}
  for(const [key,child] of Object.entries(node))walk(child,[...trail,key],depth+1)};
 walk(data,[],0);return found};
// The key a field maps to, chosen only when most rows actually carry a usable value there.
const jsonFieldKey=(items,field)=>{let best=null;
 for(const key of [...new Set(items.flatMap(item=>Object.keys(item)))]){if(!JSON_KEYS[field].test(key))continue;
  const hits=items.filter(item=>JSON_VALUES[field](item[key])).length;
  if(hits>=Math.ceil(items.length*0.6)&&(!best||hits>best.hits))best={key,hits}}
 return best?.key||null};
// Highest-scoring job-shaped array in one JSON response, as a config the json adapter already
// knows how to run: {itemsPath, fieldMap}. Null when nothing in the payload looks like a job list.
export function inferJsonMapping(data){let best=null;
 for(const {path:itemsPath,items} of jsonArrays(data)){
  const sample=items.slice(0,20),fieldMap={};
  for(const field of Object.keys(JSON_KEYS)){const key=jsonFieldKey(sample,field);if(key)fieldMap[field]=key}
  // A title and a link alone also describe a nav menu or a "related stories" carousel. A job row
  // carries at least one of an id, a location or a posting date as well, and requiring one is what
  // keeps a site footer from being offered as a job board. A row with no link of its own is
  // rejected outright: every card would point at the endpoint, which is worse than reading the page.
  if(!fieldMap.title||!fieldMap.url||!(fieldMap.externalId||fieldMap.location||fieldMap.postedAt))continue;
  const titled=sample.filter(item=>JSON_VALUES.title(item[fieldMap.title])).length,
   score=titled+(fieldMap.externalId?3:0)+(fieldMap.location?2:0)+(fieldMap.postedAt?2:0);
  if(!best||score>best.score||(score===best.score&&items.length>best.count))best={itemsPath,fieldMap,score,count:items.length}}
 return best}
// Inference is a guess until the endpoint answers on its own. Each candidate is re-fetched through
// the same guarded, robots-respecting path a saved source uses, then parsed with the config that
// would be stored — so what is offered is exactly what will run tomorrow, without a browser.
// ponytail: the generated config reads one page (the json adapter has no next-page rule of its
// own), so a board paging 20 at a time is offered its first 20. Inferring the paging parameter
// from the captured request is the next step if that ceiling bites.
async function provenJsonApi(captures,s){
 const ranked=[];
 for(const capture of captures||[]){let data;try{data=JSON.parse(capture.text)}catch{continue}
  const mapping=inferJsonMapping(data);if(mapping)ranked.push({...mapping,url:capture.url})}
 ranked.sort((a,b)=>b.score-a.score||b.count-a.count);
 for(const candidate of ranked.slice(0,5))try{
  const configJson={...(s.configJson||{}),itemsPath:candidate.itemsPath,fieldMap:candidate.fieldMap},
   probe={...s,adapterId:"json",baseUrl:candidate.url,configJson},
   response=await get(candidate.url,probe,0,{metricKind:"probe"}),
   jobs=rows(JSON.parse(response.text),probe);
  // Distinct links, not just distinct rows: a payload whose every entry resolves to one URL is a
  // widget, not a board.
  if(jobs.length>=3&&new Set(jobs.map(j=>j.applyUrl)).size>=3)return{...candidate,jobs}}
 catch{/* the next candidate endpoint is tried */}
 return null}
// One vacancy, read from its own page. A referral, a recruiter's link or a board nobody has
// configured cannot become a source — and setting one up for a single opening is the wrong trade —
// so this reads the page as a job rather than as a board. Schema.org JobPosting is the contract
// almost every ATS already publishes for Google Jobs, which makes it the one thing worth trusting
// here; the page's own <title> and og: tags are the fallback when it is absent.
const firstJobPosting=node=>{
 if(!node||typeof node!=="object")return null;
 if(Array.isArray(node)){for(const entry of node){const found=firstJobPosting(entry);if(found)return found}return null}
 const type=node["@type"];
 if(type==="JobPosting"||(Array.isArray(type)&&type.includes("JobPosting")))return node;
 return firstJobPosting(node["@graph"])};
export function captureFromHtml(html,url){
 const text=String(html||"");
 let posting=null;
 for(const match of text.matchAll(/<script[^>]+type=["']application\/ld\+json["'][^>]*>([\s\S]*?)<\/script>/gi)){
  try{posting=firstJobPosting(JSON.parse(match[1].trim()));if(posting)break}catch{/* one malformed block is not the end of the page */}}
 const meta=name=>new RegExp(`<meta[^>]+(?:property|name)=["']${name}["'][^>]+content=["']([^"']{1,400})`,"i").exec(text)?.[1];
 const heading=/<h1[^>]*>([\s\S]{1,200}?)<\/h1>/i.exec(text)?.[1];
 const strip=value=>decodeEntities(String(value??"").replace(/<[^>]*>/g," ")).replace(/\s+/g," ").trim();
 const place=value=>{
  if(!value)return null;
  const one=Array.isArray(value)?value[0]:value;
  const address=one?.address||one;
  return toLocation([address?.addressLocality,address?.addressRegion,address?.addressCountry?.name??address?.addressCountry].filter(Boolean))
   ??toLocation(one?.name)};
 const title=strip(posting?.title)||strip(meta("og:title"))||strip(heading)||strip(/<title[^>]*>([^<]{1,200})/i.exec(text)?.[1]);
 return{
  title,
  company:strip(posting?.hiringOrganization?.name)||strip(meta("og:site_name"))||siteName(text)||"",
  location:place(posting?.jobLocation)||(posting?.jobLocationType==="TELECOMMUTE"?"Remote":null),
  postedAt:posting?.datePosted??null,
  closingAt:posting?.validThrough??null,
  descriptionHtml:typeof posting?.description==="string"?posting.description:"",
  descriptionText:strip(posting?.description)||strip(meta("og:description")),
  workMode:posting?.jobLocationType==="TELECOMMUTE"?"remote":null,
  externalId:strip(posting?.identifier?.value??posting?.identifier)||null,
  url,
  // Said plainly, because a page without the standard markup gives a title and little else, and the
  // person pasting the link is the one who can fix that before it is stored.
  structured:Boolean(posting),
 }}
// Detection needs the rendered DOM for JS-only job boards. browser() returns jobs; this returns
// markup, so the same inference and parse run on the fetched and the rendered path alike.
async function renderedHtml(s,url){const metricStarted=clock.now();try{const exe=edge.find(existsSync);if(!exe)throw Error("browser_incompatible: Microsoft Edge was not found");const{browser:b,closeBrowser}=await guardedBrowser(exe,s,true);try{const ctx=await b.newContext({serviceWorkers:"block"});await installNavigationGuard(ctx,s);const p=await ctx.newPage();
 // Whatever JSON the page fetches for itself is collected here: on a JS-only board the job list
 // arrives in one of these responses, and reading it directly beats scraping the DOM it builds.
 // Only plain GETs are kept, because those are the ones a saved source can replay without a browser.
 const captures=[],bodies=[];
 p.on("response",r=>{if(captures.length+bodies.length>=CAPTURE_LIMIT)return;
  if(!/json/i.test(r.headers()["content-type"]||"")||r.request().method()!=="GET"||!r.ok())return;
  bodies.push(r.text().then(text=>{if(text.length<=CAPTURE_MAX_BYTES)captures.push({url:r.url(),text})},()=>{}))});
 const response=await p.goto(url,{waitUntil:"domcontentloaded",timeout:30000});await p.waitForLoadState("networkidle",{timeout:15000}).catch(()=>{});
 const html=await p.content();await Promise.allSettled(bodies);
 return{html,status:response?.status()??0,captures}}finally{await closeBrowser()}}finally{add("browserMs",metricStarted)}}
async function page(url,s){const r=await get(url,s),cfg=s.configJson||{};if(r.type.includes("json")||["json","workday","eightfold","icims","talentbrew-jibe","phenom"].includes(s.adapterId)||cfg.mode==="json")return{jobs:rows(JSON.parse(r.text),s),next:null};if(r.type.includes("xml")||cfg.mode==="rss"){const{XMLParser}=await import("fast-xml-parser"),x=new XMLParser({ignoreAttributes:false}).parse(r.text),items=x?.rss?.channel?.item||x?.feed?.entry||[];return{feed:true,jobs:(Array.isArray(items)?items:[items]).map(i=>job({title:i.title,company:s.name,url:typeof i.link==="string"?i.link:i.link?.["@_href"],description:i.description||i.summary,postedAt:i.pubDate||i.updated,externalId:i.guid?.["#text"]||i.guid||i.id},s)),next:null}}if(cfg.itemXPath)return xpathPage(r.text,url,s,cfg);return cssPage(r.text,url,s,cfg)}
async function requestHeaders(s){
 if(!s.__credentials)s.__credentials=(async()=>{
  const cfg=s.configJson||{};let cookies=Array.isArray(cfg.sessionCookies)?cfg.sessionCookies:[];
  if(s.sessionStatePath)try{const state=JSON.parse(await readFile(s.sessionStatePath,"utf8"));cookies=[...cookies,...(state.cookies||[])]}
  catch{throw Error("auth_required: saved browser session is unreadable")}
  s.__sessionCookies=cookies;
  return cfg.requestHeaders||{};
 })();
 return s.__credentials;
}
// Workday returns externalPath relative to the site root, not to the CXS endpoint, so
// resolving it against response.url silently drops the /wday/cxs/{tenant}/{site} prefix.
// Detail and public URLs are therefore built explicitly per adapter.
const workdayPaths=(item,cfg)=>{const p=item.externalPath;if(!p||!cfg.tenant||!cfg.site)return{};return{detail:`/wday/cxs/${encodeURIComponent(cfg.tenant)}/${encodeURIComponent(cfg.site)}${p}`,public:`/${encodeURIComponent(cfg.site)}${p}`}};
// A Workday listing row publishes no requisition id field of its own: it is the tail of
// externalPath after the last underscore ("…/Manufacturing-Technician_JR0284458" -> "JR0284458").
// bulletFields carries it too, but not at a fixed index — a promoted posting reads
// ["Spotlight Job","JR0284458"] — so trusting bulletFields[0] gave every spotlight-flagged
// posting on a board the same external id, and because rows are matched on
// (source_id, external_id) they all collapsed onto one row, overwriting each other. Only a
// listings-only update hit this: a detail fetch supplies jobReqId and never reaches the fallback.
export const workdayRequisition=item=>{
 const tail=String(item?.externalPath||"").split("/").pop()||"";
 const suffix=tail.includes("_")?tail.slice(tail.lastIndexOf("_")+1).trim():"";
 // Whatever is left: a label is prose, a requisition id is one token carrying a digit.
 const fields=(Array.isArray(item?.bulletFields)?item.bulletFields:[]).map(value=>String(value??"").trim())
  .filter(value=>value&&!/\s/.test(value)&&/\d/.test(value));
 // Workday may append a URL version suffix (JR2001099-1) while publishing JR2001099.
 // Only use the shorter identity when the listing itself corroborates it; real IDs can end -1.
 if(suffix)return fields.find(value=>value===suffix)||fields.sort((a,b)=>b.length-a.length)
  .find(value=>suffix.startsWith(value+"-")&&/^\d+$/.test(suffix.slice(value.length+1)))||suffix;
 return fields[0]||null};
// A Workday posting open in more than one office publishes the COUNT where the place belongs
// ("2 Locations"), and hides the real offices in additionalLocations, which only the detail
// payload carries. Both used to be stored verbatim, so nearly a fifth of a board's listings held
// a location no filter could ever match. The primary city needs no extra request: it is the
// segment after "job" in every Workday path ("/job/Sibiu/Crypto-Quality-SW-Process-Internship_R-1").
export const locationCount=value=>/^\s*(\d+)\s+locations?\s*$/i.exec(String(value??""))?.[1]??null;
export const pathCity=path=>{const parts=String(path||"").split("/").filter(Boolean),at=parts.indexOf("job"),raw=at>=0?parts[at+1]:null;
 if(!raw)return null;
 const city=decodeURIComponent(raw).replace(/[-_]+/g," ").replace(/\s+/g," ").trim();
 // Some tenants put the requisition in that slot ("R-10066282", "JR109173"). An office is words,
 // even when the words carry a fab number: "Taichung---Fab-16-Taiwan" is where the job is.
 return city&&/[a-z]{3}/i.test(city)&&!/^[a-z]{0,3}[\s-]*\d[\d\s-]*$/i.test(city)?city:null};
// Every place a record names, or — when it names only a count — the city from its own URL with the
// remainder counted, so "2 Locations" becomes "Sibiu (+1 more)" and stays filterable.
export const platformLocation=(record,path)=>{
 const listed=[record.location,record.additionalLocations,record.locations,record.standardizedLocations]
  .flatMap(value=>Array.isArray(value)?value:[value])
  .filter(value=>value!=null&&value!==""&&!locationCount(value));
 if(listed.length)return listed;
 const count=Number(locationCount(record.locationsText)??locationCount(record.location)),city=pathCity(path);
 if(!city)return record.locationsText??record.location??null;
 return Number.isFinite(count)&&count>1?`${city} (+${count-1} more)`:city};
// The Workday detail payload nests everything under jobPostingInfo.
// Every board publishes how many openings it holds, each under its own name. It is one number on
// the first page and the cheapest possible answer to "is there anything new here at all?".
export const boardTotal=data=>{const n=Number(data?.items?.[0]?.TotalJobsCount??data?.total??data?.count??data?.data?.count??data?.totalCount??data?.totalHits??data?.res?.totalRecords??data?.meta?.total);
 return Number.isInteger(n)&&n>=0?n:null};
const workdayTotal=(data,cfg)=>{
 const total=boardTotal(data);
 if((cfg.__countFacet||cfg.splitFacet)!=="jobFamilyGroup")return total;
 const values=data.facets?.find(f=>f.facetParameter==="jobFamilyGroup")?.values;
 return values?.length&&values.every(v=>Number.isInteger(v.count)&&v.count>=0)
  ?Math.max(total??0,values.reduce((sum,v)=>sum+v.count,0)):total;
};
const unwrap=(data,adapterId)=>adapterId==="workday"&&data?.jobPostingInfo?data.jobPostingInfo:adapterId==="oracle"&&data?.items?.[0]?data.items[0]:adapterId==="eightfold"&&data?.data&&!Array.isArray(data.data)?data.data:data;
const appleLocale=s=>s.configJson?.locale||/^\/([a-z]{2}-[a-z]{2})(?:\/|$)/i.exec(new URL(s.baseUrl).pathname)?.[1]?.toLowerCase()||"en-us";
const appleSlug=value=>String(value||"job").toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g,"").replace(/[^a-z0-9]+/g,"-").replace(/^-|-$/g,"")||"job";
async function appleDirect(s){await robots(s);const cfg=s.configJson||{},locale=appleLocale(s),origin=new URL(s.baseUrl).origin,max=cfg.maxPages||500;s.__requestHeaders=await requestHeaders(s);
 const tokenResponse=await get(`${origin}/api/v1/CSRFToken`,s,0,{metricKind:"token"}),token=tokenResponse.headers?.get("x-apple-csrf-token");if(!token)throw Error("auth_required: Apple CSRF token header is missing");
 let jobs=[],pages=0,total=Infinity,discovered=0;
 while(discovered<total&&pages<max){const request=requestFor("apple",origin,{...cfg,page:pages+1,locale,token}),response=await get(request.url,s,0,{...request,metricKind:"listing"}),data=JSON.parse(response.text),items=data?.res?.searchResults;
  total=Number(data?.res?.totalRecords);if(!Array.isArray(items)||!Number.isFinite(total))throw Error("parse_error: Apple search response changed");
  for(const item of items){discovered++;const url=`${origin}/${locale}/details/${item.positionId}/${appleSlug(item.transformedPostingTitle||item.postingTitle)}`,normalized=job({title:item.postingTitle,company:s.name,location:item.locations,url,externalId:item.positionId||item.reqId||item.id,postedAt:item.postDateInGMT||item.postingDate||item.postedDate,description:item.jobSummary,listingHash:listingHash(url,item.postingTitle,item.locations)},s);if(keep(s,normalized))jobs.push(normalized)}pages++;if(!items.length)break}
 return{jobs,discovered,complete:discovered>=total,pages,total,totalExact:true,mode:"direct-apple"}}
// Platform adapters page a JSON API 10-20 rows at a time, so the old 50-page ceiling cut
// every large board off mid-list (Qualcomm stopped at 500 of its 1,926 openings) and reported
// it only as complete:false. 500 pages covers the largest board these adapters read.
const capped=max=>[`Stopped at the configured ${max}-page limit before the end of this board. Raise maxPages on this source to read the rest.`];
async function platformDirect(s){
  await robots(s)
  const cfg=s.configJson||{},max=cfg.maxPages||500
  let state={},out=[],pages=0,complete=true,total=null,discovered=0,unstable=false;const identities=new Set()
  s.__requestHeaders=await requestHeaders(s)
  while(pages<max){
    const request=requestFor(s.adapterId,s.baseUrl,{...cfg,...state})
    const response=await get(request.url,s,0,{...request,metricKind:"listing"})
    const data=JSON.parse(response.text),items=collection(data,s)
    // Ashby publishes the entire board without a count. Only this family can use its array
    // length as an exact total; a page from another family must never stand in for its board.
    const reported=s.adapterId==="workday"?workdayTotal(data,cfg):boardTotal(data),before=identities.size
    if(total===null)total=s.adapterId==="ashby"?items.length:reported
    // Workday omits its total (or sends zero) after page one on some tenants.
    if(reported!==null&&reported!==total&&!(s.adapterId==="workday"&&reported===0))unstable=true
    for(const raw of items){
      discovered++
      // Hosted boards (Greenhouse, Ashby, Lever, Oracle) publish their own field names; each one
      // maps its record onto the generic keys this loop reads, so nothing below is per-vendor.
      const item=normalizeItem(s.adapterId,raw,cfg,s.baseUrl)
      const wd=s.adapterId==="workday"?workdayPaths(item,cfg):{}
      const identity=String(item.externalId||item.jobReqId||item.requisitionId||item.id||item.jobId||item.externalPath||item.applyUrl||item.url||item.positionUrl||item.canonicalPositionUrl||item.detailUrl||"")
      if(!identity||identities.has(identity)||!(item.title||item.jobTitle||item.name))unstable=true
      if(identity)identities.add(identity)
      let full=item,detail=wd.detail||item[cfg.detailUrlField||"detailUrl"]
      const listingUrl=item.externalUrl||item.applyUrl||(wd.public?new URL(wd.public,s.baseUrl).toString():null)||item.url||((item.positionUrl||item.canonicalPositionUrl)?new URL(item.positionUrl||item.canonicalPositionUrl,response.url).toString():null)
      const lkey=listingUrl||detail||item.externalPath||item.id||item.jobId||""
      const title=item.title||item.jobTitle||item.name
      // The hash covers what is STORED, so the day a count turns into a real city every affected
      // row is re-read once and updated in place; leaving the count in it would freeze them.
      // Hashed on the NORMALISED place, which is what gets stored: a board that reorders its
      // office list must not make every row look changed, and an object would hash as "[object
      // Object]" for every listing on the board.
      const lhash=listingHash(lkey,title,toLocation(platformLocation(item,item.externalPath||lkey)))
      const known=isKnown(s,lhash)
      if(!titlePasses(s,title)){counters.filtered++;if(known)markSeen(lhash);continue}
      if(known){markSeen(lhash);continue}
      const detailUrl=detail?new URL(detail,response.url).toString():null
      if(detailUrl&&cfg.fetchDetail!==false){
        const detailResponse=await get(detailUrl,s,0,{metricKind:"detail"})
        full=normalizeItem(s.adapterId,{...raw,...unwrap(JSON.parse(detailResponse.text),s.adapterId)},cfg,s.baseUrl)
      }
      const normalized=job({...full,
        externalId:full.externalId||full.jobReqId||full.requisitionId||full.id||full.jobId||workdayRequisition(item),
        title:full.title||full.jobTitle||full.name,
        company:full.company||full.companyName,
        location:platformLocation(full,item.externalPath||wd.public||listingUrl)??platformLocation(item,item.externalPath||lkey),
        url:full.externalUrl||full.applyUrl||(wd.public?new URL(wd.public,s.baseUrl).toString():null)||full.url||((full.positionUrl||full.canonicalPositionUrl)?new URL(full.positionUrl||full.canonicalPositionUrl,response.url).toString():null),
        description:full.description||full.jobDescription||full.job_description,
        postedAt:full.postedDate||full.postedAt||full.startDate||full.postedTs||full.creationTs||full.t_create,
        listingHash:lhash,detailUrl,
        descriptionStatus:detailUrl&&cfg.fetchDetail===false?"pending":"complete"
      },s)
      if(keep(s,normalized))out.push(normalized)
    }
    pages++
    state=request.next(data)
    // A wrapping endpoint can otherwise waste hundreds of requests on the same page.
    if(items.length&&identities.size===before){complete=false;break}
    if(!state)break
  }
  if(state)complete=false
  if(unstable||(total!==null&&discovered!==total))complete=false
  return{jobs:out,identities,discovered,complete,pages,total,totalExact:total!==null,mode:"direct-platform",warnings:[...(pages>=max&&!complete?capped(max):[]),
    ...(unstable?["The board repeated vacancies, omitted identities or changed its total while paging; this read is unfinished and cannot close stored jobs."]:[]),
    ...(total!==null&&discovered!==total?[`Read ${identities.size} unique vacancies against a published total of ${total}; this read is unfinished and cannot close stored jobs.`]:[])]}
}
// Company adapters own their own URLs and paging; they get the same guarded fetch (robots,
// SSRF guard, redirect and retry policy) every other adapter uses, and the same job normalizer.
async function companyDirect(s){if(!companyAdapters[s.adapterId].blocked){await robots(s);s.__requestHeaders=await requestHeaders(s)}
 const cfg=s.configJson||{},request=(url,options={})=>get(url,s,0,{metricKind:"listing",...options});
 const result=await scrapeCompany(s.adapterId,{request,maxPages:cfg.maxPages,
  known:s.__known||new Set(),hashListing:listingHash,acceptTitle:title=>titlePasses(s,title),fetchDetail:cfg.fetchDetail!==false});
 for(const hash of result.seen||[])markSeen(hash);
 counters.filtered+=result.filtered||0;
 const jobs=[];for(const item of result.jobs){const normalized=job(item,s);if(keep(s,normalized))jobs.push(normalized)}
 const discovered=result.discovered??result.jobs.length+(result.seen?.length||0)+(result.filtered||0);
 return{...result,discovered,jobs}}
// Workday refuses to page past ~2,000 results on an unfiltered query, so a tenant with more
// (NVIDIA publishes ~2,700) silently truncates. Splitting the query across one facet's values
// keeps every slice under the cap; overlapping slices are removed by external ID.
async function workdayDirect(s){const cfg=s.configJson||{};if(!cfg.splitFacet)return platformDirect(s);
 await robots(s);s.__requestHeaders=await requestHeaders(s);
  const probe=requestFor("workday",s.baseUrl,{...cfg,limit:1,offset:0}),data=JSON.parse((await get(probe.url,s,0,{...probe,metricKind:"listing"})).text),total=Number(data?.total);
 if(!Number.isFinite(total))throw Error("parse_error: Structure changed: Workday total is missing");
 if(total<(cfg.splitThreshold||2000))return platformDirect(s);
 const facet=(Array.isArray(data.facets)?data.facets:[]).find(f=>f.facetParameter===cfg.splitFacet),values=Array.isArray(facet?.values)?facet.values:[];
 if(!values.length)throw Error(`parse_error: Structure changed: Workday facet "${cfg.splitFacet}" published no values`);
 const seen=new Set,jobs=[],warnings=[];let pages=0,complete=true;
 for(const value of values){const slice=await platformDirect({...s,configJson:{...cfg,splitFacet:null,appliedFacets:{...(cfg.appliedFacets||{}),[cfg.splitFacet]:[value.id]}}});
  pages+=slice.pages;complete=complete&&slice.complete;
  for(const identity of slice.identities)seen.add(identity);
  if(value.count!=null&&slice.discovered!==Number(value.count))complete=false;
  warnings.push(...slice.warnings);
  jobs.push(...slice.jobs)}
 // Workday's unfiltered total itself can be capped at 2,000. Job categories are a
 // disjoint partition; location facets may overlap, so their counts must not be summed.
 const partition=cfg.splitFacet==="jobFamilyGroup"&&values.every(v=>Number.isInteger(v.count)&&v.count>=0);
 const expected=partition?Math.max(total,values.reduce((sum,v)=>sum+v.count,0)):total;
 if(seen.size!==expected)complete=false;
 if(!complete)warnings.push(`Read ${seen.size} unique vacancies against an expected total of ${expected}; this split read is unfinished and cannot close stored jobs.`);
 return{jobs,discovered:seen.size,complete,pages,total:expected,totalExact:true,mode:"direct-platform",warnings:[`Workday reported ${total} openings before splitting; checked ${seen.size} unique vacancies across ${values.length} ${facet.descriptor||cfg.splitFacet} slices.`,...warnings]}}
async function direct(s){if(s.adapterId==="apple")return appleDirect(s);if(companyAdapters[s.adapterId])return companyDirect(s);if(s.adapterId==="workday")return workdayDirect(s);if(["eightfold","icims","talentbrew-jibe","phenom","greenhouse","ashby","lever","oracle"].includes(s.adapterId))return platformDirect(s);await robots(s);const c=s.configJson||{};let u=urlFor(s),out=[],pages=0,complete=true,discovered=0,feed=false;while(u&&pages<(c.maxPages||50)){const x=await page(u,s);if(x.feed)feed=true;for(const found of x.jobs){discovered++;if(keep(s,found))out.push(found)}pages++;if(c.pageParam){const n=new URL(urlFor(s));n.searchParams.set(c.pageParam,String(pages+(c.pageStart||0)));u=x.jobs.length?n.toString():null}else u=x.next}if(u)complete=false;
 // A feed publishes its most recent items, not its publisher's whole board, and it says nothing
 // about how many were left out: TEKEVER's careers page counts 123 openings and its RSS carries
 // 100. Reaching the end of the feed therefore is not reaching the end of the board, and treating
 // it as such would reconcile the 23 it never mentioned to closed. A source that knows its feed is
 // the entire board can say so with feedIsWholeBoard.
 const capped_feed=feed&&!c.feedIsWholeBoard;
 if(capped_feed)complete=false;
 return{jobs:out,discovered,complete,pages,total:complete?discovered:null,totalExact:complete,mode:"direct",warnings:[...(pages>=(c.maxPages||50)&&!complete?capped(c.maxPages||50):[]),...(capped_feed?["A feed lists recent items rather than a whole board and publishes no total, so this read is treated as unfinished and nothing was marked closed. Set feedIsWholeBoard on this source if its feed is known to carry every opening."]:[])]}}
const pauses=new Map;const waitForUser=runId=>new Promise((resolve,reject)=>pauses.set(runId,{resolve,reject}));
async function guardedBrowser(executablePath,s,headless){
 const proxy=await createBrowserProxy(s);
 try{
  const{chromium}=await import("playwright-core");
  const browser=await chromium.launch({executablePath,headless,proxy:{server:proxy.server,bypass:proxy.bypass},args:["--force-webrtc-ip-handling-policy=disable_non_proxied_udp"]});
  return{browser,closeBrowser:async()=>{try{await browser.close()}finally{await proxy.close()}}};
 }catch(error){await proxy.close();throw error}
}
export async function installNavigationGuard(context,s){
 await context.route("**/*",async route=>{try{await allowed(route.request().url(),s);await route.continue()}catch{await route.abort("blockedbyclient")}});
}
async function browser(s,capture){const metricStarted=clock.now();try{const exe=edge.find(existsSync);if(!exe)throw Error("browser_incompatible: Microsoft Edge was not found");const{browser:b,closeBrowser}=await guardedBrowser(exe,s,s.headless!==false);try{const ctx=await b.newContext({...s.sessionStatePath?{storageState:s.sessionStatePath}:{},serviceWorkers:"block"});await installNavigationGuard(ctx,s);const p=await ctx.newPage();await p.goto((await allowed(urlFor(s),s)).toString(),{waitUntil:"domcontentloaded",timeout:30000});cancelledError(s.__runId);if(s.headless===false){emit("needs_user_action",s.__runId,{kind:capture?"login_capture":"login_or_captcha",message:"Complete login or CAPTCHA in Edge, then choose Resume."});await waitForUser(s.__runId);cancelledError(s.__runId)}if(capture)return{jobs:[],discovered:0,complete:true,pages:1,capturedStorageStateBase64:Buffer.from(JSON.stringify(await ctx.storageState())).toString("base64")};await p.waitForLoadState("networkidle",{timeout:15000}).catch(()=>{});const markup=await p.content(),cfg=s.configJson?.itemSelector?s.configJson:(await inferSelectors(markup,p.url()))||{},found=(await cssPage(markup,p.url(),s,cfg)).jobs,jobs=found.filter(item=>keep(s,item));
    // One rendered page, no paging and no vendor total — the same admission totalExact makes.
    // Claiming complete here told the database that everything not on this page had closed.
    return{jobs,discovered:found.length,complete:false,pages:1,total:null,totalExact:false}}finally{pauses.delete(s.__runId);await closeBrowser()}}finally{add("browserMs",metricStarted)}}
async function scrape(s){try{return await direct(s)}catch(error){const declared=adapters[s.adapterId]?.capabilities.includes("playwright-fallback");if(s.configJson?.allowPlaywrightFallback&&declared&&/^(parse_error|selector_broken)/.test(String(error.message||error))){emit("warning",s.__runId,{code:"playwright_fallback",message:"Declared direct adapter fallback started after parse failure."});return{...(await browser(s,false)),mode:"playwright-fallback"}}throw error}}
async function enrichDirect(s){
 const items=Array.isArray(s.enrichmentJobs)?s.enrichmentJobs:[];let completed=0,failed=0,lastProgress=0;
 if(!items.length)return{completed,failed};
 await robots(s);s.__requestHeaders=await requestHeaders(s);
 const request=(url,options={})=>get(url,s,0,{metricKind:"detail",...options});
 for(let index=0;index<items.length;index++){
  const item=items[index];cancelledError(s.__runId);
  try{
   let full;
   if(companyAdapters[s.adapterId])full=await enrichCompany(s.adapterId,item,request);
    // Deferred reads need the same vendor mapping as inline detail reads, otherwise Greenhouse
    // content and Oracle description fields are discarded when someone opens the stored job.
    else{const response=await get(item.detailUrl,s,0,{metricKind:"detail"});full=normalizeItem(s.adapterId,{...item,...unwrap(JSON.parse(response.text),s.adapterId)},s.configJson||{},s.baseUrl)}
   const normalized=job({...item,...full,jobId:item.jobId,listingHash:item.listingHash,detailUrl:item.detailUrl,
    // The stored listing row already carries a descriptionText (Arm's job category, u-blox's
    // department) and job() reads descriptionText before description, so the spread above would
    // keep that placeholder and discard the page just read. The freshly read description wins.
    description:full.description||full.jobDescription||full.job_description,
    descriptionText:full.description||full.jobDescription||full.job_description||full.descriptionText||item.descriptionText,
    descriptionStatus:"complete",descriptionError:null},s);
   if(!normalized.descriptionText.trim())throw Error("Publisher returned an empty description");
   emit("enriched_job",s.__runId,normalized);completed++
  }catch(error){failed++;emit("enrichment_failed",s.__runId,{jobId:item.jobId,listingHash:item.listingHash,message:String(error?.message||error)})}
  const now=Date.now();if(index===items.length-1||now-lastProgress>=1000){lastProgress=now;emit("progress",s.__runId,{phase:"enriching",current:index+1,total:items.length,requests:counters.requests})}
 }
 return{completed,failed}
}
export const requiredFor=id=>id==="static-css"?["itemSelector","titleSelector"]:id==="static-xpath"?["itemXPath","titleXPath"]:id==="json"?["itemsPath"]:id==="workday"?["tenant","site"]:id==="eightfold"?["domain"]:["greenhouse","ashby","lever"].includes(id)?["token"]:id==="oracle"?["apiHost","siteNumber"]:[];
// Detection signals, verified against live tenants. Workday tenant is the subdomain and
// the per-tenant site segment is published in that tenant's own robots.txt (Sitemap line,
// else the first Allow directory) — the same trick that recovered all five live tenants.
export const workdayTenant=host=>/^([a-z0-9-]+)\.wd\d+\.myworkdayjobs\.com$/i.exec(host)?.[1]||null;
export const workdaySite=text=>/\/([A-Za-z0-9_-]+)\/siteMap\.xml/i.exec(text||"")?.[1]||/^\s*Allow:\s*\/([A-Za-z0-9_-]+)\//mi.exec(text||"")?.[1]||null;
// Page titles carry boilerplate ("Arm Job Search Results", "Figma | Careers"); the source name
// shown on every card wants the company, so drop the section suffix and the filler around it.
export const siteName=html=>{const m=/<meta[^>]+property=["']og:site_name["'][^>]+content=["']([^"']{1,80})/i.exec(html||"")||/<title[^>]*>([^<]{1,80})/i.exec(html||"");if(!m)return null;
 const full=decodeEntities(m[1]).replace(/\s+/g," ").trim(),head=full.split(/\s[|–—·]\s/)[0].trim(),
  trimmed=head.replace(/\s*\b(job\s+search\s+results|search\s+results|job\s+openings|careers?|jobs?)\b\s*$/i,"").trim();
 return trimmed||head||full||null};
// A bare listing URL that redirects to the SAME page plus a query is the site applying its own
// default filter, not correcting the address: jobs.apple.com/pt-pt/search 301s to
// /pt-pt/search?location=portugal-PRTC, so a source saved from that redirect scrapes one country
// for ever. The parameter names come from the redirect itself, so emptying their values widens
// the search without knowing anything about the site — and that is exactly what makes Apple
// answer 200 with the filter off. Returns null whenever the redirect changed host or path (the
// jobs.intel.com case, a genuine correction) or the user typed a query of their own.
export const widenedUrl=(entered,final)=>{
 if(entered.search||!final.search||final.host!==entered.host||final.pathname!==entered.pathname)return null;
 const widened=new URL(final);
 for(const key of [...widened.searchParams.keys()])widened.searchParams.set(key,"");
 return widened.toString()};
// Detect on the FINAL url after redirects: jobs.intel.com 301s to intel.wd1.myworkdayjobs.com,
// and detecting on the typed host misidentifies it.
async function detect(s){const entered=await guarded(urlFor(s),s);let final=entered,html="",body="",type="",notes=[],robotsBlocked=false;
 // html is the truncated head used for vendor fingerprints; body keeps the whole document
 // because selector inference has to see the repeated job markup further down the page.
 try{const r=await get(entered.toString(),s);final=new URL(r.url);body=r.text;html=r.text.slice(0,40000);type=r.type||""}
 catch(e){const m=String(e.message||e);if(!/^(robots_denied|auth_required)/.test(m))throw e;robotsBlocked=/^robots_denied/.test(m);notes.push(`Landing page not readable (${m.split(":")[0]}); detected from host and robots policy only.`)}
 const host=final.host;let robots="";
 try{const r=await get(s.configJson?.robotsUrl||`${final.protocol}//${host}/robots.txt`,s,0,{metricKind:"robots"},true);if(!/html/i.test(r.type||"")&&!/^\s*<(!doctype|html)/i.test(r.text))robots=r.text}catch{notes.push("No readable robots.txt; policy unknown.")}
 // Platform adapters build their own paths off the origin, so they take the origin. Selector
 // and feed adapters read the page the user actually pointed at, so they keep the full path.
 const name=siteName(html),origin=`${final.protocol}//${host}/`,out=(adapter,config,confidence,evidence,url=origin,label=name)=>({recommendedAdapter:adapter,detectedConfig:config,suggestedName:label,finalUrl:url,confidence,evidence,notes,robotsBlocked});
  const tenant=workdayTenant(host);
  if(host==="jobs.apple.com"){const locale=/^\/([a-z]{2}-[a-z]{2})(?:\/|$)/i.exec(final.pathname)?.[1]?.toLowerCase()||"en-us",preview=await appleDirect({...s,baseUrl:final.toString(),adapterId:"apple",configJson:{...s.configJson,locale,maxPages:1,requestDelayMs:250}});
   return{...out("apple",{locale,maxPages:500,requestDelayMs:250},"high",`Apple search API returned ${preview.jobs.length} current vacancies.`,final.toString(),"Apple"),sampleJobs:preview.jobs.slice(0,3).map(j=>({title:j.title,location:j.location,applyUrl:j.applyUrl}))}}
 // Employer-specific boards are recognised by host, and proved by reading one live page, before
 // any vendor sniffing: careers.arm.com ships TalentBrew assets, careers.cisco.com ships Phenom
 // assets, and neither generic adapter can read the markup those tenants actually serve.
 const sample=jobs=>jobs.slice(0,3).map(j=>({title:j.title,location:j.location,applyUrl:j.applyUrl}));
 const companyId=companyAdapterIds.find(id=>companyAdapters[id].host===host&&(id!=="google"||/^\/about\/careers/i.test(final.pathname)));
 if(companyId){const board=companyAdapters[companyId];
  if(board.blocked)return out(companyId,{},"high",`${board.name} publishes no readable listing index; this source reports that blocker instead of storing an empty board as success.`,final.toString(),board.name);
  const preview=await companyDirect({...s,baseUrl:final.toString(),adapterId:companyId,configJson:{...s.configJson,maxPages:1}});
  return{...out(companyId,{maxPages:500},"high",`${board.name}'s job board returned ${preview.jobs.length} current vacancies.`,final.toString(),board.name),sampleJobs:sample(preview.jobs)}}
 if(tenant){const site=workdaySite(robots),titled=name||tenant.replace(/^./,c=>c.toUpperCase());return site?out("workday",{tenant,site},"high",`Workday host; robots.txt names site "${site}".`,origin,titled)
  :out("workday",{tenant},"medium",`Workday host, tenant "${tenant}", but robots.txt did not name a site segment — set "site" manually.`,origin,titled)}
 if(/\/api\/pcsx|\/api\/apply|\/api\/career_hub|\/careerhub\/explore\/jobs/i.test(robots)){
  // Eightfold keys its search on the employer's own domain, which the tenant publishes in the
  // domain= parameter of its robots.txt sitemap line. Tenants serve one of two response shapes,
  // so both are tried against the live endpoint and whichever answers is the one recommended.
  const domain=/sitemap:\s*\S*[?&]domain=([^&\s]+)/i.exec(robots)?.[1]||null;
  if(domain)for(const eightfoldApi of["pcsx","legacy"]){
   try{const preview=await platformDirect({...s,baseUrl:origin,adapterId:"eightfold",configJson:{...s.configJson,domain,eightfoldApi,maxPages:1,retryForbidden:true}});
    if(preview.jobs.length)return{...out("eightfold",{domain,eightfoldApi,maxPages:500,requestDelayMs:800,retryForbidden:true},"high",`Eightfold ${eightfoldApi==="legacy"?"legacy jobs":"PCSX search"} endpoint returned ${preview.jobs.length} current vacancies.`),sampleJobs:sample(preview.jobs)}}
   catch{/* the other Eightfold response shape is tried next */}}
  notes.push(domain?"Neither Eightfold response shape returned vacancies anonymously; capture a session for this tenant.":"Set \"domain\" to the employer's own domain (careers.gf.com uses globalfoundries.com); this tenant's robots.txt does not name it.");
  return out("eightfold",{...(domain?{domain}:{}),eightfoldApi:"pcsx",retryForbidden:true},"medium","robots.txt allow-list exposes Eightfold search endpoints.")}
 if(/\.icims\.com$/i.test(host))return out("icims",{},"medium","Host is an iCIMS storefront.");
 if(/xml|rss/i.test(type))return out("rss",{},"high",`Feed content-type "${type}".`,final.toString());
 if(/json/i.test(type))return out("json",{},"medium",`JSON content-type "${type}"; set "itemsPath" to the job array.`,final.toString());
 // Vendor names in the markup are only a hint — careers.arm.com ships TalentBrew assets but
 // serves Radancy's HTML-in-JSON, which the talentbrew-jibe adapter cannot parse. Those guesses
 // are therefore tried only after the inference below, which proves itself by parsing.
 const sniffed=/phenompeople|phenom\.com/i.test(html)?out("phenom",{},"medium","Phenom vendor assets referenced in page markup."):/talentbrew|radancy/i.test(html)?out("talentbrew-jibe",{},"medium","TalentBrew/Radancy vendor assets referenced in page markup."):null;
 // Otherwise infer selectors from the markup and PROVE them by parsing. A config that does not
 // actually yield listings is never offered — the caller is told the site is unsupported
 // instead of being handed empty selector fields to fill in by hand.
 const listing=final.toString(),validate=async(markup,mode,base)=>{const config=await inferSelectors(markup,base);if(!config)return null;
  const jobs=(await cssPage(markup,base,s,config)).jobs;return jobs.length>=3?{config,jobs,mode,url:base}:null};
 // Pages to read, best first: the site's default filter cleared, then the page its redirect
 // actually served. The widened page is only offered once the site has answered it without
 // redirecting straight back, so a site that insists on its filter still gets scraped — the user
 // is just told that is what happened.
 const candidates=[],widened=body?widenedUrl(entered,final):null;
 if(widened)try{const r=await get(widened,s);if(new URL(r.url).search===new URL(widened).search)candidates.push({url:widened,markup:r.text})}
  catch{/* the redirected page below is still usable */}
 if(body)candidates.push({url:listing,markup:body});
 if(widened)emit("warning",s.__runId,candidates[0]?.url===widened
  ?{code:"default_filter_cleared",message:`This site sent ${entered.pathname} to its own default filter (${final.search.slice(1)}). JobScraper cleared it, so this source reads every listing rather than only those.`}
  :{code:"default_filter_applied",message:`This site sent ${entered.pathname} to its own default filter (${final.search.slice(1)}) and would not serve the list without it, so this source only reads jobs matching that filter.`});
 let found=null;for(const candidate of candidates){found=await validate(candidate.markup,"direct",candidate.url);if(found)break}
 // Whatever the static parse concluded, the widened page is still the one to render and to save.
 const page=found?.url||candidates[0]?.url||listing;
 // Lever/Meta/Microsoft-style boards ship an empty shell and build the list in JavaScript, so
 // retry once through headless Edge. Skipped when robots blocked the fetch and no override is
 // in force — rendering it would walk straight past the policy the fetch just respected.
 // "Nothing found" has several very different causes and the user can only act on the right one:
 // a mistyped address, a site that refuses automated browsers (AMD answers a plain fetch but 403s
 // headless Edge), or a genuinely unreadable layout. Carry the distinction out to the UI.
 let reason="no_listings";
 if(!found&&(!robotsBlocked||s.robotsOverride))try{const rendered=await renderedHtml(s,page);
   if(rendered.status===404)reason="not_found";else if([401,403,429].includes(rendered.status))reason="blocked";
   // The endpoint is preferred over the rendered page: it survives a layout change, carries real
   // fields, and needs no browser on later runs. It is only offered once it has answered a plain
   // fetch of its own and parsed into listings.
   const api=await provenJsonApi(rendered.captures,s);
   if(api)return{...out("json",{itemsPath:api.itemsPath,fieldMap:api.fieldMap,expectedHost:new URL(api.url).hostname},"high",`Read ${api.jobs.length} listings from the JSON endpoint this page loads (${new URL(api.url).pathname}).`,api.url),sampleJobs:sample(api.jobs)};
   found=await validate(rendered.html,"playwright",page)}
  catch(e){const message=String(e.message||e);reason=/^browser_incompatible/.test(message)?"no_browser":reason;notes.push(`Rendered retry unavailable (${message.split(":")[0]}).`)}
 if(found)return{...out("static-css",{...found.config,...(found.mode==="playwright"?{mode:"playwright"}:{})},found.mode==="playwright"?"medium":"high",`Read ${found.jobs.length} listings straight from the page${found.mode==="playwright"?" after rendering it in Edge":""}.`,page),sampleJobs:found.jobs.slice(0,3).map(j=>({title:j.title,location:j.location,applyUrl:j.applyUrl}))};
 if(sniffed)return sniffed;
 return{...out(null,{},"none","No repeating job listings could be read from this page.",page),unsupported:true,reason}}
async function run(q){const started=Date.now();metrics=blankMetrics();counters.requests=0;counters.skipped=0;counters.filtered=0;seenHashes.clear();published.clear();requestSlots.clear();
 // Only a real scrape streams. A preview and a change check read at most one page and report a
 // sample, so they keep collecting into the returned array and emit from it below.
 publish=q.command==="scrape_source"?item=>emit("job",q.runId,item):null;
 // The caller passes the listing hashes it already holds for this source. An empty list is a full
 // read, which is what a preview, a first run and an explicit full refresh all want.
 const s={...(q.source||{}),__runId:q.runId,__known:new Set(Array.isArray(q.known)?q.known:[]),__titleTerms:Array.isArray(q.titleTerms)?q.titleTerms.map(String).filter(Boolean):[],__defaultRequestKind:q.command==="probe_source"?"probe":q.command==="enrich_source"?"detail":"listing"};if(q.command==="scrape_source"&&q.deferDetails&&!companyDatesNeedDetail(s.adapterId))s.configJson={...(s.configJson||{}),fetchDetail:false};const a=adapters[s.adapterId],cfg=s.configJson||{},required=requiredFor(s.adapterId),warnings=required.filter(key=>!(key in cfg)).map(key=>`Missing recommended field: ${key}`);emit("started",q.runId,{command:q.command,adapter:s.adapterId,adapterVersion:a?.version||null,protocolVersion:V});if(q.command==="capture_job"){
  const target=await guarded(String(q.url||""),s);
  const response=await get(target.toString(),s,0,{metricKind:"probe"});
  const captured=captureFromHtml(response.text,response.url);
  if(!captured.title)throw Error("parse_error: no job title could be read from that page");
  emit("completed",q.runId,terminalPayload({...captured,complete:true,mode:"capture",requests:counters.requests,timingMs:Date.now()-started}));
  return}
 if(q.command==="probe_source"){const d=await timedAdapter(()=>detect(s));emit("completed",q.runId,terminalPayload({...d,adapterVersion:adapters[d.recommendedAdapter]?.version||null,requiredFields:requiredFor(d.recommendedAdapter),warnings:d.notes,mode:"direct",timingMs:Date.now()-started,complete:true}));return}if(!a||a.unsupported||s.kind==="reference")throw Error(s.adapterId==="reference"?"incomplete: reference source cannot be scraped":"incomplete: source needs a verified custom adapter; it remains disabled");if(q.command==="cancel"){emit("cancelled",q.runId,terminalPayload({complete:false}));return}if(q.command==="enrich_source"){const result=await timedAdapter(()=>enrichDirect(s));emit("completed",q.runId,terminalPayload({...result,complete:true,mode:"enrichment",requests:counters.requests,timingMs:Date.now()-started}));return}
 if(q.command==="check_source"){const probe=await timedAdapter(()=>scrape({...s,configJson:{...cfg,maxPages:1,fetchDetail:false,__countFacet:cfg.splitFacet,splitFacet:null}}));
  emit("completed",q.runId,terminalPayload({complete:true,mode:"check",boardTotal:probe.totalExact?probe.total??null:null,boardTotalExact:Boolean(probe.totalExact),fresh:probe.jobs.length,
   discovered:probe.discovered??probe.jobs.length+counters.skipped+counters.filtered,requests:counters.requests,skipped:counters.skipped,filtered:counters.filtered,
   warnings,timingMs:Date.now()-started}));return}
 const readSource=q.command==="test_source"?{...s,configJson:{...cfg,maxPages:1,fetchDetail:false,__countFacet:cfg.splitFacet,splitFacet:null}}:s;
 const o=await timedAdapter(()=>q.command==="capture_session"?browser(s,true):(cfg.mode==="playwright"||s.adapterId==="playwright"?browser(s,false):scrape(readSource))),
  // A scrape already published each job as it was accepted; re-emitting them here would double
  // every listing. A preview shows the first ten and persists none of them.
  jobs=q.command==="scrape_source"?[]:q.command==="test_source"?o.jobs.slice(0,10):o.jobs,emitStarted=clock.now();
 if(q.command==="scrape_source"&&seenHashes.size)emit("seen_batch",q.runId,{listingHashes:[...seenHashes]});
 const progressEvery=Math.max(1,Math.ceil(jobs.length/5));
 for(let i=0;i<jobs.length;i++){if(q.command==="test_source"||i===0||i===jobs.length-1||(i+1)%progressEvery===0)emit("progress",q.runId,{phase:"saving",current:i+1,total:jobs.length,elapsedMs:Date.now()-started});emit("job",q.runId,jobs[i])}
 add("emitMs",emitStarted);
 const discovered=o.discovered??o.jobs.length+counters.skipped+counters.filtered;
 // Last line of defence for every adapter, including the ones whose completeness logic nobody has
 // re-read lately. A board that showed nothing at all is far more often a broken read than an
 // employer with nothing open, and a *complete* empty read is the one outcome that reconciles
 // every stored job for this source to closed. Nothing read proves nothing.
 // ponytail: a source that is genuinely empty therefore never closes its old rows — they stay
 // active until it publishes something again. Showing a stale opening is visible and recoverable;
 // silently deleting a live board is neither. Revisit if a real source sits empty for long.
 const emptyRead=o.complete&&discovered===0;
 emit("completed",q.runId,terminalPayload({command:q.command,complete:o.complete&&!emptyRead,discovered,persisted:q.command==="test_source"?0:o.jobs.length,skipped:counters.skipped,filtered:counters.filtered,requests:counters.requests,boardTotal:o.totalExact?o.total??null:null,boardTotalExact:Boolean(o.totalExact),mode:o.mode||cfg.mode||"direct",warnings:[...warnings,...(o.warnings||[]),...(emptyRead?["This source published no listings at all. That reads as an unfinished run rather than an empty board, so nothing already stored was marked closed."]:[])],timingMs:Date.now()-started,pages:o.pages,...(o.capturedStorageStateBase64?{capturedStorageStateBase64:o.capturedStorageStateBase64}:{})}))}
const code=e=>/^(auth_required|captcha_required|robots_denied|rate_limited|browser_incompatible|selector_broken|parse_error|network_error|incomplete|cancelled)/.exec(String(e?.message||e))?.[1]||"parse_error";
const receive=line=>{let q;try{q=JSON.parse(line);if(q.protocolVersion!==V)throw Error("Unsupported protocol version");if(q.command==="resume"){const pause=pauses.get(q.runId);if(!pause)throw Error("incomplete: no paused scrape");pause.resolve();return}if(q.command==="cancel"){cancelled.add(q.runId);const pause=pauses.get(q.runId);if(pause)pause.reject(Error("cancelled"));return}if(!["probe_source","test_source","scrape_source","check_source","capture_session","capture_job","enrich_source"].includes(q.command))throw Error("Unknown command");cancelled.delete(q.runId);const progressStarted=Date.now(),progressTimer=setInterval(()=>emit("progress",q.runId,{phase:q.command==="enrich_source"?"enriching":"fetching",requests:counters.requests,elapsedMs:Date.now()-progressStarted}),5000);run(q).then(()=>cancelled.delete(q.runId)).catch(e=>{cancelled.delete(q.runId);const failure=code(e);emit(failure==="cancelled"?"cancelled":"failed",q.runId,terminalPayload({code:failure,message:String(e.message||e),complete:false,needsUserAction:["auth_required","captcha_required"].includes(failure)}))})
   // One run per process. Releasing stdin once it ends lets the event loop drain and the process
   // exit on its own; while it ran, stdin had to stay open to receive resume/cancel. Without this
   // a finished worker lingers forever waiting for input that will never come.
   .finally(()=>{clearInterval(progressTimer);process.stdin.pause()})}catch(e){emit("failed",q?.runId||"unknown",terminalPayload({code:"parse_error",message:String(e.message||e),complete:false}))}};
if(process.argv[1]?.replace(/\\/g,"/").endsWith("/worker.mjs"))readline.createInterface({input:process.stdin,crlfDelay:Infinity}).on("line",receive);
