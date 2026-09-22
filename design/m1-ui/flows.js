'use strict';
let importSelection=['studio','api'];
const candidates=[{id:'studio',name:'Motion Studio',command:'npm run dev',port:4000,icon:'canvas',color:'#3e6977'},{id:'api',name:'FastAPI Notes',command:'uv run uvicorn main:app',port:8000,icon:'note',color:'#87613a'}];
function importDirectory(){showModal('添加应用',`<div class="step-line"><b>01 选择目录</b><span>—</span><span>02 检查候选</span><span>—</span><span>03 确认声明</span></div><h3>应用放在哪个目录？</h3><p class="subtitle">只扫描选定目录。目录建议不会触发自动扫描。</p><div class="directory-choice"><input id="import-path" aria-label="示例目录" readonly placeholder="尚未选择目录"><button data-action="choose-directory">选择示例目录</button></div><p class="note">原型使用 ~/Projects/demo-workspace 演示，不读取你的文件。</p>`,`<button data-action="close-modal">取消</button><button id="scan-button" class="primary" data-action="scan" disabled>扫描目录</button>`);}
function showCandidates(){showModal('检查发现的应用',`<div class="step-line"><span>01 选择目录</span><span>—</span><b>02 检查候选</b><span>—</span><span>03 确认声明</span></div><p class="muted">~/Projects/demo-workspace · 发现 2 个应用和 1 个库</p>${candidates.map(c=>`<label class="candidate"><input type="checkbox" name="candidate" value="${c.id}" ${apps.some(a=>a.sourceId===c.id)?'disabled':importSelection.includes(c.id)?'checked':''}><div><strong>${c.name}</strong><small>${apps.some(a=>a.sourceId===c.id)?'已登记 · 不重复导入':c.id==='studio'?'Node · package.json':'Python · pyproject.toml'}</small><code>${c.command}</code></div></label>`).join('')}<label class="candidate disabled"><input type="checkbox" disabled><div><strong>shared-utils</strong><small>仅库 / 构建任务，未发现可运行服务；不自动选中。</small></div></label><p id="import-error" class="form-error" role="alert"></p>`,`<button data-action="import">上一步</button><button class="primary" data-action="confirm-import">检查启动声明</button>`);}
function confirmImport(){importSelection=[...modal.querySelectorAll('input[name=candidate]:checked')].map(x=>x.value);if(!importSelection.length){document.querySelector('#import-error').textContent='请至少选择一个应用。';return;}showModal('确认启动声明',`<div class="step-line"><span>01 选择目录</span><span>—</span><span>02 检查候选</span><span>—</span><b>03 确认声明</b></div>${candidates.filter(c=>importSelection.includes(c.id)).map(c=>`<div class="candidate"><div><strong>${c.name}</strong><small>工作目录：~/Projects/demo-workspace/${c.id}</small><code>命令：${c.command}<br>端口：${c.port}</code></div></div>`).join('')}<label class="check-label"><input id="confirm-declaration" type="checkbox"><span>我已核对所选应用的启动声明，允许导入并创建缺失的 apphost.toml。</span></label><p class="note">导入不会启动应用。确认仅绑定声明内容，不验证脚本和依赖是否可信。</p>`,`<button data-action="candidates">上一步</button><button class="primary" id="commit-import" data-action="commit-import" disabled>确认导入 ${importSelection.length} 个应用</button>`);}
function diagnostic(app){const text=`AppHost 诊断信息（示例 · 已脱敏）\n应用：${app.name}\n状态：FAILED\n原因：PORT_IN_USE\n阶段：启动前检查\n端口：5173\n占用者：node · PID 18273（外部进程）\n项目路径：~/Projects/<project>\n环境变量值：[已隐藏]\n建议：释放端口后重试，或修改启动声明。\n未执行依赖安装，未终止外部进程。`;showModal('预览诊断信息',`<p class="muted">复制前请检查内容。默认隐藏环境变量值及敏感路径；自动脱敏可能遗漏应用输出中的秘密。</p><div style="height:16px"></div><pre id="diagnostic-text" tabindex="0">${escape(text)}</pre>`,`<button data-action="close-modal">关闭</button><button class="primary" data-action="copy-diagnostic">复制已脱敏信息</button>`);}
// Compare the edited text to the last confirmed example, never to a fixed fake diff.
function declarationDiff(app,next){
  const previous=app.confirmedDeclaration || (app.id==='vision'&&app.review?declaration(app).replace('["npm","run","dev"]','["npm","start"]'):declaration(app));
  const before=previous.split('\n'), after=next.split('\n');
  const changes=before.filter((line,index)=>line!==after[index]).map(line=>`<div class="removed">− ${escape(line)}</div>`).join('')+after.filter((line,index)=>line!==before[index]).map(line=>`<div class="added">＋ ${escape(line)}</div>`).join('');
  return `<div class="diff">${changes || '声明内容未变化。'}</div>`;
}
function reviewApp(app){showModal('复核启动声明',`<p><strong>${escape(app.name)}</strong></p>${declarationDiff(app,declaration(app))}<pre>${escape(declaration(app))}</pre><label class="check-label"><input id="confirm-declaration" type="checkbox"><span>我已复核当前声明，并确认按此声明启动。</span></label>`,`<button data-action="close-modal">暂不确认</button><button class="primary" id="commit-import" data-action="commit-review" data-id="${app.id}" disabled>确认并启动</button>`);}
function editApp(app,text=declaration(app)){showModal('编辑启动声明',`<label for="manifest-editor" class="muted">${escape(app.name)} · apphost.toml（示例副本）</label><div style="height:12px"></div><textarea id="manifest-editor" class="code-editor" spellcheck="false">${escape(text)}</textarea><p id="edit-error" class="form-error" role="alert"></p><p class="note">下一步展示差异和完整声明，由你明确保存并确认。${active(app)?'保存前先停止全部服务。':''}原型仅演示必填项提示，正式校验由核心执行。</p>`,`<button data-action="close-modal">取消</button><button class="primary" data-action="save-manifest" data-id="${app.id}">检查修改</button>`);}
function acceptDeclaration(app,text){app.declaration=text;app.confirmedDeclaration=text;app.review=false;const port=text.match(/\bport\s*=\s*(\d+)/);app.port=port?Number(port[1]):null;}
async function handleFlow(action,app,button){
  switch(action){
    case 'import':importSelection=candidates.filter(c=>!apps.some(a=>a.sourceId===c.id)).map(c=>c.id);importDirectory();break;
    case 'choose-directory':document.querySelector('#import-path').value='~/Projects/demo-workspace';document.querySelector('#scan-button').disabled=false;break;
    case 'scan':case 'candidates':showCandidates();break;
    case 'confirm-import':confirmImport();break;
    case 'commit-import':{
      if(!document.querySelector('#confirm-declaration').checked||!requireConnection())return;
      const chosen=candidates.filter(c=>importSelection.includes(c.id));
      for(const c of chosen)apps.push({...c,sourceId:c.id,id:`${c.id}-${++nextId}`,projectId:`00000000-0000-4000-8000-${String(100+nextId).padStart(12,'0')}`,path:`~/Projects/demo-workspace/${c.id}`,branch:'main',state:'STOPPED'});
      event(`导入 ${chosen.length} 个应用并确认声明`);closeModal();view='home';query='';document.querySelector('#search').value='';render();toast(`已导入 ${chosen.length} 个应用，点击磁贴即可启动。`);break;
    }
    case 'diagnostic':diagnostic(app);break;
    case 'copy-diagnostic':{
      const text=document.querySelector('#diagnostic-text').textContent;
      try{await navigator.clipboard.writeText(text);toast('已复制脱敏诊断信息。');}
      catch{const range=document.createRange();range.selectNodeContents(document.querySelector('#diagnostic-text'));window.getSelection().removeAllRanges();window.getSelection().addRange(range);toast('浏览器未允许自动复制。已选中内容，请按 ⌘C / Ctrl+C。');}break;
    }
    case 'review':reviewApp(app);break;
    case 'commit-review':if(document.querySelector('#confirm-declaration').checked&&requireConnection()){acceptDeclaration(app,declaration(app));event(`确认 ${app.name} 的启动声明`);closeModal();startApp(app);}break;
    case 'edit':editApp(app,modal.querySelector('#pending-declaration')?.textContent || declaration(app));break;
    case 'save-manifest':{
      const text=document.querySelector('#manifest-editor').value;
      if(!text.includes('[[services]]')||!text.includes('command =')){document.querySelector('#edit-error').textContent='声明缺少 [[services]] 或 command。请检查后再保存。';return;}
      if(!requireConnection())return;
      showModal('保存并确认启动声明',`<p><strong>${escape(app.name)}</strong></p>${declarationDiff(app,text)}<pre id="pending-declaration">${escape(text)}</pre><label class="check-label"><input id="confirm-declaration" type="checkbox"><span>我已核对差异和完整声明，允许保存并确认。</span></label><p class="note">${active(app)?'确认后先停止全部服务，再保存。':''}保存不会自动启动应用。确认声明不代表脚本及依赖可信。</p>`,`<button data-action="edit" data-id="${app.id}">返回编辑</button><button class="primary" id="commit-import" data-action="commit-manifest" data-id="${app.id}" disabled>保存并确认启动声明</button>`);break;
    }
    case 'commit-manifest':{
      if(!document.querySelector('#confirm-declaration').checked||!requireConnection())return;
      const text=document.querySelector('#pending-declaration').textContent;
      closeModal();const save=()=>{acceptDeclaration(app,text);app.state='STOPPED';event(`保存并确认 ${app.name} 的启动声明`);update();toast('声明已保存并确认，应用保持停止。');};if(active(app))stopApp(app,save);else save();break;
    }
    case 'detach':showModal(`解除 ${escape(app.name)} 的登记？`,`<p>${active(app)?'先停止这个应用的全部服务，再解除登记。':'解除后，该应用将从面板移除。'}</p><p class="note">项目目录、源码、apphost.toml 和已有日志均保留，不删除项目文件。</p>`,`<button data-action="close-modal">取消</button><button class="danger" data-action="commit-detach" data-id="${app.id}">${active(app)?'停止并解除登记':'解除登记'}</button>`);break;
    case 'commit-detach':{
      if(!requireConnection())return;closeModal();const remove=()=>{apps=apps.filter(a=>a!==app);event(`解除 ${app.name} 登记，项目文件保留`);detail.close();render();toast('已解除登记，项目文件保留。');};if(active(app))stopApp(app,remove);else remove();break;
    }
    case 'relink':showModal('重新关联项目路径',`<p>${escape(app.name)}</p><label class="section-label" for="relink-path">新的项目根目录</label><input id="relink-path" class="full" value="~/Projects/${escape(app.id)}" aria-label="新的示例项目根目录"><p class="note">${active(app)?'先停止当前应用，再验证新路径。':'验证项目身份和声明一致后才生效。'}登记身份保留。这里只演示路径选择，不访问文件。</p>`,`<button data-action="close-modal">取消</button><button class="primary" data-action="commit-relink" data-id="${app.id}">${active(app)?'停止并验证路径':'验证并关联'}</button>`);break;
    case 'commit-relink':{
      if(!requireConnection())return;const path=document.querySelector('#relink-path').value.trim();if(!path){toast('请选择项目根目录。');return;}closeModal();const relink=()=>{app.path=path;event(`重新关联 ${app.name} 的示例路径`);update();toast('示例路径已关联；正式版本会验证项目身份与当前声明。');};if(active(app))stopApp(app,relink);else relink();break;
    }
    case 'environment':showModal('运行环境 PATH',`<p class="muted">来源：${preferences.pathSaved?'已保存的主动采集结果':'系统路径文件 + 已存在的 Homebrew bin'}（示例）</p><div style="height:16px"></div><pre>${preferences.pathSaved?'~/.nvm/versions/node/v22/bin\n/opt/homebrew/bin\n/usr/bin\n/bin':'/usr/local/bin\n/usr/bin\n/bin\n/usr/sbin\n/sbin\n/opt/homebrew/bin'}</pre><p class="note">主动采集会运行交互式登录 shell 的初始化脚本，最多等待 5 秒。采集结果需要你确认后才保存。</p>`,`<button data-action="close-modal">关闭</button><button class="primary" data-action="capture-path">主动采集示例 PATH</button>`);break;
    case 'capture-path':showModal('确认采集结果',`<p>这是模拟采集结果，没有执行你的 shell。</p><div style="height:16px"></div><pre>~/.nvm/versions/node/v22/bin\n/opt/homebrew/bin\n/usr/bin\n/bin</pre><p class="note">保存只影响后续启动，不修改正在运行应用的环境。</p>`,`<button data-action="close-modal">放弃</button><button class="primary" data-action="save-path">保存示例结果</button>`);break;
    case 'save-path':if(!requireConnection())return;preferences.pathSaved=true;event('保存共享 PATH 示例');closeModal();toast('示例环境已保存。');break;
  }
}
document.addEventListener('change',e=>{
  if(e.target.id==='confirm-declaration')document.querySelector('#commit-import').disabled=!e.target.checked;
  if(e.target.dataset.setting){if(!requireConnection()){e.target.checked=preferences[e.target.dataset.setting];return;}preferences[e.target.dataset.setting]=e.target.checked;event(`${e.target.checked?'启用':'关闭'}${e.target.dataset.setting==='login'?'登录自启':'托盘常驻'}（示例）`);toast('仅更改此原型的示例设置。');}
});
