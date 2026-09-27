// @vitest-environment happy-dom
import { mount, flushPromises } from '@vue/test-utils';
import { describe, it, expect, vi } from 'vitest';
const { invoke } = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke}));
import ApplicationSettings from './components/ApplicationSettings.vue';
const app = {installation_id:'app',project_id:'p',label:'Example',path:'/project',registration_state:'REGISTERED',requires_confirmation:false,runtime:null};
const button = (panel:ReturnType<typeof mount>, text:string) => panel.findAll('button').find(b => b.text() === text)!;
describe('desktop registration management', () => {
  it('requires explicit detach confirmation and preserves the selected installation identity', async () => {
    invoke.mockReset().mockResolvedValue({completed:true});
    const panel = mount(ApplicationSettings,{props:{apps:[app],installationId:'app'}});
    await button(panel,'解除登记…').trigger('click'); expect(invoke).not.toHaveBeenCalled();
    await button(panel,'停止并解除登记').trigger('click'); await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_detach',{installationId:'app'});
    expect(panel.text()).toContain('已解除登记，项目文件已保留'); panel.unmount();
  });
  it('a canceled native relink does not report success or refresh registrations', async () => {
    invoke.mockReset().mockResolvedValue(null);
    const panel = mount(ApplicationSettings,{props:{apps:[app],installationId:'app'}});
    await button(panel,'重新关联路径…').trigger('click'); await flushPromises();
    expect(panel.emitted('changed')).toBeUndefined(); expect(panel.text()).not.toContain('已重新关联'); panel.unmount();
  });
  it('does not treat an accepted mutation as completed and surfaces its final error', async () => {
    let finish!:(value:unknown)=>void;
    invoke.mockReset().mockImplementation((cmd:string) => cmd === 'desktop_detach' ? Promise.resolve({operation_id:'op'}) : new Promise(resolve => {finish=resolve;}));
    const panel = mount(ApplicationSettings,{props:{apps:[app],installationId:'app'}});
    await button(panel,'解除登记…').trigger('click'); await button(panel,'停止并解除登记').trigger('click'); await flushPromises();
    expect(panel.text()).toContain('等待服务停止'); expect(panel.text()).not.toContain('已解除登记');
    finish({completed:true,error:{code:'STOP_FAILED',message:'服务尚未结束'}}); await flushPromises();
    expect(panel.text()).toContain('STOP_FAILED'); expect(invoke.mock.calls.filter(c => c[0] === 'desktop_detach')).toHaveLength(1); panel.unmount();
  });
});
