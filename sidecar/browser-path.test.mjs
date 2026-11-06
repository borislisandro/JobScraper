import assert from "node:assert/strict";
import {test} from "node:test";
import {browserCandidates,findBrowser} from "./browser-path.mjs";

test("Windows only ever considers Edge",()=>{
 const found=browserCandidates({ProgramFiles:"C:/Program Files","ProgramFiles(x86)":"C:/Program Files (x86)"},"win32");
 assert.deepEqual(found,["C:/Program Files/Microsoft/Edge/Application/msedge.exe","C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"]);
});
test("an explicit browser wins on every platform",()=>{
 assert.equal(browserCandidates({JOBSCRAPER_BROWSER:"/opt/mine/chrome",PATH:"/usr/bin"},"linux")[0],"/opt/mine/chrome");
 assert.equal(browserCandidates({JOBSCRAPER_BROWSER:"D:/chrome.exe"},"win32")[0],"D:/chrome.exe");
});
test("Linux searches PATH for the Chromium family before fixed install locations",()=>{
 const found=browserCandidates({PATH:"/usr/local/bin:/usr/bin"},"linux");
 assert.ok(found.indexOf("/usr/bin/google-chrome-stable")>-1);
 assert.ok(found.indexOf("/usr/local/bin/chromium")<found.indexOf("/opt/google/chrome/chrome"));
 assert.equal(findBrowser({PATH:"/usr/bin"},"linux",path=>path==="/usr/bin/chromium"),"/usr/bin/chromium");
 assert.equal(findBrowser({PATH:"/usr/bin"},"linux",()=>false),undefined);
});
