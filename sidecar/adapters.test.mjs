import assert from "node:assert/strict";import{test}from"node:test";import{requestFor}from"./adapters.mjs";
test("platform request builders produce deterministic pagination",()=>{const apple=requestFor("apple","https://x.test",{locale:"pt-pt",page:3,token:"csrf"});assert.equal(apple.method,"POST");assert.equal(apple.headers["x-apple-csrf-token"],"csrf");assert.equal(apple.body.page,3);const wd=requestFor("workday","https://x.test",{listingPath:"/workday",offset:20,limit:20,query:"RF design"});assert.equal(wd.method,"POST");assert.equal(wd.body.searchText,"RF design");assert.deepEqual(wd.next({jobPostings:Array(20)}),{offset:40});const eight=requestFor("eightfold","https://x.test",{domain:"x.com",start:2,pageSize:2,query:"fw"});assert.match(eight.url,/\/api\/pcsx\/search\?/);assert.match(eight.url,/domain=x.com/);assert.match(eight.url,/start=2/);assert.deepEqual(eight.next({data:{positions:[{},{}],count:10}}),{start:4});assert.equal(eight.next({data:{positions:[{},{}],count:4}}),null);const legacy=requestFor("eightfold","https://x.test",{domain:"x.com",eightfoldApi:"legacy",pageSize:100});assert.match(legacy.url,/\/api\/apply\/v2\/jobs\?/);assert.match(legacy.url,/num=100/);assert.deepEqual(legacy.next({positions:[{}],count:9}),{start:1});assert.throws(()=>requestFor("eightfold","https://x.test",{}),/^Error: incomplete/);for(const a of["icims","talentbrew-jibe","phenom"]){assert.deepEqual(requestFor(a,"https://x.test",{pageSize:1}).next({jobs:[{}]}),{page:a==="phenom"?1:2})}});
test("unsupported request builders are explicit",()=>assert.throws(()=>requestFor("custom-api","https://x.test",{}),/unsupported/));
// PTC's Workday answers an offset past the end with the first page again instead of an empty one,
// so "a full page came back" is not proof the board continues and the traversal ran forever. The
// published total is what ends it, and it is carried forward because that tenant reports 0 on the
// pages after the first.
test("Workday paging stops on the published total, not on a full page",()=>{
 const page=offset=>requestFor("workday","https://x.test",{listingPath:"/wd",offset,limit:20,...(offset?{boardTotal:180}:{})});
 assert.deepEqual(page(0).next({total:180,jobPostings:Array(20)}),{offset:20,boardTotal:180});
 assert.deepEqual(page(140).next({total:0,jobPostings:Array(20)}),{offset:160,boardTotal:180});
 assert.equal(page(160).next({total:0,jobPostings:Array(20)}),null,"the last full page is the end of a 180-row board");
 assert.equal(requestFor("workday","https://x.test",{listingPath:"/wd",offset:0,limit:20}).next({jobPostings:Array(19)}),null);
});
