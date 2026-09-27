export interface AdviceAction { id: string; title: string; primary: boolean; argv: string[]; cwd?: string; copy_only?: boolean }
export interface FailureReport {
  failure: null | { failure_id: string; instance_id?:string|null; at_ms: number; service: string | null; phase: string; collecting: boolean; gaps: string[]; stderr_tail: string; details?: FailureDetails };
  details_expired: boolean;
  records?: {failure_id:string;at_ms:number;code:string}[];
  changes_since_success?: {available:boolean;reason?:string;changes?:{field:string;before:unknown;after:unknown}[]};
  history: { last_success_at_ms?: number | null; last_failure?: {failure_id:string} | null };
  advice?: { code: string; title: string; impact: string; clears_when: string; consecutive_same_cause: number;
    evidence: { inferred: boolean; excerpt?: string | null; line?: number | null; rule_id: string };
    actions: AdviceAction[]; hints: {code: string; title: string}[] };
}
export interface DiagnosticPreview { sections: {kind: string; value: unknown}[]; omitted: string[]; redaction: string; redaction_context_available: boolean }
const titles: Record<string, string> = { application:'应用', command:'命令（当前已确认声明，非历史 argv）', environment:'运行环境', category:'失败类别与建议', evidence:'证据', output_tail:'输出末尾', last_success:'上次成功' };
export function diagnosticText(value: DiagnosticPreview) {
  return ['AppHost 诊断信息', `脱敏：${value.redaction}`, `脱敏上下文可用：${value.redaction_context_available ? '是' : '否'}`,
    ...value.sections.map(section => `${titles[section.kind] || section.kind}\n${JSON.stringify(section.value, null, 2)}`),
    `未包含的内容\n${value.omitted.map(item => `- ${item}`).join('\n')}`].join('\n\n');
}
// Advice is copied for a person to review; it is never sent to a shell.
export function commandText(argv: string[]) {
  return argv.map(arg => /^[a-zA-Z0-9_./:@=-]+$/.test(arg) ? arg : `'${arg.replaceAll("'", "'\\''")}'`).join(' ');
}

interface OwnerEvidence {port?:number;installation_id?:string;instance_id?:string;ownership?:{owners?:{pid:number;name?:string}[];incomplete?:boolean}}
interface FailureDetails extends OwnerEvidence {details?:OwnerEvidence;operation_error?:{details?:OwnerEvidence};services?:{name:string;port?:number;observed_listeners?:{ports?:number[];incomplete?:boolean}}[]}
export function failureEvidence(report:FailureReport) {
  const details = report.failure?.details;
  const owner = details?.operation_error?.details || details?.details || details;
  return {owner, listeners:details?.services?.filter(s => s.observed_listeners) || []};
}
