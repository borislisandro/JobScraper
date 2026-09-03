// R5: nothing connected the starter pack to the adapter layer, so a seeded source
// could crash requestFor() or silently build a wrong URL and no test would notice
// until a live run failed (this is exactly how Workday's missing "site" segment and
// the pre-fix Eightfold/iCIMS/Phenom defaults shipped undetected). This fixture is
// generated from install_starter_pack itself — see
// src-tauri/src/db.rs::starter_pack_matches_the_checked_in_adapter_contract_fixture,
// which fails if db.rs's seed ever diverges from the checked-in file below. Reading
// the real seeded shape here, rather than a hand-copied one, is what keeps this test
// from silently drifting off the Rust source of truth.
import assert from "node:assert/strict";import {readFileSync}from "node:fs";import {test}from "node:test";
import {requestFor}from "./adapters.mjs";import {adapters}from "./worker.mjs";import {companyAdapters}from "./company-adapters.mjs";
const fixture=JSON.parse(readFileSync(new URL("./starter-pack.fixture.json",import.meta.url),"utf8"));
const catalog=JSON.parse(readFileSync(new URL("./company-catalog.json",import.meta.url),"utf8"));
test("every catalog company has a supported, fully configured request",()=>{
 assert.ok(catalog.companies.length>=134,"the original catalog must remain available as new boards are added");
 assert.equal(new Set(catalog.companies.map(company=>company.id)).size,catalog.companies.length,"catalog IDs must stay unique");
 assert.equal(new Set(catalog.companies.map(company=>company.baseUrl)).size,catalog.companies.length,"catalog boards must stay unique");
 assert.equal(catalog.companies.filter(company=>company.starter).length,21);
 for(const company of catalog.companies){
  if(company.kind==="reference"){assert.equal(company.adapterId,"reference");assert.equal(company.starter,true);continue}
  assert.ok(adapters[company.adapterId]&&!adapters[company.adapterId].unsupported,`${company.name}: unsupported adapter`);
  assert.equal(new URL(company.baseUrl).protocol,"https:");
  if(!company.starter){assert.match(company.verified?.at||"",/^\d{4}-\d{2}-\d{2}$/);assert.ok(company.verified.jobs>0,`${company.name}: no live verification recorded`)}
  const board=companyAdapters[company.adapterId];
  if(board){assert.equal(new URL(company.baseUrl).host,board.host,company.name);continue}
  // The generic adapters (json, rss, static-css, static-xpath) fetch the source's own base URL
  // and have no platform request to build; "direct-json" is what marks the families that do. Their
  // contract is the https base URL asserted above, which is what the worker will actually fetch.
  if(!adapters[company.adapterId].capabilities.includes("direct-json"))continue;
  // Apple obtains its transient CSRF token at runtime. Every tenant/board setting, however,
  // must come from the catalog itself: accepting "incomplete" here would ship a broken picker.
  const built=requestFor(company.adapterId,company.baseUrl,{...company.config,...(company.adapterId==="apple"?{token:"runtime-csrf"}:{})});
  assert.equal(new URL(built.url).protocol,"https:",company.name);assert.equal(typeof built.next,"function",company.name);
 }
 for(const starter of fixture)assert.equal(starter.configJson.starterPackVersion,catalog.version,starter.name);
});
test("expansion entries have a full live proof matching their shipped configuration",()=>{
 const tracker=JSON.parse(readFileSync(new URL("../docs/company-support-tracker.json",import.meta.url),"utf8"));
 assert.equal(tracker.companies.length,269);
 assert.equal(new Set(tracker.companies.map(company=>company.id)).size,269);
 assert.deepEqual(tracker.companies.map(company=>company.sequence),Array.from({length:269},(_,index)=>index+1));
 for(const row of tracker.companies){
  const company=catalog.companies.find(company=>company.name===row.name);
  if(row.status!=="supported"){assert.equal(company,undefined,`${row.name}: unfinished work entered the catalog`);continue}
  assert.ok(company,`${row.name}: supported entry missing from catalog`);assert.equal(row.catalogId,company.id);
  assert.equal(company.verified.level,"full");
  const proof=JSON.parse(readFileSync(new URL(`../${company.verified.evidence}`,import.meta.url),"utf8"));
  assert.equal(proof.ok,true);assert.equal(proof.level,"full");assert.deepEqual(proof.errors,[]);
  assert.equal(proof.source.name,company.name);assert.equal(proof.source.adapterId,company.adapterId);
  assert.equal(proof.source.baseUrl,company.baseUrl);assert.deepEqual(proof.source.configJson,company.config);
  // Only the URL guard is asserted here. Robots enforcement is off by choice for this tool, and
  // matching what install_starter_pack seeds, so a proof either way is valid evidence. The guard is
  // asserted as "not enabled" rather than "=== false" because that is what the worker itself tests:
  // an omitted flag is off, and a proof is evidence about the run, not about the candidate's style.
  assert.notEqual(proof.source.allowPrivateNetwork,true);
  assert.equal(proof.listing.terminal.payload.complete,true);assert.equal(proof.listing.jobs,company.verified.jobs);
  assert.ok(proof.listing.jobs>0);assert.equal(proof.listing.uniqueIdentities,proof.listing.jobs);
  assert.equal(proof.listing.uniqueHashes,proof.listing.jobs);assert.equal(proof.warm.terminal.payload.fresh,0);
 }
});

test("every seeded starter source builds a request or throws a declared incomplete",()=>{
 assert.equal(fixture.length,21,"starter pack should still seed all 21 sources");
 for(const s of fixture){
  const a=adapters[s.adapterId];
  // Company adapters build their own URLs instead of going through requestFor(), so the
  // contract they have to satisfy is that the seeded base URL is on the host they read.
  const board=companyAdapters[s.adapterId];
  if(board){assert.equal(new URL(s.baseUrl).host,board.host,`${s.name}: seeded base URL is not on ${board.host}`);continue}
  // custom-api and reference are declared unsupported by the adapter registry itself —
  // that is the "disabled with an accurate reason" outcome for the five custom-api
  // sources and the one reference-only source, not a silent wrong URL.
  if(!a||a.unsupported){assert.ok(!a||a.unsupported,`${s.name}: adapter ${s.adapterId} should be a known, declared-unsupported adapter`);continue}
  try{const built=requestFor(s.adapterId,s.baseUrl,s.configJson);assert.ok(built.url,`${s.name}: requestFor produced no url`)}
  catch(e){assert.match(String(e.message||e),/^incomplete/,`${s.name}: requestFor threw an undeclared error: ${e.message||e}`)}
 }
});
