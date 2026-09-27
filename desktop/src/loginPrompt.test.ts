// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
const {invoke}=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke,isTauri:()=>true}));
import FirstRunLogin from './components/FirstRunLogin.vue';
describe('first launch login choice',()=>{
  it.each([true,false])('writes only after an explicit %s choice and hides only after saving',async enabled=>{
    invoke.mockReset().mockImplementation(async cmd=>({choice_made:cmd==='desktop_login_save'}));
    const panel=mount(FirstRunLogin);await flushPromises();
    expect(panel.find('section').exists()).toBe(true);expect(invoke).toHaveBeenCalledTimes(1);
    expect(panel.findAll('input')).toHaveLength(0);
    await panel.findAll('button')[enabled?0:1]!.trigger('click');await flushPromises();
    expect(invoke).toHaveBeenCalledWith('desktop_login_save',{enabled});expect(panel.find('section').exists()).toBe(false);panel.unmount();
  });
  it('does not prompt again for an existing explicit choice',async()=>{
    invoke.mockReset().mockResolvedValue({choice_made:true});const panel=mount(FirstRunLogin);await flushPromises();
    expect(panel.find('section').exists()).toBe(false);expect(invoke).toHaveBeenCalledTimes(1);panel.unmount();
  });
  it('keeps both choices after a failed save without claiming completion',async()=>{
    invoke.mockReset().mockImplementation(async cmd=>{if(cmd==='desktop_login_save')throw {code:'IO_ERROR',message:'写入失败'};return {choice_made:false};});
    const panel=mount(FirstRunLogin);await flushPromises();await panel.findAll('button')[1]!.trigger('click');await flushPromises();
    expect(panel.text()).toContain('写入失败');expect(panel.findAll('button')).toHaveLength(2);panel.unmount();
  });
});
