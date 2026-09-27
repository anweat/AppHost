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
