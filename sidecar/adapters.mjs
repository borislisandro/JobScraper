const endpoint=(base,path)=>new URL(path,base).toString();
const jsonHeaders={accept:"application/json","content-type":"application/json"};
const pageNumber=(items,size,current)=>Array.isArray(items)&&items.length===size?{page:current+1}:null;
export const platformRequests={
  apple:(base,{locale="en-us",page=1,query="",filters={},token}={})=>{if(!token)throw Error("incomplete: Apple source requires a CSRF token");return{url:endpoint(base,"/api/v1/search"),method:"POST",headers:{...jsonHeaders,"x-apple-csrf-token":token,locale,browserlocale:locale},body:{query,filters,page,locale,sort:"newest",format:{longDate:"MMMM D, YYYY",mediumDate:"MMM D, YYYY"}},next:()=>null}},
  // Workday CXS is /wday/cxs/{tenant}/{site}/jobs — the site segment is per-tenant
  // ("External", "External_Career", "NVIDIAExternalCareerSite", …) and is discoverable
  // from the tenant's robots.txt sitemap line. Omitting it 404s, so it is required.
  workday:(base,{tenant,site,listingPath,offset=0,limit=20,pageSize,query="",appliedFacets={}}={})=>{if(!listingPath&&!(tenant&&site))throw Error("incomplete: Workday source requires listingPath, or both tenant and site");const size=pageSize||limit;return{url:endpoint(base,listingPath||`/wday/cxs/${encodeURIComponent(tenant)}/${encodeURIComponent(site)}/jobs`),method:"POST",headers:jsonHeaders,body:{appliedFacets,limit:size,offset,searchText:query},next:r=>Array.isArray(r.jobPostings)&&r.jobPostings.length===size?{offset:offset+size}:null}},
  // Eightfold's readable search paths are the two its own robots.txt allow-lists on every live
  // tenant: "/api/pcsx/search" (current, {data:{positions,count}}) and "/api/apply/v2/jobs"
  // (legacy, {positions,count}). Both answer anonymously; the earlier "/api/career_hub" default
  // did not. Tenants differ on which one they serve — stmicroelectronics 403s PCSX and answers
  // legacy — so "eightfoldApi" selects. Both require the tenant's own "domain" (422 without it), which
  // is the employer domain, not the careers host: careers.gf.com uses globalfoundries.com.
  eightfold:(base,{listingPath,eightfoldApi="pcsx",domain,start=0,pageSize=10,cursor,query="",location=""}={})=>{
   if(!domain&&!listingPath)throw Error("incomplete: Eightfold source requires domain");
   const legacy=eightfoldApi==="legacy",url=new URL(listingPath||(legacy?"/api/apply/v2/jobs":"/api/pcsx/search"),base);
   if(domain)url.searchParams.set("domain",domain);
   if(legacy){url.searchParams.set("start",String(start));url.searchParams.set("num",String(pageSize))}
   else{url.searchParams.set("query",query);url.searchParams.set("location",location);url.searchParams.set("start",String(start));url.searchParams.set("sort_by","relevance")}
   if(cursor)url.searchParams.set("cursor",cursor);
   return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>{
    const nextCursor=r.nextCursor||r.data?.nextCursor,items=r.data?.positions||r.positions,total=Number(r.data?.count??r.count);
    if(nextCursor)return{cursor:nextCursor,start:start+(Array.isArray(items)?items.length:0)};
    if(!Array.isArray(items)||!items.length)return null;
    const next=start+items.length;
    return Number.isFinite(total)&&next>=total?null:{start:next}}}},
  icims:(base,{listingPath="/jobs/search",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("search",query);url.searchParams.set("page",String(page));url.searchParams.set("pageSize",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  "talentbrew-jibe":(base,{listingPath="/jobs",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("size",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  phenom:(base,{listingPath="/search-jobs",page=0,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("limit",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.data?.jobs,pageSize,page)}}
};
export function requestFor(adapter,base,config){const build=platformRequests[adapter];if(!build)throw Error(`unsupported adapter request: ${adapter}`);return build(base,config)};
