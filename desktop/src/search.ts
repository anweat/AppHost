import type { AppEntry } from './model';
import type { Preferences } from './preferences';
export type SearchAction = 'primary'|'details'|'start'|'stop'|'restart'|'logs'|'failure'|'manifest';
const verbs:Record<string,SearchAction>={启动:'start',start:'start',停止:'stop',stop:'stop',重启:'restart',restart:'restart',打开:'primary',open:'primary',日志:'logs',logs:'logs',详情:'details',失败:'failure',声明:'manifest'};
export function searchApps(apps:AppEntry[], query:string, preferences:Preferences) {
  let text=query.trim().toLocaleLowerCase(), onlyActions=text.startsWith('>'); if(onlyActions)text=text.slice(1).trim();
  let state='', group='';
  text=text.replace(/\bis:([^\s]+)/g,(_,v:string)=>{state=v;return '';}).replace(/\bin:([^\s]+)/g,(_,v:string)=>{group=v;return '';}).trim();
  const first=text.split(/\s+/)[0]||'', action=verbs[first];
  if(action)text=text.slice(first.length).trim();
  const aliases:Record<string,string>={运行:'running',失败:'failed',停止:'stopped',启动中:'starting',待确认:'review',缺失:'missing'};
  state=aliases[state]||state;
  const result=apps.filter(app=>{
    const status=app.registration_state==='SOURCE_MISSING'?'missing':app.requires_confirmation?'review':(app.runtime?.status.state||'STOPPED').toLowerCase();
    const custom=preferences.groups.find(g=>g.members.includes(app.installation_id));
    return (!state||state===status) && (!group||(custom?.name||app.group_label||'应用').toLocaleLowerCase().includes(group)) && (!text||app.label.toLocaleLowerCase().includes(text)||(preferences.search.match_path&&app.path.toLocaleLowerCase().includes(text)));
  });
  return {apps:result,action,onlyActions,showActions:preferences.search.show_actions&&(onlyActions||!!action), actions:action?[action]:['start','stop','restart','logs','details'] as SearchAction[]};
}
