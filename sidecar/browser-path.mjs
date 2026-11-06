import {existsSync} from "node:fs";
import {delimiter,join} from "node:path";

// The worker drives any Chromium-based browser through Playwright. Windows ships Edge, so that is
// the only place it looks there; elsewhere it takes whichever Chromium family browser is installed.
// JOBSCRAPER_BROWSER overrides the search for machines where the browser lives somewhere unusual.
const LINUX_NAMES=["microsoft-edge","microsoft-edge-stable","google-chrome-stable","google-chrome","chromium","chromium-browser","brave-browser"];
const LINUX_FIXED=["/opt/microsoft/msedge/msedge","/opt/google/chrome/chrome","/usr/lib64/chromium-browser/chromium-browser","/usr/lib/chromium/chromium","/snap/bin/chromium"];

export function browserCandidates(env=process.env,platform=process.platform){
 const explicit=env.JOBSCRAPER_BROWSER?[env.JOBSCRAPER_BROWSER]:[];
 if(platform==="win32")return[...explicit,env.ProgramFiles&&`${env.ProgramFiles}/Microsoft/Edge/Application/msedge.exe`,env["ProgramFiles(x86)"]&&`${env["ProgramFiles(x86)"]}/Microsoft/Edge/Application/msedge.exe`].filter(Boolean);
 const onPath=(env.PATH||"").split(delimiter).filter(Boolean).flatMap(dir=>LINUX_NAMES.map(name=>join(dir,name)));
 return[...explicit,...onPath,...LINUX_FIXED];
}
export const findBrowser=(env=process.env,platform=process.platform,exists=existsSync)=>browserCandidates(env,platform).find(path=>exists(path));
export const BROWSER_MISSING="browser_incompatible: No Chromium-based browser (Microsoft Edge, Google Chrome or Chromium) was found";
