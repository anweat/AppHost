'use strict';
// Top-bar activity area (5.9): batch operations and global state. Nothing here pops up
// or takes focus. A running batch shows progress; a finished one leaves a summary chip
// that expands into per-item results. Global states (disconnected, wake paused) stay as
// chips until they end; short notices carry at most one follow-up such as Undo.
// Item status: pending | working | done | failed | skipped | canceled | orphan.
const activity=(()=>{
  const root=document.querySelector('#activity');
  const panel=document.createElement('div');panel.className='act-panel';panel.id='activity-panel';panel.hidden=true;
  panel.setAttribute('role','region');panel.setAttribute('aria-label','操作结果');
  root.closest('.topbar').append(panel);
  let op=null,notice=null,offline=null,paused=false,noticeTimer,settleTimer,seq=0;
  const STATUS={pending:'等待',canceled:'已取消启动',orphan:'遗留进程 · 未结束'};
  const tone=item=>({failed:'fail',orphan:'fail',skipped:'neutral',canceled:'neutral'})[item.status]||'ok';
  const statusText=item=>item.status==='working'?op.verbs.working:item.status==='done'?(item.text||op.verbs.done):item.status==='failed'?`失败：${item.text}`:item.status==='skipped'?`跳过：${item.text}`:item.status==='orphan'?`${STATUS.orphan} · ${item.text}`:STATUS[item.status];
  function counts(){const c={};for(const item of op.items)c[item.status]=(c[item.status]||0)+1;return c;}
  const finished=()=>op&&op.items.every(i=>!['pending','working'].includes(i.status));
  function summary(){
    const c=counts(),parts=[];
    if(!finished())return `${op.verbs.working} ${op.items.length-(c.pending||0)-(c.working||0)}/${op.items.length}`;
    if(c.done)parts.push(`${op.verbs.done} ${c.done}`);
    if(c.canceled)parts.push(`取消 ${c.canceled}`);
    if(c.failed)parts.push(`失败 ${c.failed}`);
    if(c.skipped)parts.push(`跳过 ${c.skipped}`);
    if(c.orphan)parts.push(`遗留 ${c.orphan}`);
    return parts.join(' · ')||op.verbs.done;
  }
  function chip(kind,content,attrs=''){return `<div class="act-chip" data-kind="${kind}" ${attrs}>${content}</div>`;}
  function paint(){
    const chips=[];
    if(offline)chips.push(chip('offline',`<span class="act-dot" aria-hidden="true"></span><span class="act-label">未连接 · 显示 ${offline} 的状态</span><button class="act-btn" data-action="disconnect">重连</button>`));
    if(paused)chips.push(chip('paused',`${icon('pause')}<span class="act-label">自动唤醒已暂停</span><button class="act-btn" data-action="resume">恢复</button>`));
    if(op){
      const done=finished(),c=counts(),t=c.failed||c.orphan?'fail':c.skipped||c.canceled?'neutral':'ok';
      chips.push(`<button class="act-chip act-op" data-kind="op" data-tone="${done?t:'busy'}" data-action="activity-toggle" aria-expanded="${!panel.hidden}" aria-controls="activity-panel"><span class="act-spin" aria-hidden="true"></span><span class="act-label"><b>${escape(op.title)}</b> ${escape(summary())}</span></button>`);
    }
    if(notice)chips.push(chip('notice',`<span class="act-label">${escape(notice.text)}</span>${notice.action?`<button class="act-btn" data-action="activity-notice">${escape(notice.action.label)}</button>`:''}`));
    root.innerHTML=chips.join('');
    if(!panel.hidden){paintPanel();place();}
  }
  // The results panel hangs under its chip, kept inside the bar.
  function place(){const chip=root.querySelector('.act-op'),bar=root.closest('.topbar');if(chip)panel.style.left=`${Math.max(8,Math.min(chip.offsetLeft,bar.clientWidth-panel.offsetWidth-8))}px`;}
  function paintPanel(){
    if(!op){panel.hidden=true;return;}
    panel.innerHTML=`<div class="act-head"><div><b>${escape(op.title)}</b><span>${escape(op.time)} · ${escape(summary())}</span></div><button class="co-close" data-action="activity-close" aria-label="关闭结果">×</button></div>
      ${op.note?`<p class="act-note">${escape(op.note)}</p>`:''}
      <ul class="act-list">${op.items.map(item=>{const app=item.appId&&getApp(item.appId);return `<li data-tone="${tone(item)}" data-status="${item.status}">${app?`<span class="pal-badge" style="${idStyle(app)}">${glyph(app)}</span>`:`<span class="pal-icon">${icon('activity')}</span>`}<span class="act-name">${escape(item.name)}</span><span class="act-status">${escape(statusText(item))}</span>${app&&['failed','skipped'].includes(item.status)?`<button class="act-btn" data-action="details" data-id="${escape(app.id)}">查看</button>`:''}</li>`;}).join('')}</ul>
      <div class="act-foot"><button class="text-button" data-action="activity-journal">在操作记录中查看</button></div>`;
  }
  function begin({title,verbs,items,note}){
    // The newest batch replaces a lingering notice, so the bar never stacks up.
    clearTimeout(settleTimer);clearTimeout(noticeTimer);notice=null;
    op={id:++seq,title,verbs,note,time:new Date().toLocaleTimeString('zh-CN',{hour12:false,hour:'2-digit',minute:'2-digit'}),items:items.map(i=>({status:'pending',...i}))};
    paint();settle();
    const id=op.id;
    return {set(key,status,text){if(!op||op.id!==id)return;const item=op.items.find(i=>i.key===key);if(!item)return;item.status=status;item.text=text;paint();settle();}};
  }
  // A clean result retires itself; anything failed, skipped or left behind waits to be read.
  function settle(){
    if(!finished()||op.settled)return;
    op.settled=true;const c=counts();announce(`${op.title}：${summary()}`);
    clearTimeout(settleTimer);
    if(!c.failed&&!c.orphan&&!c.skipped)settleTimer=setTimeout(()=>{if(panel.hidden){op=null;paint();}},5000);
  }
  function toggle(){panel.hidden=!panel.hidden;if(!panel.hidden)paintPanel();paint();}
  function close(){panel.hidden=true;if(finished())op=null;paint();root.querySelector('.act-op')?.focus();}
  function showNotice(text,action){clearTimeout(noticeTimer);notice={text,action};paint();announce(text);noticeTimer=setTimeout(()=>{notice=null;paint();},action?8000:4500);}
  function runNotice(){const run=notice?.action?.run;notice=null;paint();run?.();}
  document.addEventListener('pointerdown',e=>{if(!panel.hidden&&!panel.contains(e.target)&&!root.contains(e.target)){panel.hidden=true;paint();}},true);
  panel.addEventListener('keydown',e=>{if(e.key==='Escape'){e.preventDefault();e.stopPropagation();close();}});
  return {begin,toggle,close,notice:showNotice,runNotice,
    setOffline(since){offline=since;paint();},setPaused(value){paused=value;paint();},
    reset(){op=null;notice=null;offline=null;paused=false;panel.hidden=true;paint();}};
})();
