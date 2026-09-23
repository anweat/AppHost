'use strict';
// Failure records and how the desktop explains them (M1 5.7, D9, 10.4).
// A record is shaped like the daemon's versioned failure JSON: code, service, phase,
// evidence, output tail with the matched line, and whether the category is inferred.
// explainFailure() turns one record into what every surface shows: one sentence, the
// impact, evidence, an optional copy-only command, and exactly one primary action.

// Actions are ordinary [data-action] buttons, so callouts, the drawer and menus share
// one click handler. `focus` tells the editor which line to put the caret on.
const ACT={
  retry:{label:'重试启动',action:'retry',icon:'restart'},
  restart:{label:'完整重启',action:'restart',icon:'restart'},
  stop:{label:'停止全部服务',action:'stop',icon:'stop'},
  details:{label:'查看原因',action:'details',icon:'info'},
  diagnostic:{label:'预览诊断信息',action:'diagnostic',icon:'logs'},
  terminal:{label:'在终端打开项目目录',action:'open-terminal',icon:'command'},
  environment:{label:'查看运行环境',action:'environment',icon:'sliders'},
  edit:focus=>({label:'修改启动声明',action:'edit',icon:'edit',focus})
};
const PHASE_LABEL={preflight:'启动前检查',startup:'启动后短时间内',readiness:'等待就绪',running:'运行中'};

function explainFailure(app,f){
  const service=f.service?`服务 ${f.service}`:'服务';
  const exited=f.after?`${service} 启动 ${f.after}后退出（exit ${f.exitCode}）`:'';
  switch(f.code){
    case 'PORT_IN_USE':{
      const holder=f.holderId&&getApp(f.holderId);
      if(holder)return {
        title:`端口 ${f.port} 正被 ${holder.name} 使用`,
        impact:`${app.name} 没有启动；${holder.name} 不受影响。`,
        evidence:[['占用者',`${holder.name} · 由 AppHost 管理`],['原因','两个安装声明了同一个固定端口，不能同时运行'],['检查阶段','启动前检查，没有启动任何服务']],
        primary:{label:`停止 ${holder.name} 并启动`,action:'stop-holder-start',icon:'play'},
        secondary:[ACT.edit('port'),ACT.diagnostic]
      };
      return {
        title:`端口 ${f.port} 已被占用`,
        impact:`${app.name} 没有启动。`,
        evidence:[['占用进程',f.process],['管理归属','外部进程，不是 AppHost 启动的'],['占用开始',f.processSince],['检查阶段','启动前检查，没有启动任何服务']],
        primary:ACT.retry,repeatPrimary:ACT.edit('port'),
        secondary:[ACT.edit('port'),ACT.diagnostic],
        note:'AppHost 不会结束外部进程。可以在它自己的终端里停止它，或修改这个应用的端口。'
      };
    }
    case 'DEPENDENCY_MISSING':return {
      title:`缺少 Python 依赖 ${f.module}`,
      impact:`${app.name} 没有启动：${exited}。`,
      evidence:[['判断依据',`输出第 ${f.mark+1} 行，规则 ${f.rule}`],['提示',f.hint]],
      suggestion:{command:f.command,note:'在项目目录中运行。AppHost 只提供命令，不会替你安装依赖。'},
      primary:{label:`复制 ${f.command}`,action:'copy-command',icon:'command'},repeatPrimary:ACT.details,
      secondary:[ACT.terminal,ACT.diagnostic]
    };
    case 'RUNTIME_MISMATCH':return {
      title:`Node 版本不满足项目要求`,
      impact:`${app.name} 没有启动：${exited}。`,
      evidence:[['项目要求',f.required],['实际使用',f.actual],['判断依据','版本信息来自 package.json 与实际执行的 node']],
      primary:ACT.environment,
      secondary:[ACT.edit('env'),ACT.diagnostic],
      note:'AppHost 不会自动切换或安装运行时。可以在运行环境中重新采集 PATH，或在声明里为这个服务指定 env.PATH。'
    };
    case 'INVALID_COMMAND':return {
      title:`找不到 npm 脚本 “${f.script}”`,
      impact:`${app.name} 没有启动：${exited}。`,
      evidence:[['执行的命令',f.command],['可用脚本',f.scripts],['判断依据',`输出第 ${f.mark+1} 行`]],
      primary:ACT.edit('command'),
      secondary:[ACT.diagnostic]
    };
    case 'READINESS_TIMEOUT':return {
      title:`${f.timeoutS} 秒内端口 ${f.port} 没有就绪`,
      impact:`${service} 仍在运行，但不在声明的端口上；应用入口不可用。`,
      evidence:[['声明的端口',`:${f.port}`],['实际监听',`:${f.observed} · 属于本次启动的进程`],['就绪期限',`${f.timeoutS} 秒（startup_timeout_ms）`]],
      suggestion:{text:`把声明中的 port 改为 ${f.observed}，保存并确认后重试。实际监听的端口只作为建议，不会自动写回。`},
      primary:ACT.edit('port'),
      secondary:[ACT.restart,ACT.stop,ACT.diagnostic]
    };
    case 'EARLY_EXIT':return {
      title:`${f.service} 启动 ${f.after}后退出`,
      impact:f.live?'其余服务仍在运行，保留用于排查；应用不可用。':`${app.name} 没有启动。`,
      evidence:[['退出码',String(f.exitCode)],['判断依据','启动后短时间退出，没有匹配到更具体的原因'],['最后一条错误',`输出第 ${f.mark+1} 行`]],
      primary:f.live?ACT.restart:ACT.retry,repeatPrimary:ACT.diagnostic,
      secondary:[ACT.edit('env'),...(f.live?[ACT.stop]:[]),ACT.diagnostic],
      note:'复制诊断信息时会附上这段输出，可以直接交给 agent 排查。'
    };
    default:return {
      title:`${f.service||'服务'} 运行中退出（exit ${f.exitCode}）`,
      impact:f.live?'其余服务仍在运行，保留用于排查。':'应用已停止。',
      evidence:[['判断依据','没有匹配的规则，归为未知原因'],['退出码',String(f.exitCode)]],
      primary:f.live?ACT.restart:ACT.retry,
      secondary:[...(f.live?[ACT.stop]:[]),ACT.diagnostic]
    };
  }
}
// Repeating the same failure stops offering retry as the main step (10.4).
function mainAction(x,f){return f.count>1&&x.repeatPrimary?x.repeatPrimary:x.primary;}
// A prompt disappears once its condition clears (5.9 clears_when); the record stays.
function clearedCondition(app){
  const f=app.failure;if(!f||app.state!=='FAILED'||f.code!=='PORT_IN_USE')return null;
  if(f.holderId){const holder=getApp(f.holderId);return holder&&!hasLive(holder)?`${holder.name} 已停止，端口 ${f.port} 已空出`:null;}
  return portBusy?null:`端口 ${f.port} 已释放`;
}

/* ── Sample scenarios: what the simulated daemon observes on start ── */
const startupTimeoutOf=app=>Number(declaration(app).match(/startup_timeout_ms\s*=\s*(\d+)/)?.[1]||30000);
function portHolder(app){
  // Preflight against other live installations first: that conflict has a managed remedy.
  const ports=new Set(app.services.map(s=>s.port).filter(Boolean));
  return apps.find(other=>other!==app&&hasLive(other)&&other.services.some(s=>ports.has(s.port)&&['running','starting'].includes(s.state)));
}
function simulateStart(app){
  const text=declaration(app),holder=portHolder(app);
  if(holder)return {delay:450,failure:{code:'PORT_IN_USE',phase:'preflight',service:app.services.find(s=>holder.services.some(h=>h.port===s.port))?.name,port:app.services.find(s=>holder.services.some(h=>h.port===s.port))?.port,holderId:holder.id}};
  switch(app.sim){
    case 'port-external':if(portBusy&&app.port===5173)return {delay:450,failure:{code:'PORT_IN_USE',phase:'preflight',service:'frontend',port:5173,process:'node · PID 18273',processSince:'今天 09:12'}};break;
    case 'dependency':return {delay:1200,failure:{code:'DEPENDENCY_MISSING',phase:'startup',service:'web',after:'0.9 秒',exitCode:1,module:'gradio',command:'uv sync',rule:'python.module_not_found',hint:'项目目录中没有 .venv',mark:3,
      tail:['Traceback (most recent call last):','  File "app.py", line 3, in <module>','    import gradio as gr',"ModuleNotFoundError: No module named 'gradio'"]}};
    case 'runtime':if(!preferences.pathSaved)return {delay:900,failure:{code:'RUNTIME_MISMATCH',phase:'startup',service:'web',after:'0.4 秒',exitCode:1,required:'node >= 20（package.json 的 engines）',actual:'v16.20.2 · /usr/local/bin/node',mark:2,
      tail:['file:///Users/me/Tools/ledger/node_modules/vite/bin/vite.js:7','    await import(\'source-map-support\').then((r) => r.default.install())','SyntaxError: Unexpected reserved word','Node.js v16.20.2']}};break;
    case 'command':if(text.includes('"serve"'))return {delay:700,failure:{code:'INVALID_COMMAND',phase:'startup',service:'timer',after:'0.3 秒',exitCode:1,script:'serve',command:'npm run serve',scripts:'dev · build · preview',mark:0,
      tail:['npm error Missing script: "serve"','npm error','npm error To see a list of scripts, run:','npm error   npm run']}};break;
    case 'timeout':if(!/port\s*=\s*8082/.test(text)){const ms=startupTimeoutOf(app);return {delay:ms,failure:{code:'READINESS_TIMEOUT',phase:'readiness',service:'web',port:8081,observed:8082,timeoutS:ms/1000,live:true,mark:4,
      tail:['> atlas-control@0.3.0 dev','> vite --port 8082','','  VITE v5.4.2  ready in 612 ms','  ➜  Local:   http://localhost:8082/']}};}break;
    case 'early-exit':if(!text.includes('DATABASE_URL'))return {delay:1600,failure:{code:'EARLY_EXIT',phase:'startup',service:'backend',after:'1.2 秒',exitCode:1,live:true,mark:5,
      tail:['INFO:     Started server process [48121]','INFO:     Waiting for application startup.','Traceback (most recent call last):','  File "app/db.py", line 8, in <module>','    engine = create_engine(os.environ["DATABASE_URL"])',"KeyError: 'DATABASE_URL'"]}};break;
    case 'slow':return {delay:8000};
  }
  return {delay:1400};
}
// Service states after a failure: preflight spawns nothing; later failures keep the
// survivors alive (5.2), and a readiness timeout leaves the service alive but not ready.
function applyFailure(app,f){
  for(const s of app.services){
    if(s.name===f.service){
      s.state=f.code==='READINESS_TIMEOUT'?'starting':'failed';
      if(f.exitCode!==undefined&&f.code!=='READINESS_TIMEOUT')s.exitCode=f.exitCode;
      s.note=f.code==='READINESS_TIMEOUT'?'进程存活，端口未就绪':undefined;
    }else s.state=f.phase==='preflight'||!f.live?'stopped':'running';
  }
  app.state='FAILED';
}
function recordFailure(app,f){
  const prev=app.failure;
  f.count=prev&&prev.code===f.code&&prev.service===f.service?prev.count+1:1;
  f.at=f.at||Date.now();
  app.failure=f;app.failures=[f,...(app.failures||[])].slice(0,20);
  applyFailure(app,f);
}
const clock=at=>new Date(at).toLocaleTimeString('zh-CN',{hour12:false,hour:'2-digit',minute:'2-digit'});
