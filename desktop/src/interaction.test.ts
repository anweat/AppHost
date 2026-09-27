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
