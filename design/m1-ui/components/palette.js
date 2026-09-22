'use strict';
// Search box + action index. This file owns only the listbox UI and keyboard model;
// app.js supplies results through setProvider(query) => {sections:[{title,items}]}.
// Items: {label (safe HTML), sub, visual (safe HTML), hint, run, alt}. Enter runs,
// Shift+Enter runs the alternate action (for apps: the other of open / details).
const palette=(()=>{
  const input=document.querySelector('#search'),wrap=document.querySelector('.search-wrap');
  const panel=document.createElement('div');
  panel.className='palette';panel.id='palette';panel.setAttribute('role','listbox');panel.setAttribute('aria-label','搜索结果与操作');panel.hidden=true;
  wrap.after(panel);
  input.setAttribute('role','combobox');input.setAttribute('aria-controls','palette');input.setAttribute('aria-expanded','false');input.setAttribute('aria-autocomplete','list');
  let provider=()=>({sections:[]}),items=[],active=0;
  function paint(){
    const {sections}=provider(input.value);
    items=sections.flatMap(section=>section.items);
    active=Math.min(active,Math.max(items.length-1,0));
    let n=0;
    const body=sections.filter(section=>section.items.length).map(section=>`<div class="pal-section" role="group" aria-label="${escape(section.title)}"><div class="pal-title" aria-hidden="true">${escape(section.title)}</div>${section.items.map(item=>{const i=n++;return `<div class="pal-item" role="option" id="pal-${i}" data-index="${i}" aria-selected="${i===active}">${item.visual}<span class="pal-label">${item.label}</span>${item.sub?`<span class="pal-sub">${escape(item.sub)}</span>`:''}${item.hint?`<kbd>${escape(item.hint)}</kbd>`:''}</div>`;}).join('')}</div>`).join('');
    panel.innerHTML=`<div class="pal-body">${body||'<div class="pal-empty">没有匹配的应用或操作</div>'}</div><div class="pal-foot" aria-label="搜索语法"><button type="button" data-insert="is:">is:<i>状态</i></button><button type="button" data-insert="in:">in:<i>分组</i></button><button type="button" data-insert="&gt;">&gt;<i>操作</i></button><span class="pal-keys"><kbd>↵</kbd>执行 <kbd>⇧↵</kbd>备选 <kbd>esc</kbd>清除</span></div>`;
    input.setAttribute('aria-activedescendant',items.length?`pal-${active}`:'');
    panel.querySelector('[aria-selected=true]')?.scrollIntoView({block:'nearest'});
  }
  function select(index){
    active=index;
    for(const el of panel.querySelectorAll('.pal-item'))el.setAttribute('aria-selected',String(Number(el.dataset.index)===active));
    input.setAttribute('aria-activedescendant',`pal-${active}`);
    panel.querySelector('[aria-selected=true]')?.scrollIntoView({block:'nearest'});
  }
  // Align with the search box, but stay inside the top bar on narrow windows.
  function place(){const bar=wrap.parentElement;panel.style.left=`${Math.max(8,Math.min(wrap.offsetLeft,bar.clientWidth-panel.offsetWidth-8))}px`;}
  function show(){if(panel.hidden){panel.hidden=false;panel.classList.remove('in');void panel.offsetWidth;panel.classList.add('in');}input.setAttribute('aria-expanded','true');paint();place();}
  function hide(){panel.hidden=true;input.setAttribute('aria-expanded','false');input.removeAttribute('aria-activedescendant');}
  function run(index,alternate){
    const item=items[index];if(!item)return;
    // Executing returns the canvas to its full, unfiltered layout.
    hide();input.blur();if(input.value){input.value='';input.dispatchEvent(new Event('input'));}
    (alternate&&item.alt?item.alt:item.run)();
  }
  input.addEventListener('focus',show);
  input.addEventListener('input',()=>{active=0;if(document.activeElement===input)show();});
  input.addEventListener('keydown',e=>{
    if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();if(panel.hidden)show();if(items.length)select((active+(e.key==='ArrowDown'?1:-1)+items.length)%items.length);}
    else if(e.key==='Enter'){e.preventDefault();run(active,e.shiftKey);}
    else if(e.key==='Escape'){
      e.preventDefault();e.stopPropagation();
      if(input.value){input.value='';input.dispatchEvent(new Event('input'));}else{hide();input.blur();}
    }
  });
  input.addEventListener('blur',()=>setTimeout(()=>{if(document.activeElement!==input)hide();},0));
  // Keep focus in the input while clicking results or syntax chips.
  panel.addEventListener('pointerdown',e=>e.preventDefault());
  panel.addEventListener('click',e=>{
    const chip=e.target.closest('[data-insert]');
    if(chip){const prefix=chip.dataset.insert;input.value=`${input.value.trimEnd()}${input.value.trim()?' ':''}${prefix}`;input.dispatchEvent(new Event('input'));input.focus();return;}
    const item=e.target.closest('.pal-item');if(item)run(Number(item.dataset.index),e.shiftKey);
  });
  panel.addEventListener('pointermove',e=>{const item=e.target.closest('.pal-item');if(item&&Number(item.dataset.index)!==active)select(Number(item.dataset.index));});
  return {setProvider(next){provider=next;},refresh(){if(!panel.hidden)paint();},hide};
})();
