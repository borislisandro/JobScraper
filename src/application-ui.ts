export const applicationStages=["planned","applied","screening","interviewing","offer","accepted","rejected","withdrawn"] as const;
// One vocabulary everywhere. "planned" is what the database calls a job the user pressed Save on,
// and calling it "Planned" on the board while the Jobs page says "Saved" made them look like two
// different things.
const stageNames:Record<string,string>={planned:"Saved"};
export const stageLabel=(stage?:string)=>!stage?"":stageNames[stage]??stage.charAt(0).toUpperCase()+stage.slice(1);
/** True once the job has actually been applied to, rather than only saved for later. */
export const isApplied=(stage?:string)=>!!stage&&stage!=="planned";
export function boardMoveError(from:string,to:string){
 if(from===to)return undefined;
 if(to==="applied")return "Applied is recorded only by explicit confirmation after Open application.";
 if(!applicationStages.includes(to as typeof applicationStages[number]))return "Invalid application stage.";
 return undefined;
}
export function attachmentRequest(applicationId:string,file:{name:string;type:string;base64:string},documentType:"resume"|"cover_letter"|"other"){
 return {applicationId,documentType,filename:file.name,mimeType:file.type||"application/octet-stream",base64:file.base64};
}
export function timelineText(event:{eventType:string;fromStage?:string;toStage?:string;reason?:string}){
 return [event.eventType,event.fromStage&&`${event.fromStage} → ${event.toStage}`,event.reason].filter(Boolean).join(" · ");
}
