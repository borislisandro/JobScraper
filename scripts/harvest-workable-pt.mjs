// Harvests employer accounts from Workable's public cross-company job search for Portuguese
// locations, so a board is found by what it is hiring instead of by guessing a company name.
//
//   node scripts/harvest-workable-pt.mjs > docs/ds-portugal/workable-harvest.json
//
// Read-only and unauthenticated; one request per page, paced.
const UA={"user-agent":"JobScraper/1.0 local personal use",accept:"application/json"};
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const QUERIES=["data scientist","data analyst","machine learning","AI engineer","artificial intelligence","business intelligence","data engineer","analytics","NLP","computer vision","MLOps","LLM"];
const PLACES=["Portugal","Porto","Lisbon","Braga","Aveiro","Coimbra"];
const accounts=new Map();
for(const query of QUERIES)for(const location of PLACES){
 let token="",pages=0;
 do{
  const url=`https://jobs.workable.com/api/v1/jobs?query=${encodeURIComponent(query)}&location=${encodeURIComponent(location)}&limit=20${token?`&pageToken=${encodeURIComponent(token)}`:""}`;
  let data;try{const response=await fetch(url,{headers:UA,signal:AbortSignal.timeout(20_000)});data=response.ok?await response.json():null}catch{data=null}
  if(!data?.jobs)break;
  for(const job of data.jobs){
   const company=job.company||{};const key=company.id||company.title;if(!key)continue;
   const place=[job.location?.city,job.location?.countryName||job.location?.country].filter(Boolean).join(", ");
   if(!/portugal|\bPT\b/i.test(job.location?.countryName||job.location?.country||place))continue;
   const entry=accounts.get(key)||{company:company.title,id:company.id,url:company.url,jobs:new Set(),samples:[]};
   if(!entry.jobs.has(job.id)){entry.jobs.add(job.id);if(entry.samples.length<4)entry.samples.push(`${job.title} [${place}]`)}
   accounts.set(key,entry);
  }
  token=data.nextPageToken||"";pages++;await sleep(300);
 }while(token&&pages<4);
}
console.log(JSON.stringify([...accounts.values()].map(a=>({...a,jobs:a.jobs.size})).sort((a,b)=>b.jobs-a.jobs),null,1));
