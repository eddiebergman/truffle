'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/settings.js','utf8');
test('keyboard chords match saved command syntax across modifiers',()=>{const c=vm.createContext({});vm.runInContext(source.slice(source.indexOf('function chordFromEvent('),source.indexOf('function recordedShortcut(')),c);assert.equal(c.chordFromEvent({metaKey:true,key:'k'}),'Primary+K');assert.equal(c.chordFromEvent({ctrlKey:true,shiftKey:true,key:'Enter'}),'Primary+Shift+Enter');assert.equal(c.chordFromEvent({metaKey:true,altKey:true,key:'l'}),'Primary+Alt+L');assert.equal(c.chordFromEvent({key:'x'}),'');assert.equal(c.chordFromEvent({metaKey:true,key:'k',isComposing:true}),'');});
test('configured navigation changes the view and clears provider scopes consistently',()=>{let renders=0,saved=0;const c=vm.createContext({$:()=>({open:false}),settingsDirty:false,settingsSaving:false,document:{querySelector:()=>null},source:'agents',scopePrefs:{linear:'old'},githubView:'mine',persist:()=>saved++,render:()=>renders++});vm.runInContext(source.slice(source.indexOf('function runConfiguredCommand('),source.indexOf("document.addEventListener('keydown'",source.indexOf('function runConfiguredCommand('))),c);c.runConfiguredCommand('view-linear');assert.equal(c.source,'linear');assert.equal(c.scopePrefs.linear,'');c.runConfiguredCommand('view-github');assert.equal(c.githubView,'all');assert.equal(renders,2);assert.equal(saved,2);});
test('unsaved preferences block configured commands without discarding edits',()=>{let opens=0;const error={textContent:''},c=vm.createContext({$:id=>id==='#settings-dialog'?{open:true}:error,settingsDirty:true,settingsSaving:false,openCommands:()=>opens++});vm.runInContext(source.slice(source.indexOf('function runConfiguredCommand('),source.indexOf("document.addEventListener('keydown'",source.indexOf('function runConfiguredCommand('))),c);c.runConfiguredCommand('commands');assert.equal(opens,0);assert.match(error.textContent,/Save your changes/);});

test('shortcut recorder accepts physical punctuation and rejects reserved or unsupported input',()=>{const c=vm.createContext({});vm.runInContext(source.slice(source.indexOf('function chordFromEvent('),source.indexOf('async function finishShortcutRecording(')),c);assert.equal(c.recordedShortcut({metaKey:true,shiftKey:true,key:'?',code:'Slash'}),'Primary+Shift+/');assert.equal(c.recordedShortcut({key:'Shift',shiftKey:true}),'');assert.equal(c.recordedShortcut({metaKey:true,altKey:true,key:'˚',code:'KeyK'}),'Primary+Alt+K');assert.throws(()=>c.recordedShortcut({metaKey:true,key:'q'}),/reserved/);assert.throws(()=>c.recordedShortcut({key:'a'}),/Command/);assert.throws(()=>c.recordedShortcut({metaKey:true,key:'F5'}),/Command/);});

test('typed shortcuts normalize modifiers and expose reserved and duplicate assignments',()=>{
 const c=vm.createContext({settingsDraft:{keybindings:{commands:'Primary+K',search:'Primary+F'}},appSettings:{commands:[{id:'commands',label:'Command menu'},{id:'search',label:'Search'}]}});
 vm.runInContext(source.slice(source.indexOf('function canonicalShortcut('),source.indexOf('function recordedShortcut(')),c);
 assert.equal(c.canonicalShortcut(' primary+alt+shift+k '),'Primary+Shift+Alt+K');
 assert.equal(c.canonicalShortcut(''), '');
 for(const value of ['Primary+Primary+K','Primary+Shift+Shift+K','Control+K','Primary+F5','Primary++'])assert.equal(c.canonicalShortcut(value),null,value);
 assert.match(c.shortcutValidationMessage('primary+k','search'),/Command menu/);
 assert.equal(c.shortcutValidationMessage('Primary+K','commands'),'');
 assert.equal(c.shortcutValidationMessage('','search'),'');
 assert.match(c.shortcutValidationMessage('primary+q','commands'),/reserved/);
 assert.match(c.shortcutValidationMessage('Primary+F5','commands'),/Use Primary/);
});

function recorderContext({desktop=false,invoke=async()=>{}}={}){
 const listeners={},status={textContent:''},error={textContent:''};
 const c=vm.createContext({Promise,shortcutRecording:null,shortcutRecordingSync:Promise.resolve(),settingsDirty:false,settingsSaving:false,settingsDraft:{keybindings:{commands:'Primary+K',search:'Primary+F'}},appSettings:{commands:[{id:'commands',label:'Commands'},{id:'search',label:'Search'}]},serviceClient:{isDesktop:desktop},window:{__TAURI__:{core:{invoke}},addEventListener:(name,fn)=>listeners[name]=fn},$:id=>id==='#settings-error'?error:status,updateSettingsDirty:()=>{},filterSettings(){}});
 vm.runInContext(source.slice(source.indexOf('function chordFromEvent('),source.indexOf('function runConfiguredCommand(')),c);
 const button={textContent:'Record',attributes:{},setAttribute(name,value){this.attributes[name]=value;}},input={value:'Primary+K',readOnly:false,focus(){}};
 const key=(key,extra={})=>({key,preventDefault(){this.prevented=true;},stopImmediatePropagation(){this.stopped=true;},...extra});
 return {c,listeners,status,error,button,input,key};
}

test('recording keeps composition and modifier presses pending, reports conflicts, and allows retry',async()=>{
 const {c,listeners,error,button,input,key}=recorderContext();
 await c.startShortcutRecording(button,input,'commands');
 assert.equal(input.readOnly,true);assert.equal(button.attributes['aria-pressed'],'true');
 const composing=key('k',{isComposing:true});listeners.keydown(composing);assert.equal(composing.prevented,undefined);assert.equal(c.shortcutRecording.id,'commands');
 listeners.keydown(key('Shift',{shiftKey:true}));assert.equal(error.textContent,'');
 listeners.keydown(key('f',{metaKey:true}));assert.match(error.textContent,/Search/);assert.equal(input.value,'Primary+K');assert.ok(c.shortcutRecording);
 listeners.keydown(key('j',{metaKey:true}));assert.equal(input.value,'Primary+J');assert.equal(c.settingsDraft.keybindings.commands,'Primary+J');assert.equal(input.readOnly,false);assert.equal(c.shortcutRecording,null);assert.equal(button.attributes['aria-pressed'],'false');
});

test('Tab, Escape and repeated Record cancel without changing the saved draft',async()=>{
 for(const cancellation of ['Tab','Escape','button']){
  const {c,listeners,button,input,key}=recorderContext();await c.startShortcutRecording(button,input,'commands');
  if(cancellation==='button')await c.startShortcutRecording(button,input,'commands');else{const event=key(cancellation);listeners.keydown(event);assert.equal(Boolean(event.prevented),cancellation==='Escape');}
  assert.equal(input.readOnly,false);assert.equal(c.settingsDraft.keybindings.commands,'Primary+K');assert.equal(c.shortcutRecording,null);
 }
});

test('closing during native recording startup restores native shortcuts in order',async()=>{
 const calls=[];let resolveStart;
 const {c,button,input}=recorderContext({desktop:true,invoke:async(_name,args)=>{calls.push(args.recording);if(args.recording)await new Promise(resolve=>resolveStart=resolve);}});
 const start=c.startShortcutRecording(button,input,'commands');
 await new Promise(resolve=>setImmediate(resolve));
 const finish=c.finishShortcutRecording();assert.equal(input.readOnly,false);assert.equal(c.shortcutRecording,null);
 resolveStart();await Promise.all([start,finish]);assert.deepEqual(calls,[true,false]);
});

test('native recording failures restore editable input and report the failure',async()=>{
 const {c,error,button,input}=recorderContext({desktop:true,invoke:async(_name,args)=>{if(args.recording)throw new Error('Unavailable');}});
 await c.startShortcutRecording(button,input,'commands');assert.equal(input.readOnly,false);assert.equal(c.shortcutRecording,null);assert.match(error.textContent,/Could not record.*Unavailable/);
});


test('shortcut hints follow configured assignments and clear stale accessibility shortcuts',()=>{
 const attrs={'aria-keyshortcuts':'Meta+G'},hint={};let binding='Primary+Alt+G';
 const context=vm.createContext({navigator:{platform:'MacIntel'},setting:()=>({'view-github':binding}),shortcutLabel:chord=>chord.replace('Primary+','⌘').replace('Alt+','⌥')});
 const code=fs.readFileSync('static/settings.js','utf8');vm.runInContext(code.slice(code.indexOf('function applyShortcutHint('),code.indexOf('function applyAppSettings(')),context);
 const button={querySelector:()=>hint,setAttribute:(key,value)=>attrs[key]=value,removeAttribute:key=>delete attrs[key]};
 context.applyShortcutHint(button,'view-github');assert.equal(hint.textContent,'⌘⌥G');assert.equal(hint.hidden,false);assert.equal(attrs['aria-keyshortcuts'],'Meta+Alt+G');
 binding='';context.applyShortcutHint(button,'view-github');assert.equal(hint.hidden,true);assert.equal(hint.textContent,'');assert.equal(attrs['aria-keyshortcuts'],undefined);
});

function profileImportContext(profile){
 const draft={theme:'midnight',density:'compact',uiScale:115,keybindings:{commands:'Primary+J',search:'Primary+G'},quickActions:[{id:'current',title:'Current',command:'printf current'}]};
 const error={textContent:''};let paints=0,dirty=0;
 const c=vm.createContext({structuredClone,settingsSaving:false,appSettings:{revision:7,contentHash:'a'.repeat(64)},settingsBaseline:{revision:7,contentHash:'a'.repeat(64)},api:async(path,payload)=>{assert.equal(path,'/api/settings/preview');assert.equal(payload.revision,7);assert.equal(payload.contentHash,'a'.repeat(64));return {};},settingsDraft:structuredClone(draft),file:{files:[{size:100,text:async()=>typeof profile==='string'?profile:JSON.stringify(profile)}],value:'selected'},$:()=>error,paintSettingsBody:()=>paints++,updateSettingsDirty:()=>dirty++});
 vm.runInContext(source.slice(source.indexOf('function mergeSettingsProfile('),source.indexOf('function exportSettingsProfile(')),c);
 const start=source.indexOf('file.onchange=async()=>');
 vm.runInContext(source.slice(start,source.indexOf('oldDialog.append(file)',start)),c);
 return {c,draft,error,counts:()=>({paints,dirty})};
}
test('partial profile patches the unsaved draft and merges only supplied keyboard assignments',async()=>{
 const {c,draft,error,counts}=profileImportContext({format:'truffle-settings',version:1,values:{theme:'light',keybindings:{commands:'Primary+L'}}});
 await c.file.onchange();
 const result=JSON.parse(JSON.stringify(c.settingsDraft));
 assert.deepEqual(result,{...draft,theme:'light',keybindings:{...draft.keybindings,commands:'Primary+L'}});
 assert.deepEqual(counts(),{paints:1,dirty:1});assert.match(error.textContent,/nothing has been applied/);assert.equal(c.file.value,'');
});
test('profile without keybindings retains all current draft shortcuts and other unsaved edits',async()=>{
 const {c,draft}=profileImportContext({format:'truffle-settings',version:1,values:{density:'comfortable'}});await c.file.onchange();
 assert.deepEqual(JSON.parse(JSON.stringify(c.settingsDraft)),{...draft,density:'comfortable'});
});
test('malformed or unsupported profiles leave the complete draft untouched',async()=>{
 for(const profile of ['{broken',null,[],{format:'truffle-settings',version:2,values:{}},{format:'truffle-settings',version:1,values:[]},{format:'truffle-settings',version:1,values:{theme:'light',unknown:'unsupported'}},{format:'truffle-settings',version:1,values:{theme:'light'},extension:{}},{format:'truffle-settings',version:1,values:{keybindings:{unknown:'Primary+L'}}},{format:'truffle-settings',version:1,values:{keybindings:{commands:null}}},{format:'truffle-settings',version:1,values:{keybindings:[]}},{format:'truffle-settings',version:1,values:{uiScale:'big'}},{format:'truffle-settings',version:1,values:{quickActions:[null]}},{format:'truffle-settings',version:1,values:{quickActions:[{id:'x',title:'X',command:'printf x',unsupported:true}]}}]){
  const {c,draft,error,counts}=profileImportContext(profile),identity=c.settingsDraft;await c.file.onchange();
  assert.equal(c.settingsDraft,identity);assert.deepEqual(JSON.parse(JSON.stringify(c.settingsDraft)),draft);assert.deepEqual(counts(),{paints:0,dirty:0});assert.ok(error.textContent);assert.equal(c.file.value,'');
 }
});


test('backend preview validation and revision failures preserve the exact draft',async()=>{
 for(const patch of [{uiScale:-1},{density:'bad'},{quickActions:[{id:'',title:'',command:''}]}]){
  const {c,draft,error,counts}=profileImportContext({format:'truffle-settings',version:1,values:patch}),identity=c.settingsDraft;
  c.api=async()=>{throw new Error('Backend validation rejected the profile.');};await c.file.onchange();
  assert.equal(c.settingsDraft,identity);assert.deepEqual(JSON.parse(JSON.stringify(c.settingsDraft)),draft);assert.deepEqual(counts(),{paints:0,dirty:0});assert.match(error.textContent,/Backend validation/);
 }
 const {c,draft,error}=profileImportContext('{"format":"truffle-settings","version":1,"values":{"uiScale":1e999}}'),identity=c.settingsDraft;
 c.api=async(_path,payload)=>{assert.equal(JSON.parse(JSON.stringify(payload)).patch.uiScale,null);throw new Error('Invalid numeric setting.');};await c.file.onchange();
 assert.equal(c.settingsDraft,identity);assert.deepEqual(JSON.parse(JSON.stringify(c.settingsDraft)),draft);assert.match(error.textContent,/Invalid numeric/);
 c.file.files[0].text=async()=>JSON.stringify({format:'truffle-settings',version:1,values:{uiScale:120}});
 c.api=async()=>{throw new Error('Settings changed elsewhere. Reload settings before saving.');};await c.file.onchange();assert.equal(c.settingsDraft,identity);assert.match(error.textContent,/changed elsewhere/);
});
test('edits made while preview is pending are retained and the imported draft is discarded',async()=>{
 const {c,error}=profileImportContext({format:'truffle-settings',version:1,values:{theme:'light'}});
 let finish;c.api=()=>new Promise(resolve=>finish=resolve);const importing=c.file.onchange();await new Promise(resolve=>setImmediate(resolve));
 c.settingsDraft.uiScale=125;finish({});await importing;assert.equal(c.settingsDraft.uiScale,125);assert.equal(c.settingsDraft.theme,'midnight');assert.match(error.textContent,/changed while validating/);
});

test('saving a draft uses its captured identity, preserves conflicts and serializes repeated clicks',async()=>{
 const nodes=new Map(),calls=[];let release;const gate=new Promise(resolve=>release=resolve),baseline={revision:7,contentHash:'a'.repeat(64),values:{theme:'light',keybindings:{}}};
 const c=vm.createContext({structuredClone,settingsSaving:false,settingsDirty:false,settingsBaseline:null,settingsDraft:null,appSettings:baseline,document:{activeElement:null,body:{}},$:id=>{if(!nodes.has(id))nodes.set(id,{textContent:''});return nodes.get(id);},shortcutValidationMessage:()=>'',canonicalShortcut:value=>value,finishShortcutRecording:()=>gate,updateSettingsDirty(){},applyAppSettings(){assert.fail('Conflict must not apply');},paintSettingsBody(){assert.fail('Conflict must not repaint');},api:async(path,payload)=>{calls.push(payload);throw new Error('Settings changed elsewhere. Reload before saving.');}});
 vm.runInContext(source.slice(source.indexOf('function resetSettingsDraft('),source.indexOf('function mergeSettingsProfile(')),c);c.resetSettingsDraft(baseline);c.settingsDraft.theme='midnight';c.appSettings={...baseline,revision:9,contentHash:'b'.repeat(64)};
 const first=c.savePreferences();await c.savePreferences();release();await first;assert.equal(calls.length,1);assert.equal(calls[0].revision,7);assert.equal(calls[0].contentHash,'a'.repeat(64));assert.equal(c.settingsDraft.theme,'midnight');assert.match(nodes.get('#settings-error').textContent,/changed elsewhere/);assert.equal(c.settingsSaving,false);
});
test('quick-action execution keeps the identity captured when its command was displayed',async()=>{
 const calls=[],c=vm.createContext({currentAgentRecord:()=>null,appSettings:{revision:9,contentHash:'b'.repeat(64)},api:async(path,payload)=>{calls.push(payload);throw new Error('Command changed. Review it again.');},toast:message=>calls.push(message),openActionOutput(){assert.fail('A changed command must not run');}});
 vm.runInContext(source.slice(source.indexOf('async function chooseQuickAction('),source.indexOf('async function loadQuickActionRuns(')),c);
 await c.chooseQuickAction({id:'test',command:'printf fixture'},{revision:7,contentHash:'a'.repeat(64)});assert.equal(calls[0].args.revision,7);assert.equal(calls[0].args.contentHash,'a'.repeat(64));assert.match(calls[1],/Review it again/);
});

function deferredSave(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
function settingsSaveFixture(){
 const calls=[],body={},nodes=[],footer=new Map(),document={body,activeElement:body};
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.value='';this.isConnected=true;nodes.push(this);this.classList={add:()=>{}};}
  append(...children){this.children.push(...children);for(const child of children)child.parentElement=this;}
  contains(node){return this===node||this.children.some(child=>child.contains?.(node));}
  closest(selector){for(let node=this;node;node=node.parentElement)if(selector==='[hidden]'&&node.hidden)return node;return null;}
  setAttribute(key,value){this.attrs[key]=value;}
  getAttribute(key){return this.attrs[key]||null;}
  removeAttribute(key){delete this.attrs[key];}
  add(option){this.children.push(option);}
  addEventListener(type,callback){(this.listeners??={})[type]=callback;}
  querySelectorAll(selector){return this.children.flatMap(child=>[child,...(child.querySelectorAll?.('*')||[])]).filter(node=>selector==='*'||selector==='.preference-row'&&node.className.split(' ').includes('preference-row'));}
  setCustomValidity(value){this.validationMessage=value;}
  blur(){if(document.activeElement===this){document.activeElement=body;for(let node=this;node;node=node.parentElement)node.listeners?.focusout?.();}}
  focus(options){const previous=document.activeElement;document.activeElement=this;this.focusOptions=options;if(previous!==this)for(let node=previous;node;node=node.parentElement)node.listeners?.focusout?.();}
  set disabled(value){this.frozen=value;if(value&&document.activeElement===this)document.activeElement=body;}
  get disabled(){return !!this.frozen;}
 }
 document.querySelectorAll=selector=>nodes.filter(node=>node.isConnected&&(selector==='#settings-dialog [data-settings-draft-control]'?node.dataset.settingsDraftControl:selector==='.shortcut-input'?node.className.includes('shortcut-input'):selector==='[data-settings-reset]'?node.dataset.settingsReset:false));
 document.getElementById=id=>nodes.find(node=>node.isConnected&&node.id===id)||null;
 const dialog=new Node('dialog');dialog.open=true;footer.set('#settings-dialog',dialog);
 for(const id of ['settings-error','settings-status','settings-save']){const node=new Node('button');node.id=id;footer.set('#'+id,node);}
 const values={theme:'light',keybindings:{commands:'Primary+K'},quickActions:[{id:'one',title:'One',command:'printf one'}]},baseline={revision:7,contentHash:'a'.repeat(64),values};let apiHandler=async()=>baseline,loadHandler=async()=>baseline;
 const c=vm.createContext({structuredClone,queueMicrotask,document,settingsSaving:false,settingsDirty:false,settingsDraft:null,settingsBaseline:null,appSettings:{...baseline,commands:[{id:'commands',label:'Commands'}],defaults:structuredClone(values),fields:[]},shortcutRecording:null,$:id=>footer.get(id),el:(...args)=>new Node(...args),settingsButton:(label,run,cls)=>Object.assign(new Node('button',cls,label),{onclick:run}),Option:function(text,value){this.text=text;this.value=value;},shortcutValidationMessage:()=>'',canonicalShortcut:value=>value,finishShortcutRecording:async()=>{},filterSettings(){},applyAppSettings(){},api:async(path,payload)=>{calls.push({path,payload});return apiHandler(path,payload);},loadAppSettings:async(options)=>{const next=await loadHandler(options);return !options||options.canApply()?next:null;},paintSettingsBody:()=>draw()});
 vm.runInContext(source.slice(source.indexOf('function settingsReset('),source.indexOf('function paintSettingsBody(')),c);
 vm.runInContext(source.slice(source.indexOf('function resetSettingsDraft('),source.indexOf('function mergeSettingsProfile(')),c);
 vm.runInContext(source.slice(source.indexOf('function mergeSettingsProfile('),source.indexOf('function exportSettingsProfile(')),c);
 const quick=source.indexOf(' for(const action of settingsDraft.quickActions){');vm.runInContext('function drawQuick(panel){'+source.slice(quick,source.indexOf("const output=el('div');output.id='quick-action-runs'",quick))+'}',c);
 c.resetSettingsDraft(baseline);let routine,shortcut,quickPanel;
 function draw(){for(const node of nodes)if(node.draftNode)node.isConnected=false;const start=nodes.length;routine=c.settingInput(['appearance','theme','Theme','text','']);shortcut=c.shortcutInput({id:'commands',label:'Commands'});quickPanel=new Node('section');c.drawQuick(quickPanel);for(const node of nodes.slice(start))node.draftNode=true;}
 draw();const file=new Node('input');file.dataset.settingsDraftControl='true';file.files=[];c.file=file;const importStart=source.indexOf('file.onchange=async()=>');vm.runInContext(source.slice(importStart,source.indexOf('oldDialog.append(file)',importStart)),c);
 const independent=new Node('input');independent.id='sharing-independent';
 const all=node=>[node,...node.children.flatMap(child=>child instanceof Node?all(child):[])];
 return {c,calls,document,dialog,independent,file,Node,nodes,node:id=>document.getElementById(id),error:()=>footer.get('#settings-error').textContent,api(handler){apiHandler=handler;},load(handler){loadHandler=handler;},quickInputs:()=>all(quickPanel).filter(node=>['INPUT','TEXTAREA'].includes(node.tagName)),quickButton:label=>all(quickPanel).find(node=>node.textContent===label),edit(value){const input=document.getElementById('preference-theme');input.value=value;input.oninput();},reset:()=>all(routine).find(node=>node.dataset.settingsReset),draw};
}

test('deferred Settings Save freezes only its own draft controls and guards all edit handlers',{timeout:1000},async()=>{
 const f=settingsSaveFixture();f.edit('midnight');const pending=deferredSave();f.api(()=>pending.promise);const saving=f.c.savePreferences();await new Promise(resolve=>setImmediate(resolve));
 for(const node of [f.node('preference-theme'),f.node('shortcut-commands'),...f.quickInputs(),f.quickButton('Add quick action'),f.file])assert.equal(node.disabled,true);
 assert.equal(f.independent.disabled,false);
 f.edit('later');const shortcut=f.node('shortcut-commands');shortcut.value='Primary+J';shortcut.oninput();f.quickInputs()[0].value='Later action';f.quickInputs()[0].oninput();f.quickButton('Add quick action').onclick();f.quickButton('Remove').onclick();f.reset().onclick();
 f.file.files=[{size:100,text:async()=>JSON.stringify({format:'truffle-settings',version:1,values:{theme:'imported'}})}];await f.file.onchange();await f.c.discardSettingsDraft();
 assert.equal(f.c.settingsDraft.theme,'midnight');assert.equal(f.c.settingsDraft.keybindings.commands,'Primary+K');assert.equal(f.c.settingsDraft.quickActions.length,1);assert.equal(f.c.settingsDraft.quickActions[0].title,'One');assert.equal(f.calls.length,1);
 pending.resolve({...f.c.appSettings,revision:8,contentHash:'b'.repeat(64),values:structuredClone(f.calls[0].payload.patch)});await saving;
 assert.equal(f.node('preference-theme').disabled,false);assert.equal(f.file.disabled,false);assert.equal(f.c.settingsSaving,false);assert.equal(f.c.settingsDirty,false);
});

test('deferred Settings Save submits an immutable snapshot and retains any unexpected newer draft',{timeout:1000},async()=>{
 const f=settingsSaveFixture();f.edit('midnight');const pending=deferredSave();f.api(()=>pending.promise);const saving=f.c.savePreferences();await new Promise(resolve=>setImmediate(resolve));
 f.c.settingsDraft.theme='later';assert.equal(f.calls[0].payload.patch.theme,'midnight');assert.equal(f.calls[0].payload.revision,7);assert.equal(f.calls[0].payload.contentHash,'a'.repeat(64));
 pending.resolve({...f.c.appSettings,revision:8,contentHash:'b'.repeat(64),values:structuredClone(f.calls[0].payload.patch)});await saving;
 assert.equal(f.c.settingsDraft.theme,'later');assert.equal(f.c.settingsDirty,true);assert.equal(f.c.settingsBaseline.revision,8);assert.match(f.error(),/newer draft is still unsaved/);
});

test('deferred Settings Save failure preserves draft and CAS errors, reenables controls and restores only lost focus',{timeout:1000},async()=>{
 for(const moveFocus of [false,true]){
  const f=settingsSaveFixture();f.edit('midnight');f.node('settings-save').focus();const pending=deferredSave();f.api(()=>pending.promise);const saving=f.c.savePreferences();await new Promise(resolve=>setImmediate(resolve));if(moveFocus)f.independent.focus();
  pending.reject(new Error('Settings changed elsewhere. Reload before saving.'));await saving;
  assert.equal(f.c.settingsDraft.theme,'midnight');assert.equal(f.c.settingsBaseline.revision,7);assert.match(f.error(),/changed elsewhere/);assert.equal(f.node('preference-theme').disabled,false);assert.equal(f.file.disabled,false);assert.equal(f.c.settingsDirty,true);
  assert.equal(f.document.activeElement,moveFocus?f.independent:f.node('settings-save'));
 }
});

test('profile reading and reload started before Save cannot overwrite the submitted or later draft',{timeout:1000},async()=>{
 const f=settingsSaveFixture();f.edit('midnight');const reading=deferredSave();f.file.files=[{size:100,text:()=>reading.promise}];const importing=f.file.onchange();const pending=deferredSave();f.api(()=>pending.promise);const saving=f.c.savePreferences();await new Promise(resolve=>setImmediate(resolve));
 reading.resolve(JSON.stringify({format:'truffle-settings',version:1,values:{theme:'imported'}}));await importing;assert.equal(f.c.settingsDraft.theme,'midnight');assert.equal(f.calls.length,1);
 pending.reject(new Error('Save failed'));await saving;
 const reload=deferredSave();f.load(()=>reload.promise);const discarding=f.c.discardSettingsDraft();const request=deferredSave();f.api(()=>request.promise);const second=f.c.savePreferences();await new Promise(resolve=>setImmediate(resolve));reload.resolve({...f.c.appSettings,revision:99,values:{...f.c.settingsDraft,theme:'stale reload'}});await discarding;
 assert.equal(f.c.settingsBaseline.revision,7);assert.equal(f.c.settingsDraft.theme,'midnight');request.reject(new Error('Save failed'));await second;
});

test('Settings Save blocks shortcut capture and recording finalization never changes its draft',async()=>{
 const f=recorderContext();await f.c.startShortcutRecording(f.button,f.input,'commands');f.listeners.keydown(f.key('j',{metaKey:true}));assert.equal(f.c.settingsDraft.keybindings.commands,'Primary+J');
 await f.c.startShortcutRecording(f.button,f.input,'commands');const snapshot=JSON.stringify(f.c.settingsDraft);f.c.settingsSaving=true;
 f.listeners.keydown(f.key('l',{metaKey:true}));await f.c.startShortcutRecording(f.button,f.input,'search');assert.equal(JSON.stringify(f.c.settingsDraft),snapshot);assert.equal(f.c.shortcutRecording.id,'commands');
 await f.c.finishShortcutRecording();assert.equal(JSON.stringify(f.c.settingsDraft),snapshot);assert.equal(f.c.shortcutRecording,null);assert.equal(f.input.readOnly,false);
});


function settingsModifiedFixture(){
 const f=settingsSaveFixture(),originalQuery=f.document.querySelectorAll,originalDollar=f.c.$;f.c.settingsCategory='appearance';
 f.c.$=selector=>f.document.getElementById(selector.slice(1))||originalDollar(selector);
 f.document.querySelectorAll=selector=>selector==='.preferences-panel'?f.nodes.filter(node=>node.isConnected&&node.className==='preferences-panel'):selector==='[data-settings-category]'?f.nodes.filter(node=>node.isConnected&&node.dataset.settingsCategory):originalQuery(selector);
 vm.runInContext(source.slice(source.indexOf('function resetSettingsFilter('),source.indexOf('function resetSettingsDraft(')),f.c);
 const search=new f.Node('input');search.id='settings-search';const controls=f.c.settingsFilterControls(search),head=new f.Node('div');head.append(...controls);const modified=f.node('settings-modified');
 const operational=new f.Node('section','preferences-panel');operational.dataset.category='sharing';operational.append(f.independent);f.independent.value='Unsaved sharing draft';
 const draw=f.c.paintSettingsBody;
 function mount(){for(const [category,id]of [['appearance','preference-theme'],['keyboard','shortcut-commands']]){const panel=new f.Node('section','preferences-panel');panel.dataset.category=category;panel.draftNode=true;panel.append(f.node(id).parentElement.parentElement);}f.c.filterSettings();}
 f.c.paintSettingsBody=()=>{draw();mount();};mount();
 const flush=async()=>{await Promise.resolve();await Promise.resolve();};
 return {...f,search,modified,head,operational,flush,row:id=>f.node(id).parentElement.parentElement,toggle:()=>modified.onclick()};
}

test('Modified only uses actual field and shortcut defaults with text intersection and scoped zero matches',()=>{
 const f=settingsModifiedFixture();assert.equal(f.modified.textContent,'Modified only');assert.equal(f.modified.getAttribute('aria-pressed'),'false');f.toggle();assert.equal(f.modified.getAttribute('aria-pressed'),'true');assert.equal(f.row('preference-theme').hidden,true);assert.equal(f.row('shortcut-commands').hidden,true);assert.equal(f.node('settings-filter-status').hidden,false);assert.match(f.node('settings-filter-status').textContent,/No modified preferences or shortcuts match/);assert.equal(f.operational.hidden,true);
 f.edit('dark');assert.equal(f.row('preference-theme').dataset.settingsKey,'theme');assert.equal(f.row('preference-theme').hidden,false);const shortcut=f.node('shortcut-commands');shortcut.value='Primary+J';shortcut.oninput();assert.equal(f.row('shortcut-commands').dataset.settingsCommand,'commands');assert.equal(f.row('shortcut-commands').hidden,false);
 f.search.value='commands';f.c.filterSettings();assert.equal(f.row('preference-theme').hidden,true);assert.equal(f.row('shortcut-commands').hidden,false);f.search.value='nothing matches';f.c.filterSettings();assert.equal(f.node('settings-filter-status').hidden,false);assert.match(f.node('settings-filter-scope').textContent,/Quick actions, Connections and Sharing forms are not included/);
 f.c.resetSettingsFilter();f.c.filterSettings();assert.equal(f.search.value,'');assert.equal(f.modified.getAttribute('aria-pressed'),'false');assert.equal(f.row('preference-theme').hidden,false);assert.equal(f.row('shortcut-commands').hidden,false);assert.equal(f.independent.value,'Unsaved sharing draft');assert.equal(f.calls.length,0);
});

test('Modified filter retains active edits until blur and Reset hands owned focus to the stable toggle',async()=>{
 const f=settingsModifiedFixture();f.edit('dark');f.toggle();const input=f.node('preference-theme');input.focus();input.value='light';input.oninput();assert.equal(f.row(input.id).hidden,false);assert.equal(f.document.activeElement,input);f.search.focus();await f.flush();assert.equal(f.row(input.id).hidden,true);assert.equal(f.document.activeElement,f.search);
 f.edit('dark');const reset=f.reset();reset.focus();reset.onclick();assert.equal(f.row('preference-theme').hidden,true);assert.equal(f.document.activeElement,f.modified);assert.equal(f.modified.focusOptions.preventScroll,true);
 f.edit('dark');f.node('preference-theme').focus();f.reset().onclick();assert.equal(f.row('preference-theme').hidden,true);assert.equal(f.document.activeElement,f.modified,'Reset also releases focus when the browser keeps focus in the edited input');
 f.edit('dark');f.independent.focus();f.reset().onclick();assert.equal(f.document.activeElement,f.independent);assert.equal(f.independent.value,'Unsaved sharing draft');assert.equal(f.calls.length,0);
});

test('Modified filter preserves actual shortcut recording ownership until native recording ends',async()=>{
 const f=settingsModifiedFixture();f.c.syncNativeShortcutRecording=async()=>{};vm.runInContext(source.slice(source.indexOf('async function finishShortcutRecording('),source.indexOf("window.addEventListener('keydown'")),f.c);const input=f.node('shortcut-commands'),record=input.parentElement.children.find(node=>node.className.includes('shortcut-record'));input.value='Primary+J';input.oninput();f.toggle();await f.c.startShortcutRecording(record,input,'commands');assert.equal(f.c.shortcutRecording.input,input);input.value='Primary+K';input.oninput();assert.equal(f.row(input.id).hidden,false);
 f.independent.focus();await f.flush();assert.equal(f.row(input.id).hidden,false);await f.c.finishShortcutRecording();assert.equal(f.row(input.id).hidden,true);assert.equal(f.document.activeElement,f.independent);assert.equal(input.readOnly,false);assert.equal(f.independent.value,'Unsaved sharing draft');
});

test('Modified filtering recomputes through actual save, profile import and discard without writing filter state',async()=>{
 const f=settingsModifiedFixture();f.toggle();const profile={format:'truffle-settings',version:1,values:{theme:'dark'}};f.file.files=[{size:100,text:async()=>JSON.stringify(profile)}];await f.file.onchange();assert.equal(f.row('preference-theme').hidden,false);assert.equal(f.modified.getAttribute('aria-pressed'),'true');assert.equal(f.calls[0].path,'/api/settings/preview');
 f.api(async()=>({...f.c.appSettings,revision:8,contentHash:'b'.repeat(64),values:structuredClone(f.c.settingsDraft)}));await f.c.savePreferences();assert.equal(f.row('preference-theme').hidden,false,'saved custom values still differ from defaults');assert.equal(f.modified.getAttribute('aria-pressed'),'true');assert.equal(Object.hasOwn(f.calls.find(call=>call.path==='/api/settings').payload.patch,'modifiedOnly'),false);
 f.load(async()=>({revision:9,contentHash:'c'.repeat(64),values:structuredClone(f.c.appSettings.defaults)}));await f.c.discardSettingsDraft();assert.equal(f.row('preference-theme').hidden,true);assert.equal(f.row('shortcut-commands').hidden,true);assert.equal(f.node('settings-filter-status').hidden,false);assert.equal(f.independent.value,'Unsaved sharing draft');
});


test('Reset updates only its existing input and preserves independent forms without painting or disposal',async()=>{
 const f=settingsModifiedFixture();f.edit('dark');f.toggle();const input=f.node('preference-theme'),sharing=f.independent,sharingPanel=f.operational,shortcut=f.node('shortcut-commands'),reset=f.reset();let disposals=0,paints=0;const connection=new f.Node('section','connection-setup'),token=new f.Node('input');token.value='private unsaved fixture';connection.disposeConnectionSetup=()=>disposals++;connection.append(token);sharingPanel.append(connection);f.c.paintSettingsBody=()=>{paints++;connection.disposeConnectionSetup();};
 reset.focus();reset.onclick();assert.equal(paints,0);assert.equal(disposals,0);assert.equal(f.node('preference-theme'),input);assert.equal(input.value,'light');assert.equal(f.node('shortcut-commands'),shortcut);assert.equal(f.independent,sharing);assert.equal(sharing.value,'Unsaved sharing draft');assert.equal(sharingPanel.children.includes(connection),true);assert.equal(token.value,'private unsaved fixture');assert.equal(f.document.activeElement,f.modified);await f.flush();assert.equal(f.row(input.id).hidden,true);
 f.c.settingsDraft.keybindings.commands='Primary+J';shortcut.value='Primary+J';f.c.syncNativeShortcutRecording=async()=>{};vm.runInContext(source.slice(source.indexOf('async function finishShortcutRecording('),source.indexOf("window.addEventListener('keydown'")),f.c);f.c.shortcutRecording={id:'other-command',input:token,button:new f.Node('button')};const other=f.c.shortcutRecording;sharing.focus();f.row('shortcut-commands').children[1].children.find(node=>node.dataset.shortcutReset).onclick();assert.equal(f.c.shortcutRecording,other);assert.equal(shortcut.value,'Primary+K');assert.equal(f.document.activeElement,sharing);assert.equal(paints,0);assert.equal(disposals,0);
});

test('Reset ends only matching shortcut recording and updates existing checkbox state',async()=>{
 const f=settingsModifiedFixture();f.c.syncNativeShortcutRecording=async()=>{};vm.runInContext(source.slice(source.indexOf('async function finishShortcutRecording('),source.indexOf("window.addEventListener('keydown'")),f.c);const input=f.node('shortcut-commands'),record=input.parentElement.children.find(node=>node.className.includes('shortcut-record'));input.value='Primary+J';input.oninput();await f.c.startShortcutRecording(record,input,'commands');const reset=input.parentElement.children.find(node=>node.dataset.shortcutReset);reset.onclick();assert.equal(f.c.shortcutRecording,null);assert.equal(input.readOnly,false);assert.equal(input.value,'Primary+K');assert.equal(f.node(input.id),input);
 f.c.appSettings.defaults.nativeNotifications=false;f.c.settingsDraft.nativeNotifications=true;const row=f.c.settingInput(['appearance','nativeNotifications','Notifications','checkbox']),checkbox=row.children[1].children[0];assert.equal(checkbox.checked,true);row.children[1].children.find(node=>node.dataset.settingsReset).onclick();assert.equal(checkbox.checked,false);assert.equal(f.c.settingsDraft.nativeNotifications,false);
});


test('Settings filter context bounds Unicode and reads only search, toggle and the preference dirty flag',()=>{
 const search={value:'😀'.repeat(200)+'界private-tail'},modified={getAttribute:key=>key==='aria-pressed'?'true':null},selectors=[],c=vm.createContext({settingsDirty:true,$:selector=>{selectors.push(selector);if(selector==='#settings-search')return search;if(selector==='#settings-modified')return modified;throw Error('Private form control must not be read');}});c.settingsDraft=new Proxy({},{get(){throw Error('Private preferences must not be read');}});
 vm.runInContext(source.slice(source.indexOf('function settingsFilterContext('),source.indexOf('function resetSettingsFilter(')),c);let value=c.settingsFilterContext();assert.deepEqual(JSON.parse(JSON.stringify(value)),{query:'😀'.repeat(200),queryTruncated:true,modifiedOnly:true,preferencesDirty:true});assert.equal(new TextEncoder().encode(value.query).length,800);assert.deepEqual(selectors,['#settings-search','#settings-modified']);assert.equal(JSON.stringify(value).includes('private-tail'),false);
 search.value='😀'.repeat(200);c.settingsDirty=false;value=c.settingsFilterContext();assert.equal(value.queryTruncated,false);assert.equal(value.preferencesDirty,false);search.value='';assert.equal(c.settingsFilterContext().query,'');
 modified.getAttribute=()=> 'false';assert.equal(c.settingsFilterContext().modifiedOnly,false);modified.getAttribute=()=>null;assert.equal(c.settingsFilterContext(),null);c.$=()=>null;assert.equal(c.settingsFilterContext(),null);
});

test('Settings context iteration stops after 201 code points rather than materializing an unlimited query',()=>{
 const c=vm.createContext({settingsDirty:false,$:selector=>selector==='#settings-search'?{value:'unlimited query'}:{getAttribute:()=> 'false'},steps:0});vm.runInContext(source.slice(source.indexOf('function settingsFilterContext('),source.indexOf('function resetSettingsFilter(')),c);
 vm.runInContext("String.prototype[Symbol.iterator]=function(){return {next(){steps++;if(steps>201)throw Error('Unbounded Settings search iteration');return {value:'😀',done:false};},[Symbol.iterator](){return this;}}};",c);const value=c.settingsFilterContext();assert.equal(c.steps,201);assert.equal(value.query,'😀'.repeat(200));assert.equal(value.queryTruncated,true);
});

function reviewSkillFixture(){
 const f=settingsModifiedFixture(),picker=new f.Node('dialog'),form=new f.Node('form');picker.id='quick-action-picker';picker.open=false;form.id='quick-action-picker-form';picker.append(form);let paints=0,disposals=0;
 f.Node.prototype.replaceChildren=function(...children){const disconnect=node=>{node.isConnected=false;for(const child of node.children)disconnect(child);};for(const child of this.children)disconnect(child);this.children=[];this.append(...children);};
 f.Node.prototype.showModal=function(){this.open=true;};f.Node.prototype.close=function(){this.open=false;if(this.contains(f.document.activeElement))f.document.activeElement=f.document.body;this.listeners?.close?.();};
 const originalQuery=f.document.querySelectorAll;f.document.querySelectorAll=selector=>selector==='dialog[open]'?f.nodes.filter(n=>n.isConnected&&n.tagName==='DIALOG'&&n.open):originalQuery(selector);
 for(const key of ['claudeReviewCommand','codexReviewCommand']){f.c.settingsDraft[key]='';f.c.appSettings.defaults[key]='';f.c.settingsBaseline.values[key]='';const panel=new f.Node('section','preferences-panel');panel.dataset.category='agents';panel.append(f.c.settingInput(['agents',key,key,'text','']));}
 f.c.settingsCategory='agents';f.c.currentAgentRecord=()=>null;f.c.skillPickerMatches=skills=>skills;f.c.paintSettingsBody=()=>{paints++;disposals++;};
 const connection=new f.Node('section','connection-setup'),token=new f.Node('input');token.value='private unsaved token';connection.disposeConnectionSetup=()=>disposals++;connection.append(token);f.operational.append(connection);
 vm.runInContext(source.slice(source.indexOf('async function browseReviewSkill('),source.indexOf('async function chooseQuickAction(')),f.c);
 const pending=deferredSave();f.api(()=>pending.promise);const catalog={skills:[{name:'Review',commandName:'team-review',path:'/fixture/review',description:'Review changes',agents:['claude','codex']}]};
 return {...f,picker,form,pending,catalog,connection,token,counts:()=>({paints,disposals}),results:()=>f.nodes.filter(n=>n.isConnected&&n.className==='skill-picker-result'),start:(key='claudeReviewCommand')=>f.c.browseReviewSkill(key)};
}
test('review skill selection updates only exact Claude/Codex control and draft, preserving operational nodes and drafts',async()=>{
 for(const [key,prefix]of [['claudeReviewCommand','/'],['codexReviewCommand','$']]){
  const f=reviewSkillFixture(),input=f.node('preference-'+key),sharing=f.independent,connection=f.connection;input.focus();const opening=f.start(key);f.pending.resolve(f.catalog);await opening;const button=f.results()[0];button.focus();button.onclick();
  assert.equal(f.c.settingsDraft[key],prefix+'team-review');assert.equal(input.value,prefix+'team-review');assert.equal(f.node(input.id),input);assert.equal(f.c.settingsDirty,true);assert.equal(f.picker.open,false);assert.equal(f.document.activeElement,input);assert.deepEqual(f.counts(),{paints:0,disposals:0});assert.equal(f.connection,connection);assert.equal(f.token.value,'private unsaved token');assert.equal(sharing.value,'Unsaved sharing draft');assert.equal(f.calls.length,1);
 }
});
test('review skill picker ignores stale catalog or selection after close, reopen, replaced draft or source field, or Save',async()=>{
 for(const stage of ['catalog','selection'])for(const change of ['close','reopen','draft','baseline','field','field-value','saving','settings-close']){
  const f=reviewSkillFixture(),input=f.node('preference-claudeReviewCommand'),opening=f.start();if(stage==='selection'){f.pending.resolve(f.catalog);await opening;}const oldButton=f.results()[0];
  if(change==='close')f.picker.close();if(change==='settings-close')f.dialog.close();if(change==='reopen'){f.picker.close();f.api(async()=>f.catalog);await f.start('codexReviewCommand');}if(change==='draft')f.c.settingsDraft=structuredClone(f.c.settingsDraft);if(change==='baseline')f.c.settingsBaseline=structuredClone(f.c.settingsBaseline);if(change==='field-value')input.value='newer direct edit';if(change==='field'){input.isConnected=false;const replacement=new f.Node('input');replacement.id=input.id;replacement.value='';}if(change==='saving')f.c.settingsSaving=true;
  if(stage==='catalog'){f.pending.resolve(f.catalog);await opening;}else oldButton.onclick();
  assert.equal(f.c.settingsDraft.claudeReviewCommand,'',stage+change);assert.equal(input.value,change==='field-value'?'newer direct edit':'');assert.deepEqual(f.counts(),{paints:0,disposals:0});if(stage==='catalog'&&change!=='reopen')assert.equal(f.results().length,0);
 }
});
test('review skill selection follows Modified visibility and only restores picker-owned focus',async()=>{
 for(const move of ['owned','outside','default','child']){
  const f=reviewSkillFixture(),input=f.node('preference-claudeReviewCommand');if(move==='default'){f.c.appSettings.defaults.claudeReviewCommand='/team-review';f.c.settingsDraft.claudeReviewCommand='/old';input.value='/old';}f.toggle();const opening=f.start();f.pending.resolve(f.catalog);await opening;const button=f.results()[0];button.focus();let outside;
  if(move==='outside'){outside=f.independent;outside.focus();}if(move==='child'){outside=new f.Node('dialog');outside.open=true;outside.focus();}
  button.onclick();if(move==='child'){assert.equal(f.c.settingsDraft.claudeReviewCommand,'');assert.equal(f.picker.open,true);}else{assert.equal(f.c.settingsDraft.claudeReviewCommand,'/team-review');assert.equal(f.document.activeElement,move==='outside'?outside:move==='default'?f.modified:input);if(move==='default')assert.equal(input.closest('[hidden]')!==null,true);}assert.equal(f.token.value,'private unsaved token');assert.deepEqual(f.counts(),{paints:0,disposals:0});
 }
});

test('queued old native close event cannot retire a reopened review skill picker',async()=>{
 const f=reviewSkillFixture(),first=f.start();f.picker.open=false;const secondReply=deferredSave();f.api(()=>secondReply.promise);const second=f.start('codexReviewCommand');f.picker.listeners.close();secondReply.resolve(f.catalog);await second;assert.equal(f.results().length,1);f.pending.resolve(f.catalog);await first;assert.equal(f.results().length,1);f.results()[0].focus();f.results()[0].onclick();assert.equal(f.c.settingsDraft.codexReviewCommand,'$team-review');assert.equal(f.c.settingsDraft.claudeReviewCommand,'');assert.deepEqual(f.counts(),{paints:0,disposals:0});
});
