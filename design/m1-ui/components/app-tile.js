'use strict';
// A tile only renders its installation; activity never changes its position.
function appTile(app,label,showBranch=false) {
  const marker=app.review?'?':({RUNNING:'▶',STOPPED:'■',FAILED:'!',STARTING:'◌',STOPPING:'◌'}[app.state]);
  const hint=app.review||app.state==='FAILED'?'查看详情':app.state==='RUNNING'?(app.port?'打开应用':'查看日志'):app.state==='STOPPED'?'启动应用':label;
  return `<article class="tile" data-installation="${app.id}" style="--tile:${app.color}">
    <button class="tile-main" data-action="launch" data-id="${app.id}" aria-label="${escape(app.name)}，${label}，${hint}" title="${escape(app.name)} · ${escape(app.branch)} · ${label}">
      <span class="tile-state" aria-hidden="true">${marker}</span>
      <span class="tile-icon">${icon(app.icon)}</span>
      <span class="tile-name">${escape(app.name)}</span>${showBranch?`<span class="tile-branch">${escape(app.branch)}</span>`:''}
      <span class="tile-arrow" aria-hidden="true">↗</span>
    </button>
    <button class="tile-menu" data-action="details" data-id="${app.id}" aria-label="${escape(app.name)} 详情" title="详情与编辑">···</button>
  </article>`;
}
