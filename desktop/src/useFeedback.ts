import { ref } from 'vue';
import { Channel, invoke } from '@tauri-apps/api/core';
import type { AdviceAction, FailureReport } from './diagnostics';
import type { CoreError, Snapshot } from './model';

export interface Feedback {
  text: string;
  persistent: boolean;
  report?: FailureReport;
  cleared?: boolean;
  error?: CoreError;
  impact?: string;
  primary?: AdviceAction;
  since?: number;
}
export function feedbackPrimary(item:Feedback) { return item.report?.advice?.actions.find(a=>a.primary) || item.primary; }
function controlFeedback(error:CoreError, since:number):Feedback {
  const operation:Record<string,[string,string]>={
    REVIEW_REQUIRED:['review_manifest','复核启动声明'],
    STALE_LAUNCH:['refresh_status','刷新应用状态'],
    RESTART_REQUIRED:['retry','完整重新启动'],
    BUSY:['refresh_status','刷新应用状态'],
    RUNTIME_LIMIT:['view_activity','查看活动实例'],
    BACKEND_MISSING:['inspect_daemon','检查后台连接'],
  };
  const [id,title]=operation[error.code] || ['inspect_daemon','检查后台连接'];
  return {text:error.message || error.code,error,persistent:true,since,
    impact:'本次操作未确认完成；现有应用状态以最新观察为准。',
    primary:{id:id!,title:title!,primary:true,argv:[]}};

}
// Only explicit single-app operations create bubbles. Tokens bind late reads and
// condition events to the displayed result, so dismissal/retry cannot revive it.
export function useFeedback() {
  const feedback = ref(new Map<string, Feedback>());
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const tokens = new Map<string, string>();
  const conditions = new Map<string, string>();
  function dismiss(id:string) {
    feedback.value.delete(id); tokens.delete(id);
    clearTimeout(timers.get(id)); timers.delete(id);
    const token=conditions.get(id);
    if(token) { conditions.delete(id); void invoke('desktop_condition_unwatch',{token}).catch(()=>{}); }
  }
  function bubble(id:string,text:string) {
    dismiss(id); feedback.value.set(id,{text,persistent:false});
    timers.set(id,setTimeout(()=>dismiss(id),8000));
  }
  async function failure(id:string, since:number, instance?:string, error?:CoreError) {
    dismiss(id);
    const token=crypto.randomUUID(); tokens.set(id,token);
    feedback.value.set(id,controlFeedback(error || {code:'OPERATION_FAILED',message:'本次操作未能完成，失败原因暂不可用'},since));
    try {
      let report:FailureReport|undefined;
      // State notifications can precede publication of the failure file. Retry
      // reads only (five attempts, 2 s total backoff); never show older advice.
      for(const delay of [0,100,300,600,1000]) {
        if(delay) await new Promise(resolve=>setTimeout(resolve,delay));
        if(tokens.get(id)!==token) return;
        try { report=await invoke<FailureReport>('desktop_failure',{installationId:id,failureId:null}); }
        catch(cause) { if((cause as {code?:string})?.code!=='NOT_FOUND') return; }
        if(report?.failure && report.advice && report.failure.at_ms>=since &&
          (!instance || !report.failure.instance_id || report.failure.instance_id===instance)) break;
        report=undefined;
      }
      if(tokens.get(id)!==token || !report?.failure || !report.advice) return;
      const currentReport=report;
      feedback.value.set(id,{text:report.advice.title,persistent:true,report});
      if(report.advice.code!=='PORT_IN_USE') return;
      const channel=new Channel<{data:{state:string;failure_id:string}}>();
      channel.onmessage=event=>{
        const item=feedback.value.get(id);
        if(tokens.get(id)===token && event.data.failure_id===currentReport.failure!.failure_id && event.data.state==='cleared' && item) item.cleared=true;
      };
      conditions.set(id,token);
      try { await invoke('desktop_condition_watch',{installationId:id,failureId:report.failure.failure_id,token,channel}); }
      finally { if(conditions.get(id)===token) conditions.delete(id); }
    } catch { /* Keep this operation's error; no unrelated history link is offered. */ }
  }
  function resolved(snapshot:Snapshot|null) {
    if(!snapshot) return;
    for(const [id,item] of feedback.value) {
      if(!item.persistent) continue;
      const app=snapshot.apps.find(a=>a.installation_id===id);
      if(!app) {dismiss(id);continue;}
      if(!item.report) {
        // Clear only evidence we can observe, never an unrelated old success.
        const code=item.error?.code;
        if((snapshot.read_at_ms >= (item.since || 0) && code==='REVIEW_REQUIRED' && !app.requires_confirmation) ||
          (snapshot.read_at_ms >= (item.since || 0) && code==='SOURCE_MISSING' && app.registration_state!=='SOURCE_MISSING') ||
          (['BACKEND_MISSING','DAEMON_UNAVAILABLE','DAEMON_START_TIMEOUT'].includes(code||'') && snapshot.live && snapshot.read_at_ms >= (item.since || 0) && !snapshot.runtime_error) ||
          (snapshot.live && app.runtime && app.runtime.updated_at_ms >= (item.since||Infinity) && !app.runtime.operation_error && ['RUNNING','STOPPED'].includes(app.runtime.status.state))) dismiss(id);
      } else if(snapshot.live && app.runtime && !app.runtime.operation_error && ['RUNNING','STOPPED'].includes(app.runtime.status.state)) dismiss(id);
    }
  }
  function dispose(){ for(const id of [...feedback.value.keys()]) dismiss(id); }
  return {feedback,bubble,failure,dismiss,resolved,dispose};
}
