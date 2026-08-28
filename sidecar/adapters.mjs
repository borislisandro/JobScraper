export const platformRequests={
  workday:(base,{offset=0,limit=20,query="",location=""}={})=>({url:new URL("/wday/cxs/jobs",base).toString(),method:"POST",headers:{"content-type":"application/json"},body:{limit,offset,searchText:query,locations:location?[location]:[]},page:r=>r.jobPostings?.length===limit?offset+limit:null}),
  eightfold:(base,{page=0,pageSize=20,query=""}={})=>({url:new URL(`/api/jobs?query=${encodeURIComponent(query)}&page=${page}&pageSize=${pageSize}`,base).toString(),method:"GET",headers:{accept:"application/json"},page:r=>r.positions?.length===pageSize?page+1:null}),
  icims:(base,{page=1,pageSize=20,query=""}={})=>({url:new URL(`/jobs/search?search=${encodeURIComponent(query)}&page=${page}&pageSize=${pageSize}`,base).toString(),method:"GET",headers:{accept:"application/json"},page:r=>r.jobs?.length===pageSize?page+1:null}),
  "talentbrew-jibe":(base,{page=1,pageSize=20,query=""}={})=>({url:new URL(`/jobs?keyword=${encodeURIComponent(query)}&page=${page}&size=${pageSize}`,base).toString(),method:"GET",headers:{accept:"application/json"},page:r=>r.jobs?.length===pageSize?page+1:null}),
  phenom:(base,{page=0,pageSize=20,query=""}={})=>({url:new URL(`/search-jobs?keyword=${encodeURIComponent(query)}&page=${page}&limit=${pageSize}`,base).toString(),method:"GET",headers:{accept:"application/json"},page:r=>r.jobs?.length===pageSize?page+1:null})
};
export function requestFor(adapter,base,config){const build=platformRequests[adapter];if(!build)throw Error(`unsupported adapter request: ${adapter}`);return build(base,config)};
