export type State = 'STOPPED' | 'STARTING' | 'RUNNING' | 'STOPPING' | 'FAILED';
export interface CoreError { code: string; message: string }
export interface Instance {
  installation_id: string; instance_id: string; updated_at_ms: number;
  operation_id?: string; operation_done?: boolean; operation_error?: CoreError | null;
  status: { state: State; failure: string | null };
  services: { name: string; port: number | null; alive: boolean; ready: boolean }[];
}
export interface AppEntry {
  installation_id: string; project_id: string; label: string; path: string; group?: string; group_label?: string;
  registration_state: string; requires_confirmation: boolean; has_entry?: boolean; runtime: Instance | null;
}
export interface Snapshot {
  apps: AppEntry[]; live: boolean; automatic_wake_paused?: boolean; read_at_ms: number; runtime_error: CoreError | null;
}
const labels: Record<State, string> = {
  STOPPED: '已停止', STARTING: '正在启动', RUNNING: '运行中', STOPPING: '正在停止', FAILED: '运行失败',
};
export function stateLabel(app: AppEntry, snapshot: Snapshot) {
  if (app.runtime) return `${snapshot.live ? '' : '上次记录 · '}${labels[app.runtime.status.state]}`;
  return snapshot.runtime_error ? '状态未知' : '尚未启动';
}
export function tileState(app: AppEntry) {
  if (app.registration_state === 'SOURCE_MISSING') return 'missing';
  if (app.requires_confirmation) return 'review';
  return app.runtime?.status.state.toLowerCase() ?? 'stopped';
}
export function hasLiveServices(app: AppEntry, snapshot: Snapshot) {
  return snapshot.live && !!app.runtime?.services.some(service => service.alive);
}
export function identityHue(projectId: string) {
  const hues = [115, 145, 170, 195, 220, 245, 270, 295, 320];
  let hash = 0x811c9dc5;
  for (const char of projectId) hash = Math.imul(hash ^ char.charCodeAt(0), 0x01000193) >>> 0;
  return hues[hash % hues.length];
}
export function errorMessage(error: unknown): CoreError {
  if (error && typeof error === 'object' && 'code' in error && 'message' in error)
    return { code: String(error.code), message: String(error.message) };
  return { code: 'DESKTOP_READ_FAILED', message: String(error) };
}
