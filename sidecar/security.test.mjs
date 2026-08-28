import assert from "node:assert/strict";
import { test } from "node:test";
import { jitterMs, makeUrlGuard, privateAddress, retryAfterMs } from "./worker.mjs";

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
