// Proves the Workable candidates and admits those that pass, once Workable stops rate-limiting this
// machine (Cloudflare error 1015 after heavy probing). Exits 2 while still blocked, so it can be polled.
//   node docs/ds-portugal/finish-workable.mjs
import {spawnSync}from "node:child_process";
const root=new URL("../../",import.meta.url).pathname;
const probe=await fetch("https://apply.workable.com/api/v1/widget/accounts/knok?details=true",{headers:{"user-agent":"JobScraper/1.0 local personal use"}}).catch(()=>null);
const body=probe?await probe.text():"";
if(!probe?.ok||!body.trimStart().startsWith("{")){console.log("Workable is still blocking this machine.");process.exit(2)}
const run=(args)=>spawnSync(process.execPath,args,{cwd:root,encoding:"utf8"});
const ids=run(["docs/ds-portugal/make-candidates-5.mjs"]).stdout.trim().split(/\s+/).filter(id=>!["critical-software"].includes(id));
for(const id of ids){
 const result=run(["scripts/verify-company-catalog.mjs","--full","--candidate",`docs/ds-portugal/candidates/${id}.json`,"--report",`docs/company-proofs/ds-pt-${id}.json`]);
 console.log((result.stdout.split("\n").find(line=>line.startsWith("{"))||result.stderr.trim()).slice(0,260));
 await new Promise(resolve=>setTimeout(resolve,4000));
}
console.log(run(["docs/ds-portugal/admit-all.mjs"]).stdout);
