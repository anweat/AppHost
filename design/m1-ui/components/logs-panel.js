'use strict';
// Refresh updates the observation time, not fabricated service output.
function logsPanel(app,connected,readAt) {
  const text=app.state==='FAILED'?'[14:32:08] 端口 5173 已被占用\n[14:32:08] node · PID 18273\n[14:32:08] 未启动服务。':app.state==='STOPPED'?'当前没有活动实例。':'[14:30:02] 正在启动 web\n[14:30:03] 服务已就绪\n[14:30:03] GET / 200\n[14:30:06] GET /assets/main.css 200';
  return `<section aria-label="应用日志"><div class="panel-toolbar"><span class="muted">${connected?'示例日志':'上次日志 · 已离线'}</span><button data-action="refresh-logs" data-id="${app.id}" aria-label="刷新日志">↻ 刷新</button></div><pre tabindex="0">${escape(text)}</pre><p class="muted panel-status" role="status">${readAt?`已刷新 ${readAt} · 无新增输出`:''}</p></section>`;
}
