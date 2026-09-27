// @vitest-environment happy-dom
import { shallowMount, flushPromises } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
const { invoke }=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke,isTauri:()=>true,Channel:class {onmessage?:unknown}}));
import App from './App.vue';
import TileBoard from './components/TileBoard.vue';
import { defaults } from './preferences';
const snapshot={apps:['a','b'].map(id=>({installation_id:id,project_id:id,label:id==='a'?'Notes':'Server',path:'/app/'+id,registration_state:'REGISTERED',requires_confirmation:false,runtime:null})),live:true,read_at_ms:1,runtime_error:null};
beforeEach(()=>invoke.mockReset().mockImplementation(async(cmd:string)=>{
  if(cmd==='desktop_snapshot')return snapshot;
  if(cmd==='desktop_preferences'||cmd==='desktop_ui_edit')return defaults();
  if(cmd==='desktop_watch')return new Promise(()=>{});
  if(cmd==='desktop_action')return {results:[]};
}));
describe('desktop compound interactions',()=>{
  it('keeps Space available for button activation while canvas typing starts search',async()=>{
    const panel=shallowMount(App,{attachTo:document.body});await flushPromises();
    const button=panel.find('button.stop-all');
    const space=new KeyboardEvent('keydown',{key:' ',bubbles:true,cancelable:true});button.element.dispatchEvent(space);
    expect(space.defaultPrevented).toBe(false);
    const letter=new KeyboardEvent('keydown',{key:'n',bubbles:true,cancelable:true});button.element.dispatchEvent(letter);await flushPromises();
    expect(letter.defaultPrevented).toBe(true);
    expect((panel.find('input[type=search]').element as HTMLInputElement).value).toBe('n');panel.unmount();
  });
  it('focuses search for the macOS IME first key without appending Latin text or canceling composition',async()=>{
    const panel=shallowMount(App,{attachTo:document.body});await flushPromises();
    const firstKey=new KeyboardEvent('keydown',{key:'n',keyCode:229,bubbles:true,cancelable:true});
    panel.find('.brand').element.dispatchEvent(firstKey);await flushPromises();
    const field=panel.find('input[type=search]');
    expect(firstKey.defaultPrevented).toBe(false);expect((field.element as HTMLInputElement).value).toBe('');expect(document.activeElement).toBe(field.element);
    await field.trigger('compositionstart');await field.setValue('停止 Notes');await field.trigger('keydown',{key:'Enter',isComposing:true,keyCode:229});expect(invoke.mock.calls.some(c=>c[0]==='desktop_action')).toBe(false);await field.setValue('你好');await field.trigger('compositionend',{data:'你好'});
    expect((field.element as HTMLInputElement).value).toBe('你好');panel.unmount();
  });
  it('disables Stop All when the library only contains stopped registrations',async()=>{
    const panel=shallowMount(App);await flushPromises();expect(panel.find('button.stop-all').attributes('disabled')).toBeDefined();
    await panel.find('button.stop-all').trigger('click');expect(invoke.mock.calls.some(c=>c[0]==='desktop_action')).toBe(false);panel.unmount();
  });
  it('retains every skipped group member in activity without issuing starts',async()=>{
    invoke.mockImplementation(async cmd=>{if(cmd==='desktop_snapshot')return {...snapshot,apps:snapshot.apps.map(a=>({...a,requires_confirmation:true}))};if(cmd==='desktop_preferences')return defaults();if(cmd==='desktop_watch')return new Promise(()=>{});});
    const panel=shallowMount(App);await flushPromises();panel.findComponent(TileBoard).vm.$emit('batch','start',['a','b']);await flushPromises();
    const activity=panel.find('.activity-panel');expect(activity.text()).toContain('Notes · 已跳过');expect(activity.text()).toContain('Server · 已跳过');
    expect(invoke.mock.calls.some(c=>c[0]==='desktop_action')).toBe(false);panel.unmount();
  });
  it('queues a layout change behind a recent-use write instead of silently dropping it',async()=>{
    let release!:(value:unknown)=>void;
    invoke.mockImplementation(async(cmd,params)=>{if(cmd==='desktop_snapshot')return snapshot;if(cmd==='desktop_preferences')return defaults();if(cmd==='desktop_watch')return new Promise(()=>{});if(cmd==='desktop_ui_edit'){if(params.change.kind==='touch')return new Promise(r=>{release=r;});return defaults();}if(cmd==='desktop_action')return {results:[]};});
    const panel=shallowMount(App);await flushPromises();const board=panel.findComponent(TileBoard);
    board.vm.$emit('primary','a');await flushPromises();board.vm.$emit('preference',{kind:'recent',enabled:false});await flushPromises();
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_ui_edit')).toHaveLength(1);
    release(defaults());await flushPromises();expect(invoke.mock.calls.filter(c=>c[0]==='desktop_ui_edit').map(c=>c[1].change.kind)).toEqual(['touch','recent']);panel.unmount();
  });
  it('prunes old UI references after an installation disappears from the observed registry',async()=>{
    let apps=snapshot.apps;
    const prefs={...defaults(),groups:[{id:'g',name:'Study',members:['a','b']}],last_used:{a:1,b:2}};
    invoke.mockImplementation(async cmd=>{if(cmd==='desktop_snapshot')return {...snapshot,apps};if(cmd==='desktop_preferences')return prefs;if(cmd==='desktop_ui_edit')return {...prefs,groups:[{...prefs.groups[0]!,members:['a']}],last_used:{a:1}};if(cmd==='desktop_watch')return new Promise(()=>{});});
    const panel=shallowMount(App);await flushPromises();expect(invoke.mock.calls.some(c=>c[0]==='desktop_ui_edit')).toBe(false);
    apps=[snapshot.apps[0]!];await panel.find('[aria-label="刷新应用与状态"]').trigger('click');await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_ui_edit',{change:{kind:'prune'}});panel.unmount();
  });
  it('Stop All cancels not-yet-dispatched members of a group launch',async()=>{
    let release!:(value:unknown)=>void;
    invoke.mockImplementation(async(cmd:string,params:any)=>{
      if(cmd==='desktop_snapshot')return snapshot;
      if(cmd==='desktop_preferences'||cmd==='desktop_ui_edit')return defaults();
      if(cmd==='desktop_watch')return new Promise(()=>{});
      if(cmd==='desktop_action'&&params.action==='start')return new Promise(resolve=>{release=resolve;});
      if(cmd==='desktop_action')return {results:[]};
    });
    const panel=shallowMount(App);await flushPromises();
    panel.findComponent(TileBoard).vm.$emit('batch','start',['a','b']);await flushPromises();
    await panel.find('button.stop-all').trigger('click');await flushPromises();
    release({installation_id:'a',instance_id:'i',operation_id:'o'});await flushPromises();
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_action'&&c[1].action==='start').map(c=>c[1].installationId)).toEqual(['a']);
    expect(invoke.mock.calls.filter(c=>c[0]==='desktop_action'&&c[1].action==='stop-all')).toHaveLength(1);panel.unmount();
  });
  it('executes an explicit verb + app query from Enter',async()=>{
    const panel=shallowMount(App);await flushPromises();
    const field=panel.find('input[type=search]');await field.setValue('停止 notes');await field.trigger('keydown',{key:'Enter'});await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_action',{action:'stop',installationId:'a'});
    expect((field.element as HTMLInputElement).value).toBe('');panel.unmount();
  });
  it('shows only operation results for > and restores the board when cleared',async()=>{
    const panel=shallowMount(App);await flushPromises();
    const field=panel.find('input[type=search]');await field.setValue('> notes');
    expect(panel.findComponent(TileBoard).exists()).toBe(false);
    expect(panel.find('[aria-label="操作搜索结果"]').text()).toContain('启动 Notes');
    expect(panel.find('.empty').exists()).toBe(false);
    await field.setValue('');expect(panel.findComponent(TileBoard).exists()).toBe(true);panel.unmount();
  });
});
