const endpoint=(base,path)=>new URL(path,base).toString();
const jsonHeaders={accept:"application/json","content-type":"application/json"};
const pageNumber=(items,size,current)=>Array.isArray(items)&&items.length===size?{page:current+1}:null;
export const platformRequests={
  apple:(base,{locale="en-us",page=1,query="",filters={},token}={})=>{if(!token)throw Error("incomplete: Apple source requires a CSRF token");return{url:endpoint(base,"/api/v1/search"),method:"POST",headers:{...jsonHeaders,"x-apple-csrf-token":token,locale,browserlocale:locale},body:{query,filters,page,locale,sort:"newest",format:{longDate:"MMMM D, YYYY",mediumDate:"MMM D, YYYY"}},next:()=>null}},
  // Workday CXS is /wday/cxs/{tenant}/{site}/jobs — the site segment is per-tenant
  // ("External", "External_Career", "NVIDIAExternalCareerSite", …) and is discoverable
  // from the tenant's robots.txt sitemap line. Omitting it 404s, so it is required.
  // Asking for an offset at or past the end does not return an empty page on every tenant: PTC's
  // wraps back to the first page, so a full-page response is not proof there is more board left and
  // paging on it never terminated. The published total ends the traversal instead. It is only
  // reported on some pages (PTC sends 0 on the others), so the first real one is carried forward.
  workday:(base,{tenant,site,listingPath,offset=0,limit=20,pageSize,query="",appliedFacets={},boardTotal}={})=>{if(!listingPath&&!(tenant&&site))throw Error("incomplete: Workday source requires listingPath, or both tenant and site");const size=pageSize||limit;return{url:endpoint(base,listingPath||`/wday/cxs/${encodeURIComponent(tenant)}/${encodeURIComponent(site)}/jobs`),method:"POST",headers:jsonHeaders,body:{appliedFacets,limit:size,offset,searchText:query},next:r=>{
   if(!Array.isArray(r.jobPostings)||r.jobPostings.length<size)return null;
   const reported=Number(r.total),total=reported>0?reported:boardTotal,next=offset+size;
   if(Number.isFinite(total)&&total>0&&next>=total)return null;
   return Number.isFinite(total)?{offset:next,boardTotal:total}:{offset:next}}}},
  // Eightfold's readable search paths are the two its own robots.txt allow-lists on every live
  // tenant: "/api/pcsx/search" (current, {data:{positions,count}}) and "/api/apply/v2/jobs"
  // (legacy, {positions,count}). Both answer anonymously; the earlier "/api/career_hub" default
  // did not. Tenants differ on which one they serve — stmicroelectronics 403s PCSX and answers
  // legacy — so "eightfoldApi" selects. Both require the tenant's own "domain" (422 without it), which
  // is the employer domain, not the careers host: careers.gf.com uses globalfoundries.com.
  eightfold:(base,{listingPath,eightfoldApi="pcsx",domain,start=0,pageSize=10,cursor,query="",location="",sortBy="relevance"}={})=>{
   if(!domain&&!listingPath)throw Error("incomplete: Eightfold source requires domain");
   const legacy=eightfoldApi==="legacy",url=new URL(listingPath||(legacy?"/api/apply/v2/jobs":"/api/pcsx/search"),base);
   if(domain)url.searchParams.set("domain",domain);
   if(legacy){url.searchParams.set("start",String(start));url.searchParams.set("num",String(pageSize))}
   else{url.searchParams.set("query",query);url.searchParams.set("location",location);url.searchParams.set("start",String(start));if(sortBy)url.searchParams.set("sort_by",sortBy)}
   if(cursor)url.searchParams.set("cursor",cursor);
   return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>{
    const nextCursor=r.nextCursor||r.data?.nextCursor,items=r.data?.positions||r.positions,total=Number(r.data?.count??r.count);
    if(nextCursor)return{cursor:nextCursor,start:start+(Array.isArray(items)?items.length:0)};
    if(!Array.isArray(items)||!items.length)return null;
    const next=start+items.length;
    return Number.isFinite(total)&&next>=total?null:{start:next}}}},
  icims:(base,{listingPath="/jobs/search",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("search",query);url.searchParams.set("page",String(page));url.searchParams.set("pageSize",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  "talentbrew-jibe":(base,{listingPath="/jobs",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("size",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  phenom:(base,{listingPath="/search-jobs",page=0,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("limit",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.data?.jobs,pageSize,page)}},
  // The four boards below are hosted: every employer on them answers at one shared API host, keyed
  // by the employer's own board token. That is why they take a token rather than a listingPath —
  // the base URL is the human-readable board an applicant sees, and the request goes to the API.
  //
  // Greenhouse hands back the entire board in one response, so there is no page to ask for. The
  // listing carries no description; the per-job endpoint does, and it is what fetchDetail reads.
  greenhouse:(base,{token}={})=>{if(!token)throw Error("incomplete: Greenhouse source requires token");
   return{url:`https://boards-api.greenhouse.io/v1/boards/${encodeURIComponent(token)}/jobs`,method:"GET",headers:{accept:"application/json"},next:()=>null}},
  // Ashby also answers whole-board, and unlike Greenhouse its listing already carries the
  // description, so a scrape of an Ashby board is exactly one request.
  ashby:(base,{token}={})=>{if(!token)throw Error("incomplete: Ashby source requires token");
   return{url:`https://api.ashbyhq.com/posting-api/job-board/${encodeURIComponent(token)}`,method:"GET",headers:{accept:"application/json"},next:()=>null}},
  // Lever pages with skip/limit and caps a page at 100. Its listing carries the description too.
  lever:(base,{token,pageSize=100,skip=0}={})=>{if(!token)throw Error("incomplete: Lever source requires token");
   const size=Math.min(Number(pageSize)||100,100);
   const host=new URL(base).hostname==="jobs.eu.lever.co"?"api.eu.lever.co":"api.lever.co";
   return{url:`https://${host}/v0/postings/${encodeURIComponent(token)}?mode=json&limit=${size}&skip=${skip}`,method:"GET",headers:{accept:"application/json"},
    next:r=>Array.isArray(r)&&r.length===size?{skip:skip+size}:null}},
  // Oracle Recruiting (ORC). The employer's careers page is a vanity host in front of an Oracle
  // Fusion pod, and both halves are needed: apiHost is the pod that answers, siteNumber is the
  // careers site on it. The vanity host is what job links are built from, so it stays the base URL.
  oracle:(base,{apiHost,siteNumber,pageSize=100,offset=0,query=""}={})=>{
   if(!apiHost||!siteNumber)throw Error("incomplete: Oracle source requires apiHost and siteNumber");
   const finder=`findReqs;siteNumber=${siteNumber},limit=${pageSize},offset=${offset},sortBy=POSTING_DATES_DESC${query?`,keyword=${encodeURIComponent(query)}`:""}`;
   const url=`https://${apiHost}/hcmRestApi/resources/latest/recruitingCEJobRequisitions?onlyData=true&expand=requisitionList.secondaryLocations&finder=${finder}`;
   return{url,method:"GET",headers:{accept:"application/json"},next:r=>{
    const search=r?.items?.[0],list=search?.requisitionList;
    if(!Array.isArray(list)||!list.length)return null;
    const next=offset+list.length,total=Number(search.TotalJobsCount);
    return Number.isFinite(total)&&next>=total?null:{offset:next}}}}
};
// Hosted boards each publish their own field names. Rather than teach the shared scrape loop four
// more shapes, each family maps one raw record onto the generic keys that loop already reads.
// Applied to the listing row AND to the row merged with its detail payload, so a field that only
// the detail page carries (Greenhouse's description) is mapped just the same.
const ghDetail=(token,id)=>token&&id!=null?`https://boards-api.greenhouse.io/v1/boards/${encodeURIComponent(token)}/jobs/${encodeURIComponent(id)}`:null;
export const platformItems={
 // PCSX search omits descriptions and detail URLs. Its public frontend reads this endpoint
 // when a vacancy opens; legacy Eightfold and explicitly provided detail links retain theirs.
 eightfold:(x,cfg={},base="")=>({...x,description:x.description||x.jobDescription||x.job_description,
  detailUrl:x.detailUrl||((cfg.eightfoldApi||"pcsx")==="pcsx"&&cfg.domain&&x.id!=null&&base
   ?new URL(`/api/pcsx/position_details?position_id=${encodeURIComponent(x.id)}&domain=${encodeURIComponent(cfg.domain)}`,base).toString():null)}),
 greenhouse:(x,cfg={})=>({...x,externalId:String(x.id??x.internal_job_id??""),url:x.absolute_url,applyUrl:x.absolute_url,
  location:x.location?.name??x.location,company:x.company_name,
  // first_published is when the opening appeared; updated_at moves whenever anything is edited,
  // so using it would make an untouched year-old posting look new.
  postedAt:x.first_published||x.updated_at,description:x.content,detailUrl:ghDetail(cfg.token,x.id)}),
 ashby:x=>({...x,externalId:String(x.id??""),url:x.jobUrl,
  // applyUrl on Ashby is the application form, not the posting; the posting is what gets stored.
  applyUrl:x.jobUrl,location:[x.location,...(x.secondaryLocations||[]).map(l=>l?.location)].filter(Boolean),
  postedAt:x.publishedAt,description:x.descriptionPlain||x.descriptionHtml,descriptionHtml:x.descriptionHtml,
  workMode:x.isRemote?"remote":x.workplaceType||null}),
 // Lever calls the job title "text"; nothing downstream would find it under that name.
 lever:x=>({...x,externalId:String(x.id??""),title:x.text,url:x.hostedUrl,applyUrl:x.hostedUrl,
  location:x.categories?.allLocations?.length?x.categories.allLocations:x.categories?.location,
  postedAt:x.createdAt,
  description:[x.descriptionPlain||x.description,...(x.lists||[]).flatMap(section=>[section.text,section.content]),x.additionalPlain||x.additional].filter(Boolean).join("\n\n"),
  descriptionHtml:[x.description||x.descriptionPlain,...(x.lists||[]).flatMap(section=>[section.text,section.content]),x.additional||x.additionalPlain].filter(Boolean).join("\n\n"),
  workMode:x.workplaceType||null}),
 oracle:(x,cfg={},base="")=>({...x,externalId:String(x.Id??""),
  url:base?new URL(`job/${encodeURIComponent(x.Id)}`,base.endsWith("/")?base:`${base}/`).toString():null,
  title:x.Title,location:[x.PrimaryLocation,...(x.secondaryLocations||[]).map(l=>l?.LocationName||l?.Name)].filter(Boolean),
  postedAt:x.ExternalPostedStartDate||x.PostedDate,
  description:[x.ExternalDescriptionStr,x.ExternalResponsibilitiesStr,x.ExternalQualificationsStr].filter(Boolean).join("\n\n")||x.ShortDescriptionStr,
  detailUrl:cfg.apiHost&&cfg.siteNumber&&x.Id!=null
   ?`https://${cfg.apiHost}/hcmRestApi/resources/latest/recruitingCEJobRequisitionDetails?onlyData=true&expand=all&finder=ById;Id=%22${encodeURIComponent(x.Id)}%22,siteNumber=${cfg.siteNumber}`:null})
};
export const normalizeItem=(adapter,item,config={},base="")=>platformItems[adapter]?platformItems[adapter](item,config,base):item;
export function requestFor(adapter,base,config){const build=platformRequests[adapter];if(!build)throw Error(`unsupported adapter request: ${adapter}`);return build(base,config)};
