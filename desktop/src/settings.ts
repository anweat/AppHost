export interface Warning {code: string; message?: string; title?: string}
export interface Baseline {schema_version: number; path: string; capture?: {shell: string; captured_at_ms: number}; warnings?: Warning[]}
export interface Retention {schema_version: number; days: number; max_mib: number; warnings?: Warning[]}
export interface EnvironmentCheck {installation_id: string; services: {name: string; cwd: string; executable: string}[]}
export function warningsText(warnings?: Warning[]) { return warnings?.map(w => `${w.code} · ${w.message || w.title || '操作记录不完整'}`).join('；') || ''; }
export function validRetention(days: unknown, mib: unknown) { return typeof days === 'number' && Number.isInteger(days) && days >= 1 && days <= 365 && typeof mib === 'number' && Number.isInteger(mib) && mib >= 10 && mib <= 10240; }
