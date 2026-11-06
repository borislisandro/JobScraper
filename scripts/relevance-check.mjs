// Counts chip-design roles (DV, DFT, RTL, ASIC, FPGA, SoC...) on a candidate board, overall and in the target
// countries, and records the result in the candidate's discovery.relevance. It decides starter promotion.
//   node scripts/relevance-check.mjs docs/company-candidates/<id>.json [--write]
import {readFile,writeFile}from "node:fs/promises";
import {runWorker}from "./worker-proof.mjs";
const [path,...flags]=process.argv.slice(2);if(!path)throw Error("Usage: relevance-check.mjs <candidate.json> [--write]");
// "DV" alone also matches the UK security clearance "DV Cleared", so a clearance mention is excluded first.
const ROLE=/\b(DFT|RTL|ASIC|FPGA|SoC|UVM|verification|design for test(ability)?|digital (design|ic|asic)|logic design|physical design|silicon (validation|design)|ic design|mixed[- ]signal|microarchitect\w*|DV)\b/i;
const NOT_ROLE=/DV[ -]cleared|\bDV\b.*clearance|(?:supplier|vendor|cleaning|quality|test) verification|identity verification|background verification/i;
const TARGET=/\b(portugal|lisbon|lisboa|porto|spain|barcelona|madrid|valencia|sevilla|seville|france|paris|grenoble|sophia|valbonne|toulouse|massy|meylan|netherlands|eindhoven|amsterdam|rotterdam|delft|vught|ireland|dublin|cork|limerick|austria|vienna|graz|villach|linz|premst|germany|munich|m[uü]nchen|dresden|berlin|hamburg|stuttgart|dortmund|nuremberg|aachen|frankfurt|heilbronn|switzerland|zurich|zürich|z[uü]rich|geneva|lausanne|neuch|basel|schaffhausen|united kingdom|uk|england|scotland|london|cambridge|bristol|oxford|reading|edinburgh|glasgow|manchester|newbury|kings langley|durham|bath|belfast|cardiff|leeds|chelmsford|gb|emea|europe)\b|,\s*(PT|ES|FR|NL|IE|AT|DE|GB|CH)\b/i;
const source=JSON.parse(await readFile(path,"utf8")),result=await runWorker(source,"scrape_source",{known:[],deferDetails:true});
if(result.error||result.terminal?.event!=="completed")throw Error(result.error||result.terminal?.payload?.message||"scrape failed");
const hits=result.jobs.filter(job=>ROLE.test(job.title)&&!NOT_ROLE.test(job.title)),inTarget=hits.filter(job=>TARGET.test(job.location||"")||TARGET.test(job.title));
const relevance={checkedAt:new Date().toISOString().slice(0,10),boardJobs:result.jobs.length,roleTitles:hits.length,roleTitlesInTargetCountries:inTarget.length,
 examples:inTarget.slice(0,6).map(job=>`${job.title} — ${job.location||"?"}`)};
console.log(JSON.stringify({id:source.id,...relevance}));
for(const job of hits)console.log(`${inTarget.includes(job)?"T":"-"}\t${job.title}\t${job.location||""}`);
if(flags.includes("--write")){source.discovery={...source.discovery,relevance};await writeFile(path,JSON.stringify(source,null,1)+"\n")}
