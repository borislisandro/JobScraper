export const applicationStages=["planned","applied","screening","interviewing","offer","accepted","rejected","withdrawn"] as const;
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
