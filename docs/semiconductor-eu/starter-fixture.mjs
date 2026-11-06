// Rebuilds sidecar/starter-pack.fixture.json the way db.rs::install_starter_pack seeds it, for machines that cannot build the
// Windows-only Rust crate. The authoritative regeneration remains
//   UPDATE_STARTER_PACK_FIXTURE=1 cargo test --manifest-path src-tauri/Cargo.toml starter_pack_matches_the_checked_in_adapter_contract_fixture
// and --check proves this script reproduces whatever fixture is committed, so the two cannot disagree silently.
//   node docs/semiconductor-eu/starter-fixture.mjs [--check <catalog.json> <fixture.json>]
import {readFileSync,writeFileSync}from "node:fs";
// serde_json (without preserve_order) writes object keys in sorted order at every depth.
const sorted=value=>Array.isArray(value)?value.map(sorted):value&&typeof value==="object"?Object.fromEntries(Object.keys(value).sort().map(key=>[key,sorted(value[key])])):value;
// install_starter_pack finishes by rewriting every stored source whose host is in its `upgrades` table, so those settings are part of
// what a fresh install holds. The table is read from db.rs itself rather than copied, so editing it there is enough.
const upgrades=(()=>{const source=readFileSync(new URL("../../src-tauri/src/db.rs",import.meta.url),"utf8"),start=source.indexOf("let upgrades:"),text=source.slice(start,source.indexOf("];",start)),table=new Map;
 for(const m of text.matchAll(/^\s*\("([^"]+)", "([^"]+)", "((?:[^"\\]|\\.)*)", "([^"]*)"\),$/gm))table.set(m[1],{extra:JSON.parse(m[3].replaceAll('\\"','"')),relocation:m[4],adapter:m[2]});
 return table})();
const withUpgrade=(company,config)=>{const host=new URL(company.baseUrl).hostname.toLowerCase(),upgrade=upgrades.get(host);if(!upgrade)return config;
 const next={...config,...upgrade.extra};
 if(upgrade.adapter==="sk-hynix"||upgrade.adapter==="u-blox")delete next.expectedHost;
 else if(upgrade.relocation)next.expectedHost=new URL(upgrade.relocation).hostname;
 return next};
export const build=catalog=>catalog.companies.filter(company=>company.starter).sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0).map(company=>sorted({name:company.name,baseUrl:company.baseUrl,adapterId:company.adapterId,
 configJson:withUpgrade(company,{schemaVersion:"1.1.0",starterPackVersion:catalog.version,adapterVersion:"1.1.0",mode:"direct",...company.config})}));
const args=process.argv.slice(2),runDirectly=import.meta.url===new URL(`file://${process.argv[1]}`).href;
if(runDirectly&&args[0]==="--check"){const [catalogPath,fixturePath]=args.slice(1),actual=JSON.stringify(build(JSON.parse(readFileSync(catalogPath,"utf8"))),null,2),expected=JSON.stringify(JSON.parse(readFileSync(fixturePath,"utf8")),null,2);
 if(actual!==expected){console.error("MISMATCH");process.exit(1)}console.log(`reproduces ${fixturePath}`)}
else if(runDirectly){const root=new URL("../../sidecar/",import.meta.url);writeFileSync(new URL("starter-pack.fixture.json",root),JSON.stringify(build(JSON.parse(readFileSync(new URL("company-catalog.json",root),"utf8"))),null,2)+"\n");console.log("fixture written")}
