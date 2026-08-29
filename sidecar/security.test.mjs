import assert from "node:assert/strict";
import { test } from "node:test";
import { jitterMs, makeUrlGuard, parseRobots, privateAddress, retryAfterMs, robotsAllows } from "./worker.mjs";

test("jitter and Retry-After stay bounded and deterministic",()=>{
  assert.equal(jitterMs(()=>0),1500);
  assert.equal(jitterMs(()=>.999999),3000);
  assert.equal(retryAfterMs("2",0),2000);
  assert.equal(retryAfterMs("Wed, 21 Oct 2015 07:28:00 GMT",Date.parse("Wed, 21 Oct 2015 07:27:00 GMT")),30000);
  assert.equal(retryAfterMs("999",0),30000);
});
test("resolver guard rejects private IPv4, IPv6, mapped IPv6 and DNS rebinding",async()=>{
  for(const address of ["127.0.0.1","10.0.0.1","169.254.1.1","192.0.2.1","::1","fe80::1","fc00::1","::ffff:127.0.0.1"])assert.equal(privateAddress(address),true,address);
  const guard=makeUrlGuard(async host=>host==="safe.test"?[{address:"8.8.8.8"}]:[{address:"127.0.0.1"}]);
  await assert.rejects(()=>guard("https://rebound.test/",{}),/resolved private/);
  await assert.rejects(()=>guard("file:///tmp/x",{}),/HTTP/);
  await assert.doesNotReject(()=>guard("https://safe.test/",{}));
});

test("robots parsing follows RFC 9309 group, wildcard and longest-match rules",()=>{
  // Verbatim shape published by careers.micron.com / careers.qualcomm.com. The previous
  // Disallow-only parser denied these outright; the sites explicitly permit their jobs paths.
  const eightfold=`User-agent: *
Disallow: /
Allow: /$
Allow: /careers
Allow: /api/career_hub
User-agent: IndeedJobBot
Disallow:
`;
  const rules=parseRobots(eightfold);
  assert.equal(robotsAllows(rules,"/"),true,"Allow: /$ must beat Disallow: / on the bare root");
  assert.equal(robotsAllows(rules,"/careers"),true);
  assert.equal(robotsAllows(rules,"/api/career_hub?page=0"),true);
  assert.equal(robotsAllows(rules,"/internal/admin"),false,"unlisted paths stay denied");
  // The bare "/" root allowance is end-anchored, so deeper paths do not inherit it.
  assert.equal(robotsAllows(rules,"/nope"),false);
  // Group selection: a named group wins over the wildcard group for that agent only.
  const named=parseRobots("User-agent: *\nDisallow: /\nUser-agent: jobscraper\nDisallow:\n");
  assert.equal(robotsAllows(named,"/anything"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /\n"),"/anything"),false);
  // Longest match wins, and Allow breaks an exact-length tie.
  const nested=parseRobots("User-agent: *\nDisallow: /a/\nAllow: /a/b/\n");
  assert.equal(robotsAllows(nested,"/a/x"),false);
  assert.equal(robotsAllows(nested,"/a/b/x"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /x\nAllow: /x\n"),"/x"),true);
  // Wildcards, end anchors, comments and empty Disallow.
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /*.pdf$\n"),"/docs/a.pdf"),false);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /*.pdf$\n"),"/docs/a.pdf?x=1"),true);
  assert.equal(robotsAllows(parseRobots("# comment\nUser-agent: *\nDisallow:\n"),"/anything"),true);
  assert.equal(robotsAllows(parseRobots("User-agent: *\nDisallow: /search-jobs/\n"),"/search-jobs"),true,"prefix match is literal, not fuzzy");
});
