// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { describe,it,expect,vi } from 'vitest';
const { invoke }=vi.hoisted(()=>({invoke:vi.fn().mockResolvedValue(null)}));
vi.mock('@tauri-apps/api/core',()=>({invoke}));
import TileBoard from './components/TileBoard.vue';
import { defaults } from './preferences';
import { searchApps } from './search';
import { declarationDiff, declarationSelection } from './manifestDiff';
import type { AppEntry } from './model';
const apps:AppEntry[]=[{installation_id:'a',project_id:'p',label:'Notes',path:'/notes',group:'parent',group_label:'Projects',registration_state:'REGISTERED',requires_confirmation:false,runtime:null},{installation_id:'b',project_id:'q',label:'API',path:'/server',group:'parent',group_label:'Projects',registration_state:'REGISTERED',requires_confirmation:true,runtime:null}];
describe('search and declaration navigation',()=>{
  it('combines verb, status and custom group filters without reordering source data',()=>{
    const p=defaults();p.groups=[{id:'g',name:'Study',members:['a']}];
    const value=searchApps(apps,'停止 notes is:stopped in:study',p);
    expect(value.action).toBe('stop');expect(value.apps.map(a=>a.installation_id)).toEqual(['a']);
    expect(searchApps(apps,'> logs',p).onlyActions).toBe(true);
    expect(searchApps(apps,'/server',p).apps).toHaveLength(0);p.search.match_path=true;
    expect(searchApps(apps,'/server',p).apps[0]?.installation_id).toBe('b');
    expect(searchApps(apps,'is:review',p).apps.map(a=>a.installation_id)).toEqual(['b']);
    expect(apps.map(a=>a.installation_id)).toEqual(['a','b']);
  });
  it('accepts Chinese filter names and full or partial state aliases',()=>{
    const p=defaults();
    const entry=(id:string,state:NonNullable<AppEntry['runtime']>['status']['state']):AppEntry=>({...apps[0]!,installation_id:id,runtime:{installation_id:id,instance_id:id,updated_at_ms:1,status:{state,failure:null},services:[]}});
    const states=[entry('running','RUNNING'),entry('starting','STARTING'),entry('stopped','STOPPED'),entry('stopping','STOPPING'),entry('failed','FAILED'),{...apps[1]!,installation_id:'review'},{...apps[0]!,installation_id:'missing',registration_state:'SOURCE_MISSING'}];
    const ids=(query:string)=>searchApps(states,query,p).apps.map(app=>app.installation_id);
    for(const alias of ['running','run','RUN','运行','运行中'])expect(ids(`状态:${alias}`)).toEqual(['running','starting']);
    for(const alias of ['stopped','stop','已停止','停止'])expect(ids(`is:${alias}`)).toEqual(['stopped']);
    for(const alias of ['failed','fail','error','异常','失败'])expect(ids(`is:${alias}`)).toEqual(['failed','missing']);
    for(const alias of ['review','rev','待确认','确认'])expect(ids(`状态:${alias}`)).toEqual(['review']);
    expect(ids('is:starting')).toEqual(['starting']);expect(ids('状态:正在停止')).toEqual(['stopping']);
    expect(ids('状态:缺失')).toEqual(['missing']);expect(ids('is:unknown')).toEqual([]);
  });
  it('matches a running service even when its installation is waiting for review',()=>{
    const app:AppEntry={...apps[1]!,runtime:{installation_id:'b',instance_id:'i',updated_at_ms:1,status:{state:'STOPPING',failure:null},services:[{name:'api',port:8000,alive:true,ready:false}]}};
    expect(searchApps([app],'is:running',defaults()).apps).toEqual([app]);
    expect(searchApps([app],'状态:待确认',defaults()).apps).toEqual([app]);
  });
  it('matches custom group names and default group paths independently of path search',()=>{
    const p=defaults();const grouped=apps.map(app=>({...app,group:'/Users/me/Projects'}));
    expect(searchApps(grouped,'组:/users/me',p).apps).toEqual(grouped);
    expect(searchApps(grouped,'PRJ',p).apps).toEqual(grouped);
    p.groups=[{id:'study',name:'Research Study',members:['a']}];
    expect(searchApps(grouped,'组:STUDY notes',p).apps.map(app=>app.installation_id)).toEqual(['a']);
    expect(searchApps(grouped,'research',p).apps.map(app=>app.installation_id)).toEqual(['a']);
    expect(searchApps(grouped,'in:/users/me',p).apps.map(app=>app.installation_id)).toEqual(['b']);
    expect(searchApps(grouped,'/server',p).apps).toHaveLength(0);
    p.search.match_path=true;expect(searchApps(grouped,'/srv',p).apps.map(app=>app.installation_id)).toEqual(['b']);
  });
  it('ranks prefix, substring, and subsequence matches without moving the canvas or ties',()=>{
    const labels=['P_a_p_e_r','My Paper','Paper Reader','Paper Notes'];
    const source=labels.map((label,i)=>({...apps[0]!,label,installation_id:String(i)}));
    const result=searchApps(source,'paper',defaults());
    expect(result.apps.map(app=>app.label)).toEqual(labels);
    expect(result.ranked.map(app=>app.label)).toEqual(['Paper Reader','Paper Notes','My Paper','P_a_p_e_r']);
    expect(result.ranked).not.toBe(result.apps);expect(source.map(app=>app.label)).toEqual(labels);
    const long=[source[0]!,{...source[1]!,label:`${'x'.repeat(400)}paper`}];
    expect(searchApps(long,'paper',defaults()).ranked[0]).toBe(long[1]);
  });
  it('supports prototype verb aliases and exposes only the target text',()=>{
    const aliases={open:'primary',打开:'primary',start:'start',run:'start',启动:'start',关闭:'stop',kill:'stop',reload:'restart',re:'restart',log:'logs',lo:'logs',details:'details',info:'details',详情:'details',声明:'manifest',failure:'failure'} as const;
    for(const [verb,action] of Object.entries(aliases)){
      const result=searchApps(apps,`${verb} NOTES 状态:stopped 组:projects`,defaults());
      expect(result.action).toBe(action);expect(result.text).toBe('notes');expect(result.actions).toEqual([action]);expect(result.apps).toEqual([apps[0]]);
    }
    expect(searchApps(apps,'l notes',defaults()).action).toBeUndefined();
  });
  it('honors explicit action mode even when optional action suggestions are disabled',()=>{
    const p=defaults();p.search.show_actions=false;
    for(const query of ['>logs notes','> logs notes','logs notes >']){
      const result=searchApps(apps,query,p);
      expect(result.onlyActions).toBe(true);expect(result.showActions).toBe(true);expect(result.action).toBe('logs');expect(result.text).toBe('notes');
    }
    expect(searchApps(apps,'logs notes',p).showActions).toBe(false);
    expect(searchApps(apps,'> logs',p).text).toBe('');
    expect(searchApps(apps,'>',p).showActions).toBe(true);
  });
  it('keeps bare app names searchable and parses filters only as complete tokens',()=>{
    const source=[{...apps[0]!,label:'Logs'},{...apps[1]!,label:'island:is:failed'}];
    const result=searchApps(source,'logs',defaults());
    expect(result.action).toBeUndefined();expect(result.text).toBe('logs');expect(result.apps).toEqual([source[0]]);
    expect(searchApps(source,'island:is:failed',defaults()).apps).toEqual([source[1]]);
    const empty=searchApps(source,'   ',defaults());
    expect(empty.text).toBe('');expect(empty.apps).toEqual(source);expect(empty.ranked).toEqual(source);expect(empty.showActions).toBe(false);
  });
  it('selects the related service port, never uses an output log line as a manifest line',()=>{
    const text='[[services]]\nname = "api"\nport = 8000\n[[services]]\nname = "web"\nport = 5173';
    const selection=declarationSelection(text,{service:'web',field:'port'});
    expect(text.slice(selection.start,selection.end)).toBe('port = 5173');
    expect(declarationDiff('x\na\nz','x\nb\nz')).toContain('− a\n+ b');
    expect(declarationDiff('x'.repeat(10),'x'.repeat(10))).toBe('声明文本没有变化');
    expect(declarationDiff('',Array(300).fill('line').join('\n'))).toContain('截断');
  });
});
describe('fixed tile layout and menus',()=>{
  it('shows a keyboard menu, moves membership explicitly, and retains installation order',async()=>{
    const p=defaults();p.groups=[{id:'g',name:'Study',members:[]}];
    const panel=mount(TileBoard,{attachTo:document.body,props:{apps,snapshot:{apps,live:true,read_at_ms:1,runtime_error:null},lens:null,preferences:p,feedback:new Map()}});await flushPromises();
    await panel.find('.tile-main').trigger('keydown',{key:'F10',shiftKey:true});await flushPromises();
    const menu=document.querySelector('[role=menu]')!;expect(menu.textContent).toContain('移到分组');
    const select=menu.querySelector('select')!;select.value='g';select.dispatchEvent(new Event('change',{bubbles:true}));await flushPromises();
    expect(panel.emitted('preference')?.[0]).toEqual([{kind:'move',installation_id:'a',group_id:'g'}]);
    expect(panel.findAll('.tile-name').map(n=>n.text())).toEqual(['Notes','API']);panel.unmount();
  });
  it('hides empty custom groups during search and restores them afterward',async()=>{
    const preferences={...defaults(),groups:[{id:'g',name:'Empty group',members:[]}]};
    const panel=mount(TileBoard,{props:{apps:[apps[0]!],filtering:true,snapshot:{apps,live:true,read_at_ms:1,runtime_error:null},lens:null,preferences,feedback:new Map()}});
    expect(panel.find('[aria-label="Empty group"]').exists()).toBe(false);await panel.setProps({filtering:false});expect(panel.find('[aria-label="Empty group"]').exists()).toBe(true);panel.unmount();
  });
  it('recent is optional, capped at six and separate from the stable regular tiles',async()=>{
    const many=Array.from({length:12},(_,i)=>({...apps[0]!,installation_id:String(i),label:String(i)}));const p=defaults();p.last_used=Object.fromEntries(many.map((a,i)=>[a.installation_id,i+1]));
    const panel=mount(TileBoard,{props:{apps:many,snapshot:{apps:many,live:false,read_at_ms:1,runtime_error:null},lens:null,preferences:p,feedback:new Map()}});await flushPromises();
    expect(panel.findAll('.recent .tile')).toHaveLength(6);expect(panel.findAll('.group:not(.recent) .tile-name').map(n=>n.text())).toEqual(many.map(a=>a.label));
    await panel.setProps({preferences:{...p,show_recent:false}});expect(panel.find('.recent').exists()).toBe(false);panel.unmount();
  });
});
