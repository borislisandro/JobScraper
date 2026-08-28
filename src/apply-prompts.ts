export type ApplyAttempt={applicationId:string;attemptId:string};
export type TauriListen=(event:string, handler:(event:{payload:ApplyAttempt})=>void)=>Promise<()=>void>;

/** Subscribe once; cleanup also handles an unmount before `listen` resolves. */
export function listenForApplyConfirmation(listen:TauriListen,onAttempt:(attempt:ApplyAttempt)=>void){let active=true;let stop:(()=>void)|undefined;void listen("apply-confirmation-required",event=>{if(active)onAttempt(event.payload)}).then(unlisten=>{if(active)stop=unlisten;else unlisten()});return()=>{active=false;stop?.()}}
