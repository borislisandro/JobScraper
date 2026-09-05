import assert from "node:assert/strict";
import http from "node:http";
import {existsSync} from "node:fs";
import {test} from "node:test";
import {createBrowserProxy} from "./browser-proxy.mjs";

const listen=server=>new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
const close=server=>new Promise(resolve=>server.close(resolve));
const viaProxy=(proxy,url)=>new Promise((resolve,reject)=>{
 const req=http.get(proxy.server,{path:url},res=>{res.resume();res.on("end",()=>resolve(res.statusCode))});req.on("error",reject);
});
test("browser proxy blocks private HTTP and CONNECT destinations, including DNS answers",async()=>{
 let lookups=0;
 const proxy=await createBrowserProxy({},async()=>{lookups++;return[{address:"127.0.0.1",family:4}]});
 try{
  assert.equal(await viaProxy(proxy,"http://127.0.0.1:1234/"),403);
  assert.equal(await viaProxy(proxy,"http://public.example/jobs"),403);
  assert.equal(lookups,1);
  const status=await new Promise((resolve,reject)=>{
   const req=http.request(proxy.server,{method:"CONNECT",path:"public.example:443"});
   req.on("connect",(res,socket)=>{socket.destroy();resolve(res.statusCode)});req.on("error",reject);req.end();
  });
  assert.equal(status,403);
 }finally{await proxy.close()}
});
test("explicit local-network sources work through the proxy and retain their Host header",async()=>{
 const server=http.createServer((req,res)=>res.writeHead(req.headers.host===`fixture.example:${server.address().port}`?200:400).end());await listen(server);
 const proxy=await createBrowserProxy({allowPrivateNetwork:true},async()=>[{address:"127.0.0.1",family:4}]);
 try{
  assert.equal(await viaProxy(proxy,`http://fixture.example:${server.address().port}/jobs`),200);
 }finally{await proxy.close();await close(server)}
});
const executable=[process.env["ProgramFiles(x86)"],process.env.ProgramFiles].filter(Boolean).map(root=>`${root}/Microsoft/Edge/Application/msedge.exe`).find(existsSync);
test("real Edge blocks private fetches, popup navigation and redirected traffic through the proxy",{skip:!executable,timeout:30000},async()=>{
 const {chromium}=await import("playwright-core");
 let hits=0;const server=http.createServer((_req,res)=>{hits++;res.end("private")});await listen(server);
 const proxy=await createBrowserProxy({});
 const browser=await chromium.launch({executablePath:executable,headless:true,proxy:{server:proxy.server,bypass:proxy.bypass}});
 try{
  const context=await browser.newContext({serviceWorkers:"block"}),page=await context.newPage();
  const url=`http://127.0.0.1:${server.address().port}/secret`;
  await page.setContent("<h1>Fixture</h1>");
  await page.evaluate(async url=>{await fetch(url).catch(()=>{});await new Promise(resolve=>{const img=new Image();img.onload=img.onerror=resolve;img.src=url})},url);
  const popupReady=context.waitForEvent("page");
  await page.evaluate(url=>{window.open(url)},url);
  const popup=await popupReady;await popup.waitForLoadState("domcontentloaded").catch(()=>{});
  // A fulfilled first response redirects to the private destination. Page routing does
  // not see the next hop, but the connection-level proxy must still refuse it.
  await context.route("http://fixture.example/redirect",route=>route.fulfill({status:302,headers:{location:url}}));
  await page.goto("http://fixture.example/redirect").catch(()=>{});
  assert.equal(hits,0,"no private request reached the fixture server");
 }finally{await browser.close();await proxy.close();await close(server)}
});
