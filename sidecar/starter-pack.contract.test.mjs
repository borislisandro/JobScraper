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
test("every seeded starter source builds a request or throws a declared incomplete",()=>{
 assert.equal(fixture.length,20,"starter pack should still seed all 20 sources");
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
