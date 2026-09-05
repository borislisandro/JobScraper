import http from "node:http";
import net from "node:net";
import {lookup} from "node:dns/promises";
import {makeUrlGuard} from "./request-policy.mjs";

// Chromium can bypass page routing for redirected requests, workers and popups.
// Enforce destination policy at the connection boundary instead. Resolving once
// and connecting to that checked address also closes the browser's DNS-rebinding gap.
export async function createBrowserProxy(source, resolver=lookup) {
 const sockets=new Set();let closed=false;
 const track=socket=>{sockets.add(socket);socket.once("close",()=>sockets.delete(socket));return socket};
 async function target(raw){
  let addresses;
  const guard=makeUrlGuard(async(host,options)=>addresses=await resolver(host,options));
  const url=await guard(raw,source);
  if(!addresses)addresses=await resolver(url.hostname.replace(/^\[|\]$/g,""),{all:true,verbatim:true});
  return {url,address:addresses[0].address};
 }
 const server=http.createServer(async(request,response)=>{
  try{
   const {url,address}=await target(request.url);
   if(closed||response.destroyed)return;
   if(url.protocol!=="http:")throw Error("HTTPS requires a CONNECT tunnel");
   const headers={...request.headers,host:url.host};
   delete headers["proxy-authorization"];delete headers["proxy-connection"];
   const upstream=http.request({hostname:address,port:url.port||80,path:url.pathname+url.search,method:request.method,headers},incoming=>{
    response.writeHead(incoming.statusCode,incoming.headers);incoming.pipe(response);
   });
   upstream.on("socket",track);
   upstream.setTimeout(30000,()=>upstream.destroy(Error("Proxy connection timed out")));
   upstream.on("error",()=>{if(!response.headersSent)response.writeHead(502);response.end()});
   response.on("close",()=>upstream.destroy());request.pipe(upstream);
  }catch{response.writeHead(403).end("Destination blocked")}
 });
 server.on("connection",track);
 server.on("connect",async(request,client,head)=>{
  try{
   const {url,address}=await target(`https://${request.url}`);
   if(closed||client.destroyed)return;
   const upstream=track(net.connect({host:address,port:Number(url.port)||443}));
   upstream.setTimeout(30000,()=>upstream.destroy());
   upstream.on("connect",()=>{
    client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
    if(head.length)upstream.write(head);
    client.pipe(upstream);upstream.pipe(client);
   });
   upstream.on("error",()=>client.destroy());client.on("error",()=>upstream.destroy());
   client.on("close",()=>upstream.destroy());upstream.on("close",()=>client.destroy());
  }catch{client.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")}
 });
 // Plain ws:// upgrade requests are not used by job boards and must not bypass
 // the destination guard; wss:// traffic uses the guarded CONNECT path above.
 server.on("upgrade",(_request,socket)=>socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n"));
 await new Promise((resolve,reject)=>{server.once("error",reject);server.listen(0,"127.0.0.1",resolve)});
 return {server:`http://127.0.0.1:${server.address().port}`,bypass:"<-loopback>",
  close:()=>new Promise(resolve=>{closed=true;server.close(resolve);for(const socket of sockets)socket.destroy()})};
}
