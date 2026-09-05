import net from "node:net";
import {lookup} from "node:dns/promises";
const privateHost=h=>h==="localhost"||h.endsWith(".localhost")||h.endsWith(".local")||h==="::1";
const blockedAddresses=new net.BlockList();
for(const [address,prefix] of [["0.0.0.0",8],["10.0.0.0",8],["100.64.0.0",10],["127.0.0.0",8],["169.254.0.0",16],["172.16.0.0",12],["192.0.0.0",24],["192.0.2.0",24],["192.168.0.0",16],["198.18.0.0",15],["198.51.100.0",24],["203.0.113.0",24],["224.0.0.0",4],["240.0.0.0",4]])blockedAddresses.addSubnet(address,prefix,"ipv4");
for(const [address,prefix] of [["::",96],["64:ff9b::",96],["100::",64],["2001:db8::",32],["fc00::",7],["fe80::",10],["fec0::",10],["ff00::",8]])blockedAddresses.addSubnet(address,prefix,"ipv6");
// BlockList also checks IPv4 subnets against mapped IPv6, including URL-normalized hex forms.
export const privateAddress=ip=>{const family=net.isIP(ip);return !family||blockedAddresses.check(ip,family===6?"ipv6":"ipv4")};
export const makeUrlGuard=(resolver=lookup)=>async(raw,s)=>{const u=new URL(raw);if(!["http:","https:"].includes(u.protocol))throw Error("network_error: only HTTP(S) URLs are allowed");if(s.allowPrivateNetwork)return u;const host=u.hostname.replace(/^\[|\]$/g,"");if(privateHost(host)||(net.isIP(host)&&privateAddress(host)))throw Error("network_error: private network target blocked");let addresses;try{addresses=await resolver(host,{all:true,verbatim:true})}catch{throw Error("network_error: DNS lookup failed")};if(!addresses.length||addresses.some(a=>privateAddress(a.address)))throw Error("network_error: resolved private/reserved address blocked");return u};
export const allowed=makeUrlGuard();

// Headers supplied for one request must not follow redirects to an unrelated origin.
// Unknown custom headers may contain credentials, so only ordinary content negotiation
// headers are safe to carry across that boundary.
const publicHeaders=new Set(["accept","accept-language","content-type","user-agent"]);
export function scopedHeaders(headers, destination, origin) {
 const same=new URL(destination).origin===new URL(origin).origin;
 return Object.fromEntries([...new Headers(headers||{})].filter(([name])=>same||publicHeaders.has(name)));
}

export function cookieHeader(cookies, destination, baseUrl, now=Date.now()/1000) {
 const target=new URL(destination),base=new URL(baseUrl);
 return cookies.filter(cookie=>{
  if(typeof cookie==="string")return target.origin===base.origin;
  if(!cookie||cookie.partitionKey)return false;
  if(cookie.secure&&target.protocol!=="https:")return false;
  if(cookie.expires!=null&&cookie.expires!==-1&&(!Number.isFinite(cookie.expires)||cookie.expires<=now))return false;
  if(!cookie.domain){if(target.origin!==base.origin)return false}
  else{
   const domain=cookie.domain.toLowerCase(),host=target.hostname.toLowerCase();
   if(domain.startsWith(".")?host!==domain.slice(1)&&!host.endsWith(domain):host!==domain)return false;
  }
  const path=cookie.path||"/";
  return target.pathname===path||(target.pathname.startsWith(path)&&(path.endsWith("/")||target.pathname[path.length]==="/"));
 }).map(cookie=>typeof cookie==="string"?cookie:`${cookie.name}=${cookie.value}`).join("; ");
}
