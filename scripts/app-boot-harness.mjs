import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import vm from 'node:vm';
import ts from 'typescript';
const {JSDOM}=await import(process.env.PANE_JSDOM_MODULE ?? 'jsdom');
const repo=fileURLToPath(new URL('..',import.meta.url));
const clone=value=>JSON.parse(JSON.stringify(value));
const sourceNames=['claude','codex','opencode','pi','stepcode','grok','devin','minimax','hermes','kimi','qwen'];
export const settle=async()=>{for(let n=0;n<15;n++)await new Promise(resolve=>setImmediate(resolve));};
export async function boot({failModes=false,failCatalogInitially=false,initialSources={},initialCatalogDelay,initialModeDelay,initialUsageDelay,initialConfig={},snapshots=[],spendRows=[],initialConfigDelay,initialCachedDelay,initialSpendDelay,preserveCursor=false}={}) {
 const dom=new JSDOM(readFileSync(path.join(repo,'index.html'),'utf8'),{runScripts:'outside-only',url:'https://synthetic.invalid/',pretendToBeVisual:true});
 await settle();
 const w=dom.window;const context=dom.getInternalVMContext();const calls=[];const errors=[];let revision=1;let failSave=false;let delayFamily;let delayKey;let delayScan;let failCatalog=failCatalogInitially;let nextCatalogDelay=initialCatalogDelay;let delayMode;let nextUsageDelay=initialUsageDelay;let nextSpendDelay=initialSpendDelay;let usageSnapshots=clone(snapshots);let spendSnapshot=clone(spendRows);let keepCursor=preserveCursor;const events=new Map();const intervals=new Map();const timeouts=new Map();let timerId=0;let timeOffset=0;const now=w.Date.now.bind(w.Date);w.Date.now=()=>now()+timeOffset;
 w.addEventListener('error',event=>{errors.push(event.error);event.preventDefault();});
 w.matchMedia=()=>({matches:false,addEventListener(){},removeEventListener(){}});
 w.ResizeObserver=class {observe(){} disconnect(){}};
 const observers=[];const Observer=w.MutationObserver;
 w.MutationObserver=class extends Observer {constructor(callback){super(callback);observers.push(this);}};
 w.setInterval=(callback,ms)=>{const id=++timerId;intervals.set(id,{callback,ms});return id;};w.clearInterval=id=>intervals.delete(id);
 w.setTimeout=(callback,ms)=>{const id=++timerId;timeouts.set(id,{callback,ms});return id;};w.clearTimeout=id=>timeouts.delete(id);
 w.requestAnimationFrame=()=>0;w.cancelAnimationFrame=()=>{};
 // No canvas draw/image decode is used by these settings flows. Timers and
 // animation frames do not run autonomously; tests trigger captured timers explicitly.
 w.HTMLCanvasElement.prototype.getContext=()=>null; // visual effects are explicitly outside this DOM contract test
 w.__BUILD_STAMP__='synthetic';
 const main=readFileSync(path.join(repo,'src/main.ts'),'utf8');const ast=ts.createSourceFile('main.ts',main,ts.ScriptTarget.Latest,true);
 const decl=ast.statements.find(n=>ts.isVariableStatement(n)&&n.declarationList.declarations.some(d=>d.name.getText(ast)==='config'));
 const defaults=vm.runInContext(ts.transpileModule(decl.getText(ast).replace('let config: Config =','globalThis.defaults ='),{compilerOptions:{target:ts.ScriptTarget.ES2020}}).outputText,context);
 let backend=clone(w.defaults);delete w.defaults;
 const providerDecl=ast.statements.find(n=>ts.isVariableStatement(n)&&n.declarationList.declarations.some(d=>d.name.getText(ast)==='ALL_PROVIDERS'));
 const providerIds=new Function(ts.transpileModule(providerDecl.getText(ast),{compilerOptions:{target:ts.ScriptTarget.ES2020}}).outputText+';return ALL_PROVIDERS.map(([id])=>id)')().filter(id=>id!=='hermes');
 backend.providerAutoRefresh=Object.fromEntries(providerIds.map(id=>[id,!['claude','commandcode'].includes(id)]));
 Object.assign(backend,{locale:'en',glassEffects:false,reduceAnimations:true,welcomeDismissed:true,lastSeenVersion:'0.4.57',firstSeenMs:1},clone(initialConfig));
 backend.accessPolicy.scanSources=clone(initialSources);
 for(const [source,intent] of Object.entries(initialSources))if(intent.enabled)backend.accessPolicy.scanRoots[source]=intent.mode==='custom'?intent.customDirectories:intent.defaultDirectories;
 const configWrites=[];let failConfigRead=false;
 const accessReplies=new Map();
 const replyAccess=async(command)=>{const delay=accessReplies.get(command)?.shift();if(delay)await delay;return clone(backend);};
 const status=()=>sourceNames.map(source=>{
  const saved=backend.accessPolicy.scanSources[source]??{enabled:false,mode:'default',customDirectories:[],defaultDirectories:[`/synthetic/${source}/logs`]};
  return {source,...clone(saved),directories:(saved.mode==='custom'?saved.customDirectories:saved.defaultDirectories).map(p=>({path:p,status:p.includes('missing')?'notFound':'found'}))};
 });
 const invoke=async(command,args={})=>{
  calls.push({command,args:clone(args)});
  if(command==='get_config'){if(failConfigRead){failConfigRead=false;throw new Error('Synthetic config read failure');}if(initialConfigDelay)await initialConfigDelay;return clone(backend);}
  if(command==='get_scan_sources'){const snapshot=status();const delay=nextCatalogDelay;nextCatalogDelay=null;if(delay)await delay;if(failCatalog)throw new Error('Synthetic metadata IPC unavailable');return snapshot;}
  if(command==='get_provider_modes'){if(initialModeDelay)await initialModeDelay;if(failModes){failModes=false;throw new Error('synthetic mode failure');}return [{id:'qwen',family:'qwen',choices:[{value:'china:bearer',label:'China'}],defaultSelection:null,allowLocalOrigin:false},{id:'ollama',family:'ollama',choices:[{value:'local:http://127.0.0.1:11434',label:'Local IPv4'}],defaultSelection:null,allowLocalOrigin:true}];}
  if(command==='system_ui_locale')return 'en';
  if(command==='get_autostart')return false;
  if(command==='pricing_status')return {last_success_ms:0,catalog_stamp:'builtin'};
  if(command==='fetch_usage'||command==='cached_usage'){const result={revision,snapshots:clone(usageSnapshots)};if(command==='fetch_usage'){const delay=nextUsageDelay;nextUsageDelay=null;if(delay)await delay;}else if(initialCachedDelay)await initialCachedDelay;return result;}
  if(command==='claude_redeem_credit'||command==='codex_redeem_credit')return {outcome:'success',message:'Synthetic credit redeemed'};
  if(command==='fetch_spend'){const result={revision,rows:clone(spendSnapshot),preserveCursor:keepCursor};const delay=nextSpendDelay;nextSpendDelay=null;if(delay)await delay;return result;}
  if(command==='set_config'){const write=configWrites[0]?.autoOnly&&!args.patch.providerAutoRefresh?undefined:configWrites.shift();if(write?.delay)await write.delay;if(!write?.fail||write.commit){const patch={...args.patch};if(patch.providerAutoRefresh&&typeof patch.providerAutoRefresh==='object'&&!Array.isArray(patch.providerAutoRefresh))patch.providerAutoRefresh={...backend.providerAutoRefresh,...patch.providerAutoRefresh};Object.assign(backend,patch);}if(write?.fail)throw new Error('Synthetic config write failure');return clone(backend);}
  if(command==='set_provider_family'){
   if(delayFamily)await delayFamily;
   backend.accessPolicy.enabledFamilies=backend.accessPolicy.enabledFamilies.filter(id=>id!==args.family);
   if(args.enabled)backend.accessPolicy.enabledFamilies.push(args.family);
   else backend.accessPolicy.enabledAccounts=backend.accessPolicy.enabledAccounts.filter(id=>id!==args.family);
   revision++;return replyAccess(command);
  }
  if(command==='set_provider_account'){backend.accessPolicy.enabledAccounts=backend.accessPolicy.enabledAccounts.filter(id=>id!==args.account);if(args.enabled)backend.accessPolicy.enabledAccounts.push(args.account);revision++;return replyAccess(command);}
  if(command==='set_provider_region'){if(delayMode)await delayMode;backend.accessPolicy.regions[args.id]=args.region;revision++;return replyAccess(command);}
  if(command==='configure_scan_source'){
   if(delayScan)await delayScan;
   if(failSave){failSave=false;throw new Error('Log directory is unavailable');}
   const prior=backend.accessPolicy.scanSources[args.source]??{enabled:false,mode:'default',customDirectories:[],defaultDirectories:[`/synthetic/${args.source}/logs`]};
   const next={...prior,enabled:args.enabled,mode:args.mode,customDirectories:args.directories??prior.customDirectories};
   backend.accessPolicy.scanSources[args.source]=next;
   if(next.enabled)backend.accessPolicy.scanRoots[args.source]=next.mode==='custom'?next.customDirectories:next.defaultDirectories;
   else delete backend.accessPolicy.scanRoots[args.source];
   revision++;return replyAccess(command);
  }
  if(command==='discover_provider_account'){backend.accessPolicy.accountBindings[`${args.family}@identified`]={family:args.family,directory:args.directory,name:'Synthetic account'};revision++;return replyAccess(command);}
  if(command==='set_api_key'){if(delayKey)await delayKey;return null;}
  if(command==='reset_provider_access'){backend.accessPolicy={version:1,enabledAccounts:[],enabledFamilies:[],regions:{},accountBindings:{},scanRoots:{},scanSources:{}};revision++;return replyAccess(command);}
  if(['sync_tray_surfaces','widget_apply','set_api_key','set_autostart','set_shortcut','hide_popover'].includes(command))return null;
  throw new Error(`Unexpected IPC ${command}`);
 };
 const cache=new Map();
 function load(file){
  if(cache.has(file))return cache.get(file).exports;
  const module={exports:{}};cache.set(file,module);
  const require=specifier=>{
   if(specifier==='@tauri-apps/api/core')return {invoke};
   if(specifier==='@tauri-apps/api/event')return {listen:async(name,callback)=>{events.set(name,callback);return ()=>events.delete(name);}};
   if(specifier==='@tauri-apps/api/app')return {getVersion:async()=>'0.4.57'};
   if(specifier==='@tauri-apps/plugin-opener')return {openUrl:async()=>{throw new Error('External URL access forbidden');}};
   if(specifier.endsWith('?raw'))return readFileSync(path.resolve(path.dirname(file),specifier.slice(0,-4)),'utf8');
   if(specifier.endsWith('?inline'))return 'data:image/png;base64,AA==';
   if(specifier.startsWith('.'))return load(path.resolve(path.dirname(file),specifier+'.ts'));
   throw new Error(`Unexpected import ${specifier}`);
  };
  const js=ts.transpileModule(readFileSync(file,'utf8'),{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.CommonJS}}).outputText;
  vm.runInContext(`(function(require,module,exports){${js}\n})`,context,{filename:file})(require,module,module.exports);return module.exports;
 }
 load(path.join(repo,'src/main.ts'));
 w.dispatchEvent(new w.Event('DOMContentLoaded'));
 await settle();
 const select=(selector)=>{const result=w.document.querySelector(selector);assert.ok(result,`Required DOM selector ${selector}`);return result;};
 const change=async(selector,value)=>{const el=select(selector);if(typeof value==='boolean')el.checked=value;else el.value=value;el.dispatchEvent(new w.Event('change',{bubbles:true}));await settle();};
 const click=async(selector)=>{select(selector).click();await settle();};
 return {w,calls,errors,select,change,click,
  configWrite:options=>configWrites.push(options),failConfigRead:()=>{failConfigRead=true;},
  delayNextAccessReply:(command,promise)=>{const queue=accessReplies.get(command)??[];queue.push(promise);accessReplies.set(command,queue);},
  usage:()=>calls.filter(call=>call.command==='fetch_usage'),
  revision:value=>{revision=value;},delayNextSpend:promise=>{nextSpendDelay=promise;},
  spend:(rows,preserve=false)=>{spendSnapshot=clone(rows);keepCursor=preserve;},
  delayNextUsage:promise=>{nextUsageDelay=promise;},snapshots:value=>{usageSnapshots=clone(value);},advance:ms=>{timeOffset+=ms;},
  emit:async name=>{assert.ok(events.has(name),`Registered event ${name}`);events.get(name)({});await settle();},
  tick:async ms=>{const timer=[...intervals.values()].find(timer=>timer.ms===ms);assert.ok(timer,`Interval ${ms}`);timer.callback();await settle();},
  timeout:async ms=>{const entry=[...timeouts].find(([,timer])=>typeof ms==='function'?ms(timer.ms):Math.abs(timer.ms-ms)<100);assert.ok(entry,`Timeout near ${ms}; present: ${[...timeouts.values()].map(t=>t.ms)}`);timeouts.delete(entry[0]);entry[1].callback();await settle();},
  key:async()=>{const event=new w.KeyboardEvent('keydown',{key:'r',ctrlKey:true,bubbles:true,cancelable:true});w.dispatchEvent(event);await settle();assert.equal(event.defaultPrevented,true);},
  backend:()=>clone(backend),failSave:()=>{failSave=true;},delayFamily:promise=>{delayFamily=promise;},delayKey:promise=>{delayKey=promise;},delayMode:promise=>{delayMode=promise;},delayScan:promise=>{delayScan=promise;},failCatalog:value=>{failCatalog=value;},delayNextCatalog:promise=>{nextCatalogDelay=promise;},close:()=>{for(const observer of observers)observer.disconnect();w.close();}};
}

