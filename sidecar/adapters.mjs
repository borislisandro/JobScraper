const endpoint=(base,path)=>new URL(path,base).toString();
const jsonHeaders={accept:"application/json","content-type":"application/json"};
const pageNumber=(items,size,current)=>Array.isArray(items)&&items.length===size?{page:current+1}:null;
export const platformRequests={
  // Workday CXS is /wday/cxs/{tenant}/{site}/jobs — the site segment is per-tenant
  // ("External", "External_Career", "NVIDIAExternalCareerSite", …) and is discoverable
  // from the tenant's robots.txt sitemap line. Omitting it 404s, so it is required.
  workday:(base,{tenant,site,listingPath,offset=0,limit=20,pageSize,query=""}={})=>{if(!listingPath&&!(tenant&&site))throw Error("incomplete: Workday source requires listingPath, or both tenant and site");const size=pageSize||limit;return{url:endpoint(base,listingPath||`/wday/cxs/${encodeURIComponent(tenant)}/${encodeURIComponent(site)}/jobs`),method:"POST",headers:jsonHeaders,body:{appliedFacets:{},limit:size,offset,searchText:query},next:r=>Array.isArray(r.jobPostings)&&r.jobPostings.length===size?{offset:offset+size}:null}},
  // The old default "/api/jobs" is robots-denied on every live Eightfold tenant checked
  // (careers.micron.com, careers.qualcomm.com both publish "Disallow: /" with an Allow
  // list). "/api/career_hub" is the allow-listed search path; it still 401s/redirects to
  // login for anonymous requests on these tenants, so a source stays disabled until a
  // captured session (sessionCookies/requestHeaders) is attached.
  eightfold:(base,{listingPath="/api/career_hub",page=0,pageSize=20,cursor,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("query",query);url.searchParams.set("page",String(page));url.searchParams.set("pageSize",String(pageSize));if(cursor)url.searchParams.set("cursor",cursor);return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>r.nextCursor||r.data?.nextCursor?{cursor:r.nextCursor||r.data.nextCursor,page:page+1}:pageNumber(r.positions||r.data?.positions,pageSize,page)}},
  icims:(base,{listingPath="/jobs/search",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("search",query);url.searchParams.set("page",String(page));url.searchParams.set("pageSize",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  "talentbrew-jibe":(base,{listingPath="/jobs",page=1,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("size",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.searchResults,pageSize,page)}},
  phenom:(base,{listingPath="/search-jobs",page=0,pageSize=20,query=""}={})=>{const url=new URL(listingPath,base);url.searchParams.set("keyword",query);url.searchParams.set("page",String(page));url.searchParams.set("limit",String(pageSize));return{url:url.toString(),method:"GET",headers:{accept:"application/json"},next:r=>pageNumber(r.jobs||r.data?.jobs,pageSize,page)}}
};
export function requestFor(adapter,base,config){const build=platformRequests[adapter];if(!build)throw Error(`unsupported adapter request: ${adapter}`);return build(base,config)};
