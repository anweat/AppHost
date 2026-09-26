export interface LogChunk { sequence: number; observed_ms: number; service_key: string; stream: string; bytes: number[] }
export interface LogResult {
  instance_id: string; read_at_ms: number; from_ms: number; to_ms: number; log_incomplete: boolean;
  services: {name: string; key: string}[];
  window: { chunks: LogChunk[]; prefix_evicted: boolean; incomplete_tail: boolean; truncated: boolean; sequence_gap: boolean; clock_reversed: boolean; warnings: {code: string; title: string}[] };
}
export function logEntries(value: LogResult) {
  const decoders = new Map<string, { decoder: TextDecoder; last: number }>();
  const entries = value.window.chunks.map(chunk => {
    const key = `${chunk.service_key}/${chunk.stream}`;
    if (!decoders.has(key)) decoders.set(key, { decoder: new TextDecoder(), last: 0 });
    const state = decoders.get(key)!;
    state.last = chunk.sequence;
    return { ...chunk, service: value.services.find(service => service.key === chunk.service_key)?.name || chunk.service_key,
      text: state.decoder.decode(Uint8Array.from(chunk.bytes), { stream:true }) };
  });
  for (const {decoder, last} of decoders.values()) {
    const entry = entries.find(entry => entry.sequence === last);
    if (entry) entry.text += decoder.decode();
  }
  return entries;
}
export function logWarnings(value: LogResult) {
  const w = value.window;
  return [value.log_incomplete && '采集记录不完整', w.truncated && '窗口达到读取上限，部分输出未显示',
    w.prefix_evicted && '较早输出已轮转清理', w.incomplete_tail && '日志索引尾部尚未写完',
    w.sequence_gap && '日志序号存在缺口', w.clock_reversed && '采集期间系统时间发生回退',
    ...w.warnings.map(item => `${item.code} · ${item.title}`)].filter(Boolean) as string[];
}
