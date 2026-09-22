'use strict';
// This is a system-terminal entry, not an embedded shell or a second launch path.
function terminalPanel(app) {
  return `<section aria-label="项目命令行"><p class="muted">项目目录</p><pre class="terminal-path">${escape(app.path || `~/Projects/${app.id}`)}</pre><button class="full" data-action="open-terminal" data-id="${app.id}">打开系统终端 ↗</button><p class="note">仅打开目录，不自动执行启动或修复命令。</p></section>`;
}
