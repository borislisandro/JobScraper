import { invoke } from "@tauri-apps/api/core";
export type Source={id:string;name:string;baseUrl:string;adapterId:string;adapterVersion:string;enabled:boolean;kind:string;disabledReason?:string;robotsOverride:boolean;lastSuccessAt?:string;jobCount:number;closedCount:number};
export type CatalogCompany={id:string;name:string;country:string;tags:string[];adapterId:string;baseUrl:string;config:Record<string,unknown>;starter:boolean;kind:string;disabledReason?:string|null;verified?:{at:string;jobs:number}|null};
export const listCompanyCatalog=()=>invoke<CatalogCompany[]>("list_company_catalog");
export type Job={id:string;sourceId:string;dismissedAt?:string;title:string;company:string;location?:string;workMode?:string;canonicalUrl?:string;applyUrl?:string;descriptionText:string;descriptionStatus:"complete"|"pending"|"failed";postedAt?:string;salaryMin?:number;salaryMax?:number;salaryCurrency?:string;seniority?:string;availability:"active"|"possibly_closed"|"closed"|"archived";applicationStage?:string};
export type JobPage={items:Job[];total:number;offset:number;hasMore:boolean};
export type JobDescription={text:string;status:"complete"|"pending"|"failed";error?:string};
// The Jobs page is the only caller: sources come from the selection, everything else is typed
// into the filter bar. Blank fields mean "no filter", never "hide everything".
export type JobFilter={title:string;keyword:string;sort:"recent"|"posted";savedOnly:boolean;includeClosed:boolean;postedWithinDays:number|null;sourceIds:string[];countries:string[];includeUnknownLocations:boolean;includeDismissed:boolean;firstSeenAfter:string|null};
// "Not for me", and its undo. The listing is kept in every sense except the one that matters when
// reading: it leaves the list.
// A single vacancy read from its own page: for a referral, a recruiter's link, or a board nobody
// has configured. Nothing is stored until the draft comes back and is confirmed.
export type CapturedJob={title:string;company:string;location:string|null;postedAt:string|null;closingAt:string|null;descriptionText:string;descriptionHtml:string;workMode:string|null;externalId:string|null;url:string;structured:boolean};
export const captureJob=(url:string)=>invoke<WorkerEvent[]>("capture_job",{url});
export const saveCapturedJob=(job:CapturedJob)=>invoke<string>("save_captured_job",{job});
export const setJobDismissed=(jobId:string,dismissed:boolean)=>invoke("set_job_dismissed",{jobId,dismissed});
// How many listings arrived since the reader last said they were done.
export const newSince=(since:string|null)=>invoke<number>("new_since",{since});
// When that was. Kept in settings so it survives a restart, which is the whole point of it.
export const REVIEWED_KEY="jobs.reviewedAt";
// How far back the Jobs list looks. Deliberately not persisted: every launch opens on a year.
export const POSTED_WINDOWS=[{days:7,label:"Last 7 days"},{days:30,label:"Last 30 days"},{days:90,label:"Last 3 months"},{days:182,label:"Last 6 months"},{days:365,label:"Last year"},{days:null,label:"Any time"}] as const;
export const DEFAULT_POSTED_WINDOW=365;
export type Application={id:string;jobId:string;personaId?:string;currentStage:string;recruiterName?:string;recruiterEmail?:string;recruiterPhone?:string;sourceAttribution?:string;rejectionReason?:string;rejectionCategory?:string;withdrawnReason?:string;acceptedAt?:string;appliedAt?:string;title?:string;company?:string;pendingConfirmation:boolean};
export type ApplicationEvent={id:string;eventType:string;fromStage?:string;toStage?:string;reason?:string;occurredAt:string;payloadJson:string};export type ApplicationNote={id:string;body:string;createdAt:string;updatedAt?:string};export type ApplicationDocument={id:string;kind:string;documentType:string;filename:string;mimeType:string;sha256:string;size:number;eventId?:string;createdAt:string};export type ApplicationDetails={application:Application;sourceName?:string;sourceUrl?:string;firstResponseAt?:string;events:ApplicationEvent[];notes:ApplicationNote[];documents:ApplicationDocument[]};export type RestoreStatus={applied:boolean;restart_required:boolean;message:string};export type WorkerEvent={event:string;runId:string;payload:Record<string,unknown>};
export type ApplicationDocumentInput={applicationId:string;documentType:string;filename:string;mimeType:string;eventId?:string};
export const MAX_ATTACHMENT_BYTES=20*1024*1024;
export function validateAttachmentSize(size:number){if(size>MAX_ATTACHMENT_BYTES)throw Error("Attachment exceeds 20 MB")}
export function documentMetadataHeader(input:ApplicationDocumentInput){const bytes=new TextEncoder().encode(JSON.stringify(input));return btoa(String.fromCharCode(...bytes)).replaceAll("+","-").replaceAll("/","_").replace(/=+$/g,"")}
export type Interview={id:string;applicationId:string;stage:string;scheduledAt:string;notes?:string;outcome?:string};
export type StartupStatus={phase:string;message:string;error?:string};
export const startupStatus=()=>invoke<StartupStatus>("startup_status");
export const sourceConfig=(sourceId:string)=>invoke<Record<string,unknown>>("get_source_config",{sourceId});
export type AppLog={id:string;at:string;level:string;sourceId?:string;sourceName?:string;action:string;code?:string;message:string;detailJson:string};
export const appLogs=(limit=200)=>invoke<AppLog[]>("list_app_logs",{limit});
export type PerformancePhase={key:string;milliseconds:number};
export type PerformanceRequestKind={count:number;pacingMs:number;backoffMs:number;responseMs:number;bodyMs:number};
export type PerformanceRun={id:string;at:string;sourceId?:string;sourceName?:string;action:string;outcome:"completed"|"failed"|"cancelled";totalMs:number;workerMs:number;requests:number;pages:number;jobs:number;slowestPhase?:PerformancePhase;phases:Record<string,number>;requestsByKind:Record<string,PerformanceRequestKind>;performance:unknown};
export type PerformanceAggregate={sourceId?:string;sourceName?:string;action:string;samples:number;medianMs:number;p95Ms?:number;slowestPhase?:PerformancePhase};
export type PerformanceHistory={recent:PerformanceRun[];aggregates:PerformanceAggregate[]};
export const scrapePerformance=()=>invoke<PerformanceHistory>("scrape_performance");
export const getSetting=(key:string)=>invoke<string|null>("get_setting",{key});
export const setSetting=(key:string,value:string)=>invoke("set_setting",{key,value});
export type BackgroundSyncStatus={enabled:boolean;requested:boolean;debugBuild:boolean;issue:string|null;lastResult:number|null};
export const backgroundSyncStatus=()=>invoke<BackgroundSyncStatus>("background_sync_status");
export const setBackgroundSync=(enabled:boolean)=>invoke<BackgroundSyncStatus>("set_background_sync",{enabled});
export type LaunchAtLoginStatus={enabled:boolean;debugBuild:boolean;issue:string|null;lastResult:number|null};
// Which countries the jobs in scope are actually in, and how many could not be placed at all.
export const jobCountries=(sourceIds:string[])=>invoke<[{code:string;jobs:number}[],number]>("job_countries",{sourceIds});
export const launchAtLoginStatus=()=>invoke<LaunchAtLoginStatus>("launch_at_login_status");
export const setLaunchAtLogin=(enabled:boolean)=>invoke<LaunchAtLoginStatus>("set_launch_at_login",{enabled});
export const captureSession=(source:Source)=>invoke<WorkerEvent[]>("capture_session",{source});
export type ScrapeAllResult={runId:string;completedSources:number;unchangedSources:number;failedSources:number;cancelledSources:number};
export const scrapeAll=(runId:string,sourceIds:string[],forceRefresh=false)=>invoke<ScrapeAllResult>("scrape_all",{runId,sourceIds,forceRefresh});
export const startAutomaticSync=()=>invoke<boolean>("start_automatic_sync");
export const cancelScrapeAll=(runId:string)=>invoke<boolean>("cancel_scrape_all",{runId});
export const cancelScrape=(runId:string)=>invoke<boolean>("cancel_scrape",{runId});
export const hasBrowserSession=(sourceId:string)=>invoke<boolean>("has_browser_session",{sourceId});
export const deleteBrowserSession=(sourceId:string)=>invoke("delete_browser_session",{sourceId});
export type SourceCheck={sourceId:string;name:string;fresh:number;boardTotal?:number;stored:number;changed:boolean;conclusive:boolean;requests:number;error?:string};
export const checkSources=(sourceIds:string[])=>invoke<SourceCheck[]>("check_sources",{sourceIds});
export const purgeAllJobs=(confirmation:string)=>invoke<{deleted:number;kept:number}>("purge_all_jobs",{confirmation});
export const jobDescription=(jobId:string)=>invoke<JobDescription>("job_description",{jobId});
export const api={listSources:()=>invoke<Source[]>("list_sources"),saveSource:(input:unknown)=>invoke<Source>("save_source",{input}),setSourceEnabled:(sourceId:string,enabled:boolean)=>invoke("set_source_enabled",{sourceId,enabled}),deleteSource:(sourceId:string)=>invoke("delete_source",{sourceId,purge:false}),probe:(source:Source)=>invoke<WorkerEvent[]>("probe_source",{source}),jobs:(filter?:JobFilter,offset=0)=>invoke<JobPage>("list_jobs",{filter,offset}),apps:()=>invoke<Application[]>("list_applications"),applicationDetails:(applicationId:string)=>invoke<ApplicationDetails>("application_details",{applicationId}),saveApplicationDetails:(input:unknown)=>invoke("save_application_details",{input}),saveNote:(input:unknown)=>invoke<ApplicationNote>("save_application_note",{input}),deleteNote:(noteId:string)=>invoke("delete_application_note",{noteId}),attachDocument:(input:ApplicationDocumentInput,bytes:ArrayBuffer)=>{validateAttachmentSize(bytes.byteLength);return invoke<ApplicationDocument>("attach_application_document",bytes,{headers:{"x-jobscraper-metadata":documentMetadataHeader(input)}})},exportDocument:(documentId:string)=>invoke<ArrayBuffer>("export_application_document",{documentId}),createApp:(jobId:string)=>invoke<Application>("create_application",{jobId}),deleteApplication:(applicationId:string)=>invoke("delete_application",{applicationId}),stage:(applicationId:string,stage:string,reason?:string,manualOverride=false)=>invoke<Application>("transition_application",{input:{applicationId,stage,reason,manualOverride}}),openApply:(applicationId:string)=>invoke("open_apply",{applicationId}),applyDecision:(applicationId:string,decision:"yes"|"no"|"not_yet")=>invoke("record_apply_decision",{applicationId,decision}),
 // Re-opens the confirmation dialog for an attempt a "not_yet" answer already silenced from the
 // automatic launch/focus prompt. null when the application has no attempt left to confirm.
 confirmApplication:(applicationId:string)=>invoke<{applicationId:string;attemptId:string}|null>("confirm_application",{applicationId}),interviews:(applicationId:string)=>invoke<Interview[]>("list_interviews",{applicationId}),saveInterview:(input:unknown)=>invoke<Interview>("save_interview",{input}),deleteInterview:(interviewId:string)=>invoke("delete_interview",{interviewId}),ghostThreshold:(applicationId:string,days:number)=>invoke("update_ghost_threshold",{applicationId,days}),reconcileReminders:()=>invoke("reconcile_reminders"),resumeScrape:(runId:string)=>invoke("resume_scrape",{runId}),diagnostics:()=>invoke<{dbPath:string;schemaVersion:number;sourceCount:number;jobCount:number;appCodeNetworkFreeAtStartup:boolean;webviewRuntimeNetworkNotControlledByApp:boolean;sidecarActive:boolean}> ("diagnostics"),restoreStatus:()=>invoke<RestoreStatus|undefined>("restore_status"),test:(source:Source)=>invoke<WorkerEvent[]>("test_source",{source}),scrape:(source:Source,fullRefresh=false)=>invoke<WorkerEvent[]>("scrape_source",{source,fullRefresh})};
