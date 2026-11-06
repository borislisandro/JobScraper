// Finds an employer's Workday tenant/site without a careers URL: the tenant's robots.txt names the
// site in its Sitemap line, and the CXS endpoint then answers a "Portugal" text search with a total.
// A tenant that answers is only a lead until its rows show Portuguese locations.
//
//   node scripts/probe-workday-pt.mjs adidas vestas bnpparibas
//
// Read-only and unauthenticated.
const UA={"user-agent":"JobScraper/1.0 local personal use"};
const POD=["wd1","wd3","wd5","wd103","wd102","wd501","wd12"];
const get=async(url,init={})=>{try{const response=await fetch(url,{headers:UA,signal:AbortSignal.timeout(10_000),...init});return response.ok?response:null}catch{return null}};

async function tenant(slug){
 for(const pod of POD){
  const robots=await get(`https://${slug}.${pod}.myworkdayjobs.com/robots.txt`);
  if(!robots)continue;
  const text=await robots.text(),site=/Sitemap:\s*https?:\/\/[^/\s]+\/([^/\s]+)\/siteMap\.xml/i.exec(text)?.[1]||/Sitemap:\s*https?:\/\/[^/\s]+\/[a-z-]+\/([^/\s]+)\/siteMap\.xml/i.exec(text)?.[1];
  if(site)return{pod,site};
 }
 return null;
}
async function search(slug,found,searchText){
 const response=await get(`https://${slug}.${found.pod}.myworkdayjobs.com/wday/cxs/${slug}/${found.site}/jobs`,{method:"POST",headers:{...UA,"content-type":"application/json"},body:JSON.stringify({appliedFacets:{},limit:20,offset:0,searchText})});
 return response?response.json():null;
}
const names=process.argv.slice(2);
const queue=[...names];
await Promise.all(Array.from({length:8},async()=>{
 while(queue.length){
  const name=queue.shift(),base=name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]/g,"");
  const found=await tenant(base);
  if(!found){console.log(`### ${name}\n  (no Workday tenant under ${base})`);continue}
  const data=await search(base,found,"Portugal");
  if(!data){console.log(`### ${name}\n  tenant ${base}.${found.pod} site=${found.site} but search refused`);continue}
  const sample=(data.jobPostings||[]).slice(0,4).map(p=>`${p.title} [${p.locationsText}]`);
  console.log(`### ${name}\n  workday ${base}.${found.pod} site=${found.site} Portugal-text total=${data.total}${sample.map(s=>`\n     e.g. ${s}`).join("")}`);
 }
}));
