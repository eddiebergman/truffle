'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
function onboardingTestContext(value){
 const c=vm.createContext({source:'all',document:{activeElement:null,querySelectorAll:()=>[]},shell:null,openGeneration:0,paintGeneration:0,actionSerial:0,resume:null,returnGeneration:0,paint:async()=>{},...value});
 c.onboardingRoot=()=>c.dialog;c.onboardingIsOpen=()=>c.dialog?.open!==false;
 c.closeOnboarding=({forChild}={})=>{if(forChild)c.resume={ticket:c.returnGeneration};c.dialog?.close?.();};c.showSurface=()=>c.dialog.showModal();return c;
}
function installSkillEditorInitializer(c){
 c.Event=Event;const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog')")),c);
 c.disposeFeatureForm=()=>{};c.createFeatureForm=(dialog,form)=>{const current={dialog,host:form,ui:{inspect:{},refresh:{}}};c.lastSkillEditor=current;return current;};c.refreshSkillEditor=()=>{};
 const editor=fs.readFileSync('static/skills-editor.js','utf8');vm.runInContext(editor.slice(editor.indexOf('function openSkillEditor('),editor.indexOf('function updateScheduleFields(')),c);
}
const source=fs.readFileSync('static/onboarding.js','utf8');
const context=vm.createContext({});vm.runInContext(source.slice(0,source.indexOf('(function(){')),context);
test('onboarding follows integration then skills order and bounds back/forward navigation',()=>{
 assert.equal(context.onboardingNextStep('integrations'),'skills');
 assert.equal(context.onboardingNextStep('skills',-1),'integrations');
 assert.equal(context.onboardingNextStep('assistant',-1),'assistant');
 assert.equal(context.onboardingNextStep('finish'),'finish');
});
test('diff editor choices require both installation and diff support',()=>{
 const editors=[{id:'zed',installed:true,diffSupported:true},{id:'code',installed:true,diffSupported:false},{id:'cursor',installed:false,diffSupported:true}];
 assert.deepEqual(Array.from(context.onboardingSupportedEditors(editors),e=>e.id),['zed']);
 assert.equal(context.onboardingSupportedEditors([]).length,0);
});

test('skill import respects the chosen assistant when a skill supports both',()=>{
 assert.equal(context.onboardingSkillAgent({agents:['claude','codex']},'codex'),'codex');
 assert.equal(context.onboardingSkillAgent({agents:['claude']},'codex'),'claude');
});

test('keymap preview shows every displaced shortcut and preserves unrelated bindings',()=>{
 const current={commands:'Primary+K','find-work':'Primary+J','toggle-left':'Primary+L','view-notes':'Primary+Shift+P',other:'primary+b',safe:'Primary+Alt+P',disabled:''};
 const preview=context.onboardingKeymapPreset(current);
 assert.deepEqual(Array.from(preview.displaced),['view-notes','other']);
 assert.equal(preview.bindings.commands,'Primary+Shift+P');assert.equal(preview.bindings['find-work'],'Primary+P');assert.equal(preview.bindings['toggle-left'],'Primary+B');
 assert.equal(preview.bindings['view-notes'],'');assert.equal(preview.bindings.other,'');assert.equal(preview.bindings.safe,'Primary+Alt+P');assert.equal(preview.bindings.disabled,'');
 assert.equal(current.other,'primary+b');assert.equal(current['view-notes'],'Primary+Shift+P');
 const chords=Object.values(preview.bindings).filter(Boolean).map(chord=>chord.toUpperCase());assert.equal(new Set(chords).size,chords.length);
});

test('onboarding serializes actions and releases controls after failures',async()=>{
 const back={disabled:false,closest:()=>null},next={disabled:false,closest:()=>null},select={disabled:false,closest:()=>null},attributes={},errors=[];
 const c=onboardingTestContext({actionPending:false,dialog:{setAttribute:(name,value)=>attributes[name]=value,querySelectorAll:()=>[back,next,select],querySelector:()=>null},documentState:{progress:{step:'assistant'}},$:()=>back,showError:error=>errors.push(error.message)});
 vm.runInContext(source.slice(source.indexOf(' async function runAction('),source.indexOf(' const button=')),c);
 let release,executed=0;
 const first=c.runAction(async()=>{executed++;await new Promise(resolve=>release=resolve);});
 assert.equal(next.disabled,true);assert.equal(select.disabled,true);assert.equal(attributes['aria-busy'],'true');
 await c.runAction(async()=>executed++);assert.equal(executed,1);
 release();await first;assert.equal(back.disabled,true);assert.equal(next.disabled,false);assert.equal(select.disabled,false);
 c.documentState.progress.step='theme';await c.runAction(async()=>{throw new Error('Save failed');});assert.deepEqual(errors,['Save failed']);assert.equal(back.disabled,false);assert.equal(attributes['aria-busy'],'false');
});

test('unsupported setup versions keep navigation disabled while Close remains usable',()=>{
 const close={disabled:false,closest:()=>null},back={disabled:false,closest:()=>null},next={disabled:false,closest:()=>null};
 const c=onboardingTestContext({dialog:{setAttribute(){},querySelectorAll:()=>[close,back,next],querySelector:()=>close},documentState:{progressReadOnly:true,progress:{step:'assistant'}},$:()=>back});
 vm.runInContext(source.slice(source.indexOf(' function setBusy('),source.indexOf(' const button=')),c);
 c.setBusy(false);assert.equal(close.disabled,false);assert.equal(back.disabled,true);assert.equal(next.disabled,true);
 c.setBusy(true);assert.equal(close.disabled,true);
});

test('portable review selection changes only the explicitly chosen assistant',()=>{
 const skill={available:true,commands:{claude:'/truffle-review',codex:'$truffle-review'}};
 assert.deepEqual({...context.onboardingReviewPatch(skill,'claude')},{claudeReviewCommand:'/truffle-review'});
 assert.deepEqual({...context.onboardingReviewPatch(skill,'codex')},{codexReviewCommand:'$truffle-review'});
 assert.throws(()=>context.onboardingReviewPatch({...skill,available:false},'claude'));
 assert.throws(()=>context.onboardingReviewPatch(skill,'other'));
 const defaults=JSON.parse(fs.readFileSync('static/config/defaults.json','utf8')).values;
 assert.equal(defaults.claudeReviewCommand,'');assert.equal(defaults.codexReviewCommand,'');
});


test('onboarding busy state does not disable the globally hosted App chat controls',()=>{
 const owned={disabled:false,closest:()=>null},chat={disabled:false,closest:()=>null},back={};
 const c=onboardingTestContext({dialog:{setAttribute(){},querySelectorAll:selector=>selector.includes('.onboarding-shell')?[owned]:[owned,chat],querySelector:()=>null},documentState:{progress:{step:'theme'}},$:()=>back});
 vm.runInContext(source.slice(source.indexOf(' function setBusy('),source.indexOf(' const button=')),c);
 c.setBusy(true);assert.equal(owned.disabled,true);assert.equal(chat.disabled,false);
 c.setBusy(false);assert.equal(owned.disabled,false);assert.equal(chat.disabled,false);
});

function profileFixture({previewError,initialProfile,expectedRevision,expectedContentHash="a".repeat(64)}={}){
 class Node{
  constructor(tag){this.tagName=tag;this.children=[];this.attrs={};this.dataset={};this.value='';this.handlers={};}
  append(...children){for(const child of children){child.parent=this;this.children.push(child);}}
  replaceChildren(...children){this.children=[];this.append(...children);}
  setAttribute(key,value){this.attrs[key]=value;}
  closest(tag){return tag==='dialog'?dialog:null;}
  addEventListener(type,handler){this.handlers[type]=handler;}
  removeEventListener(type){delete this.handlers[type];}
  focus(){this.focused=true;}
  click(){return this.onclick?.();}
 }
 const dialog=new Node('dialog'),container=new Node('main'),calls=[];dialog.append(container);
 const current={revision:3,contentHash:'a'.repeat(64),values:{theme:'system',editor:'zed',codexReviewCommand:'',keybindings:{commands:'Primary+K',other:'Primary+O'},quickActions:[]},approval:{theme:'routine',editor:'user',codexReviewCommand:'user',quickActions:'user',keybindings:'routine'},fields:[{key:'theme',label:'Theme'},{key:'editor',label:'Editor'}],commands:[{id:'commands',label:'Commands'}]};
 const c=onboardingTestContext({TextEncoder,structuredClone,document:{createElement:tag=>new Node(tag)},api:async(path,payload)=>{calls.push({path,payload});if(path==='/api/settings/preview'){if(previewError)throw new Error(previewError);return {revision:current.revision,contentHash:current.contentHash,values:structuredClone(payload.patch)};}if(!payload)return structuredClone(current);current.values={...current.values,...structuredClone(payload.patch)};current.revision++;current.contentHash='b'.repeat(64);return structuredClone(current);},loadAppSettings:async()=>{}});
 const settings=fs.readFileSync('static/settings.js','utf8');vm.runInContext(settings.slice(settings.indexOf('function mergeSettingsProfile('),settings.indexOf('function exportSettingsProfile(')),c);vm.runInContext(fs.readFileSync('static/settings-profile.js','utf8'),c);const root=c.renderSettingsProfilePreview(container,{initialProfile,expectedRevision,expectedContentHash});
 const nodes=()=>{const visit=n=>[n,...n.children.flatMap(visit)];return visit(root);};const button=text=>nodes().find(n=>n.tagName==='button'&&n.textContent===text),input=nodes().find(n=>n.tagName==='textarea'),file=nodes().find(n=>n.tagName==='input'&&n.type==='file'),status=nodes().find(n=>n.attrs.role==='status');
 const paste=async values=>{input.value=JSON.stringify({format:'truffle-settings',version:1,values});await button('Preview pasted profile').onclick();};
 return {c,current,calls,root,dialog,container,nodes,button,input,file,status,paste};
}
test('company profile preview reuses schema validation, clears raw input, defaults routine only and requires explicit apply',async()=>{
 const f=profileFixture();await f.paste({theme:'midnight',editor:'code',codexReviewCommand:'$review <img src=x onerror=alert(1)>',keybindings:{commands:'Primary+Shift+P'}});
 assert.equal(f.input.value,'');assert.equal(f.root.attrs['data-private'],'true');assert.equal(f.root.attrs['data-agent-private'],'true');
 assert.equal(f.calls.filter(c=>c.path==='/api/settings'&&c.payload).length,0);assert.ok(f.calls.some(c=>c.path==='/api/settings/preview'));assert.equal(f.current.values.theme,'system');
 const boxes=f.nodes().filter(n=>n.type==='checkbox');assert.equal(boxes.length,4);assert.equal(boxes.filter(n=>n.checked).length,2);assert.equal(boxes.find(n=>n.attrs['aria-label']==='Approve changing Editor').checked,false);
 assert.ok(f.nodes().some(n=>n.tagName==='pre'&&n.textContent==='$review <img src=x onerror=alert(1)>'));assert.equal(f.nodes().some(n=>n.tagName==='img'),false);
 await f.button('Apply selected changes').onclick();const patch=f.calls.find(c=>c.path==='/api/settings'&&c.payload).payload;assert.equal(patch.revision,3);assert.deepEqual(Object.keys(patch.patch).sort(),['keybindings','theme']);assert.equal(patch.patch.keybindings.other,'Primary+O');assert.equal(patch.patch.keybindings.commands,'Primary+Shift+P');assert.equal(f.current.values.editor,'zed');assert.equal(f.calls.some(c=>c.path.includes('onboarding')),false);
});
test('company profile user approval is explicit and absent policy fails closed',async()=>{
 const f=profileFixture();delete f.current.approval.editor;await f.paste({editor:'code'});assert.equal(f.button('Apply selected changes').disabled,true);const box=f.nodes().find(n=>n.type==='checkbox');assert.equal(box.checked,false);box.checked=true;box.onchange();assert.equal(f.button('Apply selected changes').disabled,false);await f.button('Apply selected changes').onclick();assert.equal(f.current.values.editor,'code');
});
test('changed settings disable company Apply until repreview resets approvals',async()=>{
 const f=profileFixture();await f.paste({theme:'midnight',editor:'code'});const box=f.nodes().find(n=>n.attrs['aria-label']==='Approve changing Editor');box.checked=true;box.onchange();f.current.revision++;
 await f.button('Apply selected changes').onclick();assert.equal(f.button('Apply selected changes').disabled,true);assert.match(f.status.textContent,/Refresh the preview/);assert.equal(f.calls.filter(c=>c.path==='/api/settings'&&c.payload).length,0);
 await f.button('Refresh preview').onclick();assert.equal(f.nodes().find(n=>n.attrs['aria-label']==='Approve changing Editor').checked,false);assert.equal(f.button('Apply selected changes').disabled,false);
});
test('invalid, oversized and backend-rejected company profiles show no partial preview or write',async()=>{
 for(const value of ['{"private-fixture":','null','x'.repeat(65537),'"'+'é'.repeat(33000)+'"']){const f=profileFixture();f.input.value=value;await f.button('Preview pasted profile').onclick();assert.equal(f.input.value,'');assert.equal(f.button('Apply selected changes').disabled,true);assert.equal(f.nodes().some(n=>n.type==='checkbox'),false);assert.equal(f.calls.some(c=>c.path==='/api/settings'&&c.payload),false);assert.equal(f.status.textContent.includes('private-fixture'),false);}
 const f=profileFixture({previewError:'Unsupported theme.'});await f.paste({theme:'unknown'});assert.equal(f.status.textContent,'Unsupported theme.');assert.equal(f.button('Apply selected changes').disabled,true);assert.equal(f.nodes().some(n=>n.type==='checkbox'),false);
});
test('company profile disposal clears private fields and ignores delayed preview responses',async()=>{
 for(const stage of ['settings','preview']){
  const f=profileFixture();let release;const original=f.c.api;f.c.api=async(path,payload)=>{if(stage==='settings'&&path==='/api/settings'&&!payload||stage==='preview'&&path==='/api/settings/preview')return new Promise(resolve=>release=()=>resolve(stage==='settings'?structuredClone(f.current):{revision:3,contentHash:f.current.contentHash,values:{...f.current.values,theme:'midnight'}}));return original(path,payload);};
  f.input.value=JSON.stringify({format:'truffle-settings',version:1,values:{theme:'midnight'}});const pending=f.button('Preview pasted profile').onclick();while(!release)await Promise.resolve();f.file.value='fixture.json';f.dialog.handlers.close();assert.equal(f.input.value,'');assert.equal(f.file.value,'');release();await pending;assert.equal(f.nodes().some(n=>n.type==='checkbox'),false);assert.equal(f.button('Apply selected changes').disabled,true);
 }
});
test('onboarding busy state preserves company importer disabled policy',()=>{
 const owned={disabled:false,closest:()=>null},company={disabled:true,closest:selector=>selector.includes('.settings-profile-import')?{}:null},back={};
 const c=onboardingTestContext({dialog:{setAttribute(){},querySelectorAll:()=>[owned,company],querySelector:()=>null},documentState:{progress:{step:'company'}},$:()=>back});vm.runInContext(source.slice(source.indexOf(' function setBusy('),source.indexOf(' const button=')),c);c.setBusy(false);assert.equal(company.disabled,true);assert.equal(owned.disabled,false);
});

test('shared proposal review previews only requested changes and applies selected fields at the proposal revision',async()=>{
 const f=profileFixture({initialProfile:{format:'truffle-settings',version:1,values:{theme:'midnight',editor:'code'}},expectedRevision:3});await f.root.ready;
 assert.equal(f.input.hidden,true);assert.equal(f.button('Choose profile file').hidden,true);assert.equal(f.button('Preview pasted profile').hidden,true);assert.equal(f.root.attrs['data-agent-context-exclude'],'true');
 const boxes=f.nodes().filter(n=>n.type==='checkbox');assert.equal(boxes.length,2);assert.equal(boxes.find(n=>n.attrs['aria-label']==='Apply Theme').checked,true);assert.equal(boxes.find(n=>n.attrs['aria-label']==='Approve changing Editor').checked,false);assert.equal(f.calls.filter(c=>c.path==='/api/settings'&&c.payload).length,0);
 await f.button('Apply selected changes').onclick();const request=f.calls.find(c=>c.path==='/api/settings'&&c.payload).payload;assert.equal(request.revision,3);assert.equal(request.contentHash,"a".repeat(64));assert.deepEqual(Object.keys(request.patch),['theme']);assert.equal(f.current.values.editor,'zed');
});
test('stale initial proposals require an explicit fresh preview and reset user approval choices',async()=>{
 const f=profileFixture({initialProfile:{format:'truffle-settings',version:1,values:{theme:'midnight',editor:'code'}},expectedRevision:2});await f.root.ready;
 assert.match(f.status.textContent,/Settings changed since this proposal/);assert.equal(f.nodes().some(n=>n.type==='checkbox'),false);assert.equal(f.button('Apply selected changes').disabled,true);assert.equal(f.calls.some(c=>c.path==='/api/settings/preview'),false);
 await f.button('Apply selected changes').onclick();assert.equal(f.calls.some(c=>c.path==='/api/settings'&&c.payload),false);
 await f.button('Refresh preview').onclick();assert.equal(f.nodes().find(n=>n.attrs['aria-label']==='Approve changing Editor').checked,false);assert.equal(f.button('Apply selected changes').disabled,false);
});
test('shared proposal renderer copies its input and enforces the bounded settings profile schema',async()=>{
 const initialProfile={format:'truffle-settings',version:1,values:{editor:'code'}},f=profileFixture({initialProfile,expectedRevision:3});initialProfile.values.editor='cursor';await f.root.ready;assert.ok(f.nodes().some(n=>n.tagName==='pre'&&n.textContent==='code'));assert.equal(f.nodes().some(n=>n.tagName==='pre'&&n.textContent==='cursor'),false);
 const huge=profileFixture({initialProfile:{format:'truffle-settings',version:1,values:{codexReviewCommand:'x'.repeat(65537)}},expectedRevision:3});await huge.root.ready;assert.match(huge.status.textContent,/64 KiB/);assert.equal(huge.calls.length,0);assert.equal(huge.button('Apply selected changes').disabled,true);
 f.root.disposeProfileImport();assert.equal(f.nodes().some(n=>n.type==='checkbox'),false);assert.equal(f.input.value,'');assert.equal(f.button('Refresh preview').disabled,true);
});

test('same-revision file changes invalidate proposal previews and approvals',async()=>{
 const initialProfile={format:'truffle-settings',version:1,values:{editor:'code'}},f=profileFixture({initialProfile,expectedRevision:3,expectedContentHash:'c'.repeat(64)});await f.root.ready;
 assert.match(f.status.textContent,/changed since this proposal/);assert.equal(f.calls.some(c=>c.path==='/api/settings/preview'),false);await f.button('Refresh preview').onclick();const box=f.nodes().find(n=>n.type==='checkbox');box.checked=true;box.onchange();f.current.contentHash='d'.repeat(64);await f.button('Apply selected changes').onclick();assert.match(f.status.textContent,/changed elsewhere/);assert.equal(f.calls.some(c=>c.path==='/api/settings'&&c.payload),false);assert.equal(f.button('Apply selected changes').disabled,true);
});

function onboardingDecisionFixture(){
 const nodes=new Map(),calls=[];const node=(tag,text,cls)=>({tagName:tag,textContent:text||'',className:cls||'',children:[],attrs:{},dataset:{},value:'',append(...children){this.children.push(...children);},replaceChildren(...children){this.children=children;},setAttribute(name,value){this.attrs[name]=value;},querySelectorAll:()=>[]});const content=node('main');
 for(const id of ['onboarding-continue','onboarding-skip','onboarding-back','onboarding-step-count'])nodes.set('#'+id,node('button'));
 const c=onboardingTestContext({node,content,actionPending:false,documentState:{progress:{step:'assistant'}},paintGeneration:0,$:selector=>nodes.get(selector),paragraph:text=>content.append(node('p',text)),button:(text,action)=>Object.assign(node('button',text),{onclick:action}),preference:async patch=>calls.push(['preference',patch]),next:async status=>calls.push(['next',status]),runAction:async action=>action()});
 vm.runInContext(source.slice(source.indexOf(' function confirmChoice('),source.indexOf(' async function leaveSettings(')),c);return {c,calls,nodes,content};
}
test('assistant question uses local pressed choices and one explicit footer commit',async()=>{
 const f=onboardingDecisionFixture(),items=[{id:'claude',label:'Claude'},{id:'codex',label:'Codex'}];f.c.confirmChoice(items,'claude','defaultAgent','Which assistant would you like for new chats?');
 assert.equal(f.calls.length,0);const group=f.content.children.find(node=>node.attrs.role==='group');assert.equal(group.attrs['aria-label'],'Which assistant would you like for new chats?');assert.equal(group.children[0].attrs['aria-pressed'],'true');assert.equal(f.nodes.get('#onboarding-continue').textContent,'Use Claude');
 await group.children[1].onclick();assert.equal(f.calls.length,0);assert.equal(group.children[0].attrs['aria-pressed'],'false');assert.equal(group.children[1].attrs['aria-pressed'],'true');assert.equal(f.nodes.get('#onboarding-continue').textContent,'Use Codex');
 await f.c.continueAction();assert.equal(f.calls[0][0],'preference');assert.equal(f.calls[0][1].defaultAgent,'codex');assert.equal(f.calls[1][0],'next');assert.equal(f.content.children.some(node=>node.textContent==='Use selected assistant'),false);
});
test('a single installed assistant or editor needs no redundant picker and saves only on confirmation',async()=>{
 for(const [key,id,label]of [['defaultAgent','codex','Codex'],['editor','zed','Zed']]){
  const f=onboardingDecisionFixture();f.c.confirmChoice([{id,label}],'unavailable',key,'Choose a tool');assert.equal(f.content.children.some(node=>node.attrs.role==='group'),false);assert.equal(f.calls.length,0);assert.equal(f.nodes.get('#onboarding-continue').textContent,'Use '+label);await f.c.continueAction();assert.equal(f.calls[0][1][key],id);
 }
});
test('question confirmation preserves revision plus content hash and does not advance after a conflict',async()=>{
 const f=onboardingDecisionFixture();const requests=[];f.c.api=async(path,payload)=>{requests.push({path,payload});if(!payload)return {revision:4,contentHash:'a'.repeat(64)};throw new Error('Settings changed.');};f.c.loadAppSettings=async()=>{};
 vm.runInContext(source.slice(source.indexOf(' async function preference('),source.indexOf(' function paragraph(')),f.c);f.c.confirmChoice([{id:'codex',label:'Codex'}],'codex','defaultAgent','Choose an assistant');await assert.rejects(f.c.continueAction(),/Settings changed/);assert.equal(requests[1].payload.revision,4);assert.equal(requests[1].payload.contentHash,'a'.repeat(64));assert.equal(requests[1].payload.patch.defaultAgent,'codex');assert.equal(f.calls.some(call=>call[0]==='next'),false);
});
test('remaining native onboarding selectors have explicit accessible labels',()=>{
 const f=onboardingDecisionFixture();for(const label of ['Theme','Skill availability','Saved skill to schedule']){const select=f.c.choices([['one','One']],'noop','one',label);assert.equal(select.attrs['aria-label'],label);}
});
test('no installed assistant keeps a clear skip path without changing preferences',async()=>{
 const f=onboardingDecisionFixture();Object.assign(f.c,{documentState:{progress:{step:'assistant'},assistants:[]},labels:{assistant:'Your assistant',theme:'Theme'},error:{},dialog:{},setting:()=> 'claude'});
 vm.runInContext(source.slice(source.indexOf(' async function paint('),source.indexOf(' window.openOnboarding=')),f.c);await f.c.paint();assert.equal(f.nodes.get('#onboarding-continue').textContent,'Continue without an assistant');await f.c.continueAction();assert.equal(f.calls.length,1);assert.deepEqual(f.calls[0],['next','skipped']);
});
test('actual assistant and editor steps render one decision instead of autosaving dropdowns and duplicate CTAs',async()=>{
 for(const step of ['assistant','editor']){
  const f=onboardingDecisionFixture();Object.assign(f.c,{documentState:{progress:{step},assistants:[{id:'claude',label:'Claude',installed:true},{id:'codex',label:'Codex',installed:true}],editors:[{id:'zed',label:'Zed',installed:true,diffSupported:true}]},labels:{assistant:'Your assistant',editor:'Diff editor'},error:{},dialog:{},setting:()=>step==='assistant'?'codex':'zed',onboardingSupportedEditors:editors=>editors.filter(editor=>editor.installed&&editor.diffSupported)});
  vm.runInContext(source.slice(source.indexOf(' async function paint('),source.indexOf(' window.openOnboarding=')),f.c);await f.c.paint();assert.equal(f.content.children.some(node=>node.tagName==='select'||/Use selected/.test(node.textContent)),false);assert.equal(f.calls.length,0);assert.equal(f.nodes.get('#onboarding-continue').textContent,step==='assistant'?'Use Codex':'Use Zed');if(step==='assistant')assert.ok(f.content.children.some(node=>/preferred installed assistant/.test(node.textContent)));
 }
});
test('entering a direct skill path initializes the chosen default assistant after resetting the form',async()=>{
 for(const preferred of ['codex','claude']){
  const fields=Object.fromEntries(['title','skill','agent','command','cwd'].map(key=>[key,{value:'stale'}])),form={elements:fields,reset(){for(const field of Object.values(fields))field.value='';fields.agent.value='claude';}},error={},child={dataset:{},showModal(){this.open=true;}},nodes={'#skill-editor-form':form,'#skill-editor-error':error,'#skill-editor':child};
  const c=onboardingTestContext({selectedSkill:null,pendingImport:false,baseline:[],api:async()=>({presets:[]}),save:async()=>{},dialog:{close(){}},$:selector=>nodes[selector],setting:()=>preferred,returnAfter(){}});
  installSkillEditorInitializer(c);vm.runInContext(source.slice(source.indexOf(' async function addSkill('),source.indexOf(' async function discoverSkills(')),c);await c.addSkill(null);assert.equal(child.dataset.skillSource,'file');assert.equal(fields.agent.value,preferred);assert.equal(fields.command.value,'');assert.equal(child.open,true);assert.equal(error.hidden,true);
 }
});

function onboardingAsyncFixture(){
 const f=onboardingDecisionFixture();Object.assign(f.c,{documentState:{progress:{step:'keymap',steps:{}},editors:[{id:'zed',label:'Zed',installed:true,diffSupported:true}]},dialog:{open:true},labels:{keymap:'Familiar shortcuts',editor:'Diff editor'},error:{textContent:''},setting:()=> 'zed',onboardingSupportedEditors:context.onboardingSupportedEditors,onboardingKeymapPreset:context.onboardingKeymapPreset,appSettings:{commands:['commands','find-work','toggle-left'].map(id=>({id,label:id}))}});f.c.showError=error=>f.c.error.textContent=error.message;
 vm.runInContext(source.slice(source.indexOf(' async function paint('),source.indexOf(' window.openOnboarding=')),f.c);return f;
}
const shortcutPreview=binding=>({revision:1,values:{keybindings:{commands:binding,'find-work':'Primary+P','toggle-left':'Primary+B'}}});
test('a delayed shortcut preview cannot append old controls after Back renders the editor step',async()=>{
 const f=onboardingAsyncFixture();let release;f.c.api=()=>new Promise(resolve=>release=resolve);const pending=f.c.paint();assert.ok(release);
 f.c.documentState.progress.step='editor';await f.c.paint();release(shortcutPreview('Primary+K'));await pending;
 assert.equal(f.content.children[0].textContent,'Diff editor');assert.equal(f.content.children.some(node=>node.tagName==='ul'||node.textContent==='Apply these shortcuts'),false);assert.equal(f.nodes.get('#onboarding-continue').textContent,'Use Zed');assert.equal(f.calls.length,0);
});
test('closing setup ignores delayed preview success and failure without adding controls or stale errors',async()=>{
 for(const fails of [false,true]){
  const f=onboardingAsyncFixture();let settle;f.c.api=()=>new Promise((resolve,reject)=>settle=()=>fails?reject(new Error('Stale request failed')):resolve(shortcutPreview('Primary+K')));const pending=f.c.paint();f.c.dialog.open=false;settle();await pending;
  assert.equal(f.content.children.some(node=>node.tagName==='ul'||node.textContent==='Apply these shortcuts'),false);assert.equal(f.c.error.textContent,'');assert.equal(f.calls.length,0);
 }
});
test('returning to the same step uses only its newest preview response',async()=>{
 const f=onboardingAsyncFixture(),release=[];f.c.api=()=>new Promise(resolve=>release.push(resolve));const old=f.c.paint();f.c.dialog.open=false;f.c.dialog.open=true;const current=f.c.paint();release[1](shortcutPreview('Primary+L'));await current;release[0](shortcutPreview('Primary+K'));await old;
 const lists=f.content.children.filter(node=>node.tagName==='ul');assert.equal(lists.length,1);assert.match(lists[0].children[0].textContent,/Primary\+L/);assert.equal(f.content.children.filter(node=>node.textContent==='Apply these shortcuts').length,1);
});
test('a silent Settings load failure restores setup with an error instead of waiting on a closed child',async()=>{
 const f=onboardingDecisionFixture();let returnHandlers=0;const child={open:false};Object.assign(f.c,{documentState:{progress:{step:'company'}},error:{textContent:''},dialog:{open:true,close(){this.open=false;},showModal(){this.open=true;}},save:async(step,status)=>f.calls.push([step,status]),openPreferences:async()=>undefined,filterSettings:()=>f.calls.push('filter'),returnAfter:()=>returnHandlers++,$:()=>child,setBusy(){},showError:error=>f.c.error.textContent=error.message});
 vm.runInContext(source.slice(source.indexOf(' async function runAction('),source.indexOf(' function setBusy(')),f.c);vm.runInContext(source.slice(source.indexOf(' async function leaveSettings('),source.indexOf(' function returnAfter(')),f.c);
 await f.c.runAction(()=>f.c.leaveSettings('sharing'));assert.equal(f.c.dialog.open,true);assert.equal(child.open,false);assert.equal(returnHandlers,0);assert.match(f.c.error.textContent,/Settings could not open.*progress is saved/);assert.deepEqual(f.calls,[['company','pending']]);assert.equal(f.c.documentState.progress.step,'company');
 child.open=true;f.c.openPreferences=async({category})=>{assert.equal(category,'sharing');return true;};await f.c.runAction(()=>f.c.leaveSettings('sharing'));assert.equal(f.c.dialog.open,false);assert.equal(returnHandlers,1);assert.equal(f.calls.includes('filter'),false);
});
test('closing during handoff progress save does not open Settings after setup has closed',async()=>{
 const f=onboardingDecisionFixture();let release,opens=0;Object.assign(f.c,{documentState:{progress:{step:'company'}},dialog:{open:true,close(){this.open=false;}},save:()=>new Promise(resolve=>release=resolve),openPreferences:async()=>opens++});vm.runInContext(source.slice(source.indexOf(' async function leaveSettings('),source.indexOf(' function returnAfter(')),f.c);
 const pending=f.c.leaveSettings('sharing');f.c.dialog.close();release();await pending;assert.equal(opens,0);assert.equal(f.c.dialog.open,false);
});

function onboardingHostFixture({step='assistant'}={}){
 const {MutationObserver,recordOpenChange}=require(require('node:path').resolve('tests/helpers/dialog-mutations.cjs'))();
 class Element{
  constructor(tag){this.tagName=tag.toUpperCase();this.children=[];this.parentElement=null;this.attrs={};this.dataset={};this.hidden=false;this.open=false;this.value='';this.listeners=new Map();this._disabled=false;}
  get open(){return this._open||false;}set open(value){if(this._open!==value){this._open=value;recordOpenChange(this);}}
  get disabled(){return this._disabled;}
  set disabled(value){this._disabled=value;if(value&&document.activeElement===this)document.activeElement=document.body;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  focus(options){if(this.isConnected&&!this.disabled){document.activeElement=this;this.focusOptions=options;}}
  get isConnected(){return this===body||Boolean(this.parentElement?.isConnected);}
  append(...nodes){for(const child of nodes){if(child.parentElement)child.parentElement.children=child.parentElement.children.filter(node=>node!==child);child.parentElement=this;this.children.push(child);}}
  replaceChildren(...nodes){if(this.children.some(child=>child.contains(document.activeElement)))document.activeElement=document.body;for(const child of this.children)child.parentElement=null;this.children=[];this.append(...nodes);}
  setAttribute(key,value){this.attrs[key]=value;}
  getAttribute(key){return this.attrs[key]||null;}
  addEventListener(type,callback,options={}){const list=this.listeners.get(type)||[];list.push({callback,once:options.once});this.listeners.set(type,list);}
  removeEventListener(type,callback){this.listeners.set(type,(this.listeners.get(type)||[]).filter(item=>item.callback!==callback));}
  dispatchEvent(event){for(const item of [...this.listeners.get(event.type)||[]])item.callback(event);return !event.defaultPrevented;}
  dispatch(type){for(const item of [...this.listeners.get(type)||[]]){if(item.once)this.removeEventListener(type,item.callback);item.callback({target:this});}}
  showModal(){this.open=true;}
  close(){if(!this.open)return;this.open=false;if(this.contains(document.activeElement))document.activeElement=document.body;this.dispatch('close');}
  closest(selector){for(let node=this;node;node=node.parentElement)if(selector.split(',').some(part=>node.matches(part)))return node;return null;}
  matches(selector){if(selector==='dialog[open]')return this.tagName==='DIALOG'&&this.open;if(selector.startsWith('.'))return (this.className||'').split(' ').includes(selector.slice(1));if(selector.startsWith('#'))return this.id===selector.slice(1);if(selector==='[data-onboarding-close]')return Boolean(this.dataset.onboardingClose);return this.tagName===selector.toUpperCase();}
  querySelectorAll(selector){const nodes=this.children.flatMap(child=>[child,...child.querySelectorAll('*')]);if(selector==='*')return nodes;return nodes.filter(node=>selector.split(',').some(part=>{const words=part.trim().split(' ');return node.matches(words.at(-1))&&(words.length===1||node.parentElement?.closest(words[0]));}));}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
 }
 const body=new Element('body'),document={body,activeElement:body,createElement:tag=>new Element(tag),querySelector:selector=>body.querySelector(selector),querySelectorAll:selector=>body.querySelectorAll(selector)};
 const host=new Element('section');host.id='inline-setup';host.hidden=true;body.append(host);const settings=new Element('dialog');settings.id='settings-dialog';body.append(settings);
 const calls=[],progress={revision:0,step,steps:{}},connection={disposed:0},profile={disposed:0};
 const c=vm.createContext({document,MutationObserver,source:'all',window:{},settingsCategory:'appearance',TextEncoder,structuredClone,$:selector=>document.querySelector(selector),setting:(key,fallback)=>fallback,appSettings:{commands:[]},loadAppSettings:async()=>{},filterSettings(){},openPreferences:async({category}={})=>{settings.showModal();c.settingsCategory=category;return true;},renderConnectionSetup:async container=>{const setup=new Element('section');setup.className='connection-setup';let disposed=false;setup.disposeConnectionSetup=()=>{if(!disposed){disposed=true;connection.disposed++;}};container.append(setup);},renderSettingsProfilePreview:container=>{const importer=new Element('section');importer.className='settings-profile-import';let disposed=false;importer.disposeProfileImport=()=>{if(!disposed){disposed=true;profile.disposed++;}};container.append(importer);},api:async(path,payload)=>{calls.push({path,payload});if(path==='/api/onboarding'){if(payload){progress.step=payload.step||progress.step;progress.revision++;return structuredClone(progress);}return {progress:structuredClone(progress),assistants:[{id:'codex',label:'Codex',installed:true}],editors:[]};}if(path==='/api/settings')return {revision:2,contentHash:'a'.repeat(64),values:{keybindings:{commands:'Primary+K','find-work':'Primary+P','toggle-left':'Primary+B'}}};return {presets:[],skills:[]};}});
 installSkillEditorInitializer(c);vm.runInContext(source,c);const tick=()=>new Promise(resolve=>setImmediate(resolve));return {c,body,document,host,settings,calls,progress,connection,profile,Element,tick};
}
test('one onboarding shell moves between connected inline section and standalone dialog without duplicate IDs',async()=>{
 const f=onboardingHostFixture();await f.c.window.openOnboarding({host:f.host});const shell=f.host.children[0];assert.equal(f.c.window.onboardingRoot(),f.host);assert.equal(f.c.window.onboardingIsOpen(),true);assert.equal(f.host.hidden,false);assert.equal(f.host.querySelectorAll('dialog').length,0);
 await f.c.window.openOnboarding();const modal=f.document.querySelector('#onboarding-dialog');assert.equal(modal.children[0],shell);assert.equal(f.host.hidden,true);assert.equal(modal.open,true);assert.equal(f.c.window.closeOnboarding({host:f.host}),false);assert.equal(modal.open,true);
 await f.c.window.openOnboarding({host:f.host});assert.equal(f.host.children[0],shell);assert.equal(modal.open,false);for(const id of ['onboarding-continue','onboarding-step-count','onboarding-back'])assert.equal(f.document.querySelectorAll('#'+id).length,1);f.c.window.closeOnboarding({host:f.host});assert.equal(f.host.hidden,true);assert.equal(f.c.window.onboardingIsOpen(),false);
 const detached=new f.Element('section');await assert.rejects(f.c.window.openOnboarding({host:detached}),/connected section/);
});
test('closing pending inline loads or shortcut previews prevents stale rendering and superseded hosts',async()=>{
 const f=onboardingHostFixture({step:'keymap'}),original=f.c.api;let release;
 f.c.api=async(path,payload)=>path==='/api/settings'?new Promise(resolve=>release=resolve):original(path,payload);const opening=f.c.window.openOnboarding({host:f.host});for(let spin=0;!release&&spin<20;spin++)await f.tick();assert.ok(release);f.c.window.closeOnboarding({host:f.host});release({revision:2,contentHash:'a'.repeat(64),values:{keybindings:{}}});await opening;assert.equal(f.host.hidden,true);assert.equal(f.host.querySelectorAll('ul').length,0);
 const g=onboardingHostFixture();let first;const initial=g.c.api;g.c.api=async(path,payload)=>!first&&path==='/api/onboarding'?new Promise(resolve=>first=resolve):initial(path,payload);const pending=g.c.window.openOnboarding({host:g.host});g.c.window.closeOnboarding({host:g.host});await g.c.window.openOnboarding();first({progress:{step:'finish',revision:99,steps:{}},assistants:[],editors:[]});await pending;assert.equal(g.c.window.onboardingRoot(),g.document.querySelector('#onboarding-dialog'));assert.equal(g.document.querySelector('#onboarding-continue').textContent,'Use Codex');assert.equal(g.host.hidden,true);
});
test('Settings child resumes the original inline host and a closed chat cannot be resurrected by its child',async()=>{
 for(const cancel of [false,true]){
  const f=onboardingHostFixture({step:'company'});await f.c.window.openOnboarding({host:f.host});const open=f.host.querySelectorAll('button').find(node=>node.textContent==='Open sharing setup');open.onclick();for(let spin=0;!f.settings.open&&spin<20;spin++)await f.tick();assert.equal(f.settings.open,true,f.host.querySelector('.onboarding-error')?.textContent);await f.tick();assert.equal(f.host.hidden,true);assert.equal(f.profile.disposed,1);
  if(cancel)f.c.window.closeOnboarding({host:f.host});f.settings.close();await f.tick();await f.tick();assert.equal(f.host.hidden,cancel);assert.equal(f.c.window.onboardingIsOpen(),!cancel);if(!cancel){assert.equal(f.c.window.onboardingRoot(),f.host);assert.equal(f.profile.disposed,1);}
 }
});
test('closing and finishing inline setup dispose live private components and leave progress intact',async()=>{
 for(const step of ['integrations','company']){const f=onboardingHostFixture({step});await f.c.window.openOnboarding({host:f.host});const before=JSON.stringify(f.progress);f.c.window.closeOnboarding({host:f.host});assert.equal(JSON.stringify(f.progress),before);assert.equal(step==='company'?f.profile.disposed:f.connection.disposed,1);}
 const f=onboardingHostFixture({step:'finish'});await f.c.window.openOnboarding({host:f.host});f.document.querySelector('#onboarding-continue').onclick();await f.tick();assert.equal(f.host.hidden,true);assert.equal(f.c.window.onboardingIsOpen(),false);assert.equal(f.progress.step,'finish');
});
test('skill and schedule child dialogs return to the same inline host without embedding a dialog',async()=>{
 for(const kind of ['skill','schedule']){
  const f=onboardingHostFixture({step:'skills'});f.c.skillPickerMatches=()=>[];const child=new f.Element('dialog');child.id=kind==='skill'?'skill-editor':'schedule-editor';f.body.append(child);
  if(kind==='skill'){
   const form=new f.Element('form');form.id='skill-editor-form';form.elements=Object.fromEntries(['title','skill','agent','command','cwd'].map(key=>[key,{value:''}]));form.reset=()=>{for(const value of Object.values(form.elements))value.value='';form.elements.agent.value='claude';};const error=new f.Element('p');error.id='skill-editor-error';f.body.append(form,error);f.c.setting=(key,fallback)=>key==='defaultAgent'?'codex':fallback;
  }else{
   f.progress.steps.skills='pending';const original=f.c.api;f.c.api=async(path,payload)=>path==='/api/library'?{presets:[{id:'sample-skill',title:'Sample skill'}]}:original(path,payload);f.c.editSchedule=()=>child.showModal();
  }
  await f.c.window.openOnboarding({host:f.host});const launch=f.host.querySelectorAll('button').find(node=>node.textContent===(kind==='skill'?'Enter a skill path':'Configure selected skill schedule'));assert.ok(launch);launch.onclick();for(let spin=0;!child.open&&spin<20;spin++)await f.tick();assert.equal(child.open,true);assert.equal(f.host.hidden,true);assert.equal(f.host.querySelectorAll('dialog').length,0);
  if(kind==='skill')assert.equal(f.document.querySelector('#skill-editor-form').elements.agent.value,'codex');await f.tick();child.close();await f.tick();await f.tick();assert.equal(f.c.window.onboardingIsOpen(),true);assert.equal(f.c.window.onboardingRoot(),f.host);assert.equal(f.host.hidden,false);
 }
});
test('reopening a shell after interrupted confirmation resets busy controls and ignores the old action finally',async()=>{
 const f=onboardingHostFixture(),original=f.c.api;let release;
 f.c.api=async(path,payload)=>path==='/api/settings'&&!payload?new Promise(resolve=>release=resolve):original(path,payload);
 await f.c.window.openOnboarding({host:f.host});const footer=f.document.querySelector('#onboarding-continue');footer.onclick();for(let spin=0;!release&&spin<20;spin++)await f.tick();assert.ok(release);assert.equal(footer.disabled,true);assert.equal(f.host.attrs['aria-busy'],'true');
 f.c.window.closeOnboarding({host:f.host});await f.c.window.openOnboarding({host:f.host});assert.equal(f.document.querySelector('#onboarding-continue'),footer);assert.equal(footer.disabled,false);assert.equal(f.host.attrs['aria-busy'],'false');assert.equal(f.host.querySelector('[data-onboarding-close]').disabled,false);
 release({revision:2,contentHash:'a'.repeat(64),values:{}});await f.tick();assert.equal(footer.disabled,false);assert.equal(f.host.attrs['aria-busy'],'false');assert.equal(f.calls.some(call=>call.path==='/api/settings'&&call.payload),false);
});

test('company prose explicitly prepares an unsent draft, clears the source and closes without changing progress',async()=>{
 const f=onboardingHostFixture({step:'company'}),drafts=[];f.c.window.prepareAppChatMessage=text=>drafts.push(text);await f.c.window.openOnboarding({host:f.host});const input=f.host.querySelector('textarea'),review=f.host.querySelectorAll('button').find(node=>node.textContent==='Review in App chat');input.value='Use our review guide.\nCheck sharing access in Settings.';const calls=f.calls.length,progress=JSON.stringify(f.progress);
 assert.equal(input.attrs['data-private'],'true');assert.equal(input.attrs['data-agent-private'],'true');assert.equal(input.closest('.onboarding-company-instructions').attrs['data-agent-context-exclude'],'true');review.onclick();await f.tick();assert.equal(drafts.length,1);assert.ok(drafts[0].endsWith('Use our review guide.\nCheck sharing access in Settings.'));assert.match(drafts[0],/Do not apply settings or run commands automatically/);assert.equal(input.value,'');assert.equal(f.host.hidden,true);assert.equal(f.calls.length,calls);assert.equal(JSON.stringify(f.progress),progress);
});
test('rejected company prose transfer preserves source, existing draft and open setup for retry',async()=>{
 for(const reason of ['A reply is still pending.','An existing chat draft is not empty.','Remove attachments first.']){
  const f=onboardingHostFixture({step:'company'});let existing='Keep this existing draft';f.c.window.prepareAppChatMessage=()=>{throw new Error(reason);};await f.c.window.openOnboarding({host:f.host});const input=f.host.querySelector('textarea');input.value='Private team instructions';const calls=f.calls.length;f.host.querySelectorAll('button').find(node=>node.textContent==='Review in App chat').onclick();await f.tick();assert.equal(input.value,'Private team instructions');assert.equal(existing,'Keep this existing draft');assert.equal(f.host.hidden,false);assert.equal(f.c.window.onboardingIsOpen(),true);assert.equal(f.host.querySelector('.onboarding-error').textContent,reason);assert.equal(f.calls.length,calls);
 }
});
test('company prose passes the complete message to helper byte validation without truncation; blank text never transfers',async()=>{
 const f=onboardingHostFixture({step:'company'}),messages=[];f.c.window.prepareAppChatMessage=text=>{messages.push(text);if(new TextEncoder().encode(text).length>4000)throw new Error('The prepared message exceeds 4000 bytes.');};await f.c.window.openOnboarding({host:f.host});const input=f.host.querySelector('textarea'),review=f.host.querySelectorAll('button').find(node=>node.textContent==='Review in App chat');
 input.value='   ';review.onclick();await f.tick();assert.equal(messages.length,0);assert.equal(f.host.hidden,false);input.value='é'.repeat(2100);review.onclick();await f.tick();assert.equal(messages.length,1);assert.ok(messages[0].endsWith(input.value));assert.equal(input.value.length,2100);assert.equal(f.host.hidden,false);assert.match(f.host.querySelector('.onboarding-error').textContent,/4000 bytes/);
});
test('company instructions clear on Close, Back, Continue and host switch; structured profile remains a separate collapsed alternative',async()=>{
 for(const action of ['close','back','continue','switch']){
  const f=onboardingHostFixture({step:'company'});f.c.skillPickerMatches=()=>[];await f.c.window.openOnboarding({host:f.host});const input=f.host.querySelector('textarea'),alternative=f.host.querySelectorAll('details').find(node=>node.children[0]?.textContent==='Or import a settings profile');assert.ok(alternative);assert.equal(alternative.open,false);assert.ok(alternative.querySelector('.settings-profile-import'));input.value='Private team guide';
  if(action==='close')f.c.window.closeOnboarding({host:f.host});else if(action==='switch')await f.c.window.openOnboarding();else{f.document.querySelector(action==='back'?'#onboarding-back':'#onboarding-continue').onclick();await f.tick();}
  assert.equal(input.value,'');assert.equal(f.profile.disposed,1);
 }
});

test('returning from an enabled schedule and choosing On demand keeps truthful schedule status without disabling it',async()=>{
 const f=onboardingHostFixture({step:'skills'}),child=new f.Element('dialog');child.id='skill-editor';f.body.append(child);
 const form=new f.Element('form');form.id='skill-editor-form';form.elements=Object.fromEntries(['title','skill','agent','command','cwd'].map(key=>[key,{value:''}]));form.reset=()=>{};const error=new f.Element('p');error.id='skill-editor-error';f.body.append(form,error);
 let library={presets:[]};const original=f.c.api;f.c.api=async(path,payload)=>path==='/api/library'?library:original(path,payload);f.c.skillPickerMatches=()=>[];
 f.c.state={skillSchedules:{}};await f.c.window.openOnboarding({host:f.host});
 const availability=()=>f.host.querySelectorAll('select').find(node=>node.attrs['aria-label']==='Skill availability');availability().value='schedule';availability().onchange();await f.tick();
 f.host.querySelectorAll('button').find(node=>node.textContent==='Enter a skill path').onclick();for(let spin=0;!child.open&&spin<20;spin++)await f.tick();assert.equal(child.open,true);
 f.c.lastSkillEditor.onSaved('new-skill');library={presets:[{id:'wrong-same-title',title:'Research notes'},{id:'new-skill',title:'Research notes'}]};f.c.state.skillSchedules['new-skill']={enabled:false,mode:'daily',time:'09:30'};child.close();await f.tick();await f.tick();
 const scheduleChild=new f.Element('dialog');scheduleChild.id='schedule-editor';f.body.append(scheduleChild);f.c.editSchedule=()=>scheduleChild.showModal();
 f.host.querySelectorAll('button').find(node=>node.textContent==='Configure schedule').onclick();for(let spin=0;!scheduleChild.open&&spin<20;spin++)await f.tick();assert.equal(scheduleChild.open,true);f.c.state.skillSchedules['new-skill'].enabled=true;scheduleChild.close();await f.tick();await f.tick();
 availability().value='on-demand';availability().onchange();await f.tick();
 const text=f.host.querySelectorAll('p').map(node=>node.textContent).join('\n');assert.match(text,/Schedule enabled: Daily at 09:30/);assert.doesNotMatch(text,/No schedule (?:was|is) enabled/);
 assert.equal(f.c.state.skillSchedules['new-skill'].enabled,true);assert.ok(f.host.querySelectorAll('button').some(node=>node.textContent==='Configure schedule'));assert.equal(f.calls.some(call=>call.path==='/api/skills/schedule'),false);
});

function localChoiceFocusFixture(){
 const document={body:{tagName:'BODY'},activeElement:null};document.activeElement=document.body;
 class Element{
  constructor(tag,text='',cls=''){Object.assign(this,{tagName:tag.toUpperCase(),textContent:text||'',className:cls,children:[],attrs:{},dataset:{},_disabled:false});}
  append(...children){this.children.push(...children);}
  setAttribute(key,value){this.attrs[key]=value;}
  closest(){return null;}
  focus(){if(!this.disabled)document.activeElement=this;}
  set disabled(value){this._disabled=value;if(value&&document.activeElement===this)document.activeElement=document.body;}
  get disabled(){return this._disabled;}
 }
 const content=new Element('main'),back=new Element('button','Back'),close=new Element('button','Close'),next=new Element('button','Continue'),nodes=new Map([['#onboarding-back',back],['#onboarding-continue',next]]),calls=[];
 const extraControls=[],controls=()=>[back,close,next,...extraControls,...content.children.flatMap(node=>node.children)];
 const root={open:true,attrs:{},setAttribute(key,value){this.attrs[key]=value;},querySelectorAll:controls,querySelector:()=>close};
 const c=onboardingTestContext({document,node:(...args)=>new Element(...args),content,dialog:root,actionPending:false,documentState:{progress:{step:'assistant'}},$:id=>nodes.get(id),showError:error=>calls.push(['error',error.message]),preference:async patch=>calls.push(['preference',patch]),next:async()=>calls.push(['next'])});
 vm.runInContext(source.slice(source.indexOf(' async function runAction('),source.indexOf(' const showError=')),c);
 vm.runInContext(source.slice(source.indexOf(' function paragraph('),source.indexOf(' function choices(')),c);
 // The real footer factory keeps the existing serialized Continue owner.
 vm.runInContext("globalThis.commitButton=button('Continue',()=>continueAction(),true)",c);extraControls.push(c.commitButton);
 return {c,document,root,content,next,calls,group:()=>content.children.find(node=>node.attrs.role==='group')};
}
test('local assistant and editor choices preserve native keyboard focus and commit only from Continue',async()=>{
 for(const [key,items]of [['defaultAgent',[{id:'claude',label:'Claude'},{id:'codex',label:'Codex'}]],['editor',[{id:'zed',label:'Zed'},{id:'cursor',label:'Cursor'}]]]){
  const f=localChoiceFocusFixture();f.c.confirmChoice(items,items[0].id,key,'Choose a tool');const group=f.group(),choice=group.children[1];choice.focus();choice.onclick();await Promise.resolve();
  assert.equal(f.document.activeElement,choice);assert.equal(choice.disabled,false);assert.equal(choice.attrs['aria-pressed'],'true');assert.equal(group.children[0].attrs['aria-pressed'],'false');assert.equal(f.next.textContent,'Use '+items[1].label);assert.ok(group.className.includes('ui-segmented'));assert.equal(f.root.attrs['aria-busy'],undefined);assert.equal(f.c.actionPending,false);assert.equal(f.calls.length,0);
  let finish;f.c.preference=async patch=>{f.calls.push(['preference',patch]);await new Promise(resolve=>finish=resolve);};f.c.commitButton.onclick();assert.equal(f.c.actionPending,true);assert.equal(choice.disabled,true);assert.equal(f.c.commitButton.disabled,true);assert.equal(f.calls[0][1][key],items[1].id);f.c.commitButton.onclick();assert.equal(f.calls.length,1);finish();await new Promise(resolve=>setImmediate(resolve));assert.equal(f.c.actionPending,false);assert.equal(choice.disabled,false);assert.deepEqual(f.calls.at(-1),['next']);
 }
});
test('local choices ignore pending, read-only and obsolete setup handlers without saving or changing the footer',()=>{
 for(const mode of ['pending','read-only','closed','reopened']){
  const f=localChoiceFocusFixture();f.c.confirmChoice([{id:'claude',label:'Claude'},{id:'codex',label:'Codex'}],'claude','defaultAgent','Choose a tool');const group=f.group();if(mode==='pending')f.c.actionPending=true;if(mode==='read-only')f.c.documentState.progressReadOnly=true;if(mode==='closed')f.root.open=false;if(mode==='reopened')f.c.openGeneration++;
  group.children[1].onclick();assert.equal(group.children[0].attrs['aria-pressed'],'true',mode);assert.equal(group.children[1].attrs['aria-pressed'],'false',mode);assert.equal(f.next.textContent,'Use Claude',mode);assert.equal(f.calls.length,0,mode);
 }
});

function pauseSetupPreference(f){
 const original=f.c.api;let resolve,reject;const settled=new Promise((a,b)=>{resolve=a;reject=b;});let writes=0;
 f.c.api=async(path,payload)=>{if(path==='/api/settings'&&payload){writes++;await settled;}return original(path,payload);};
 return {resolve,reject,writes:()=>writes};
}
async function openFocusSetup(f,inline){
 if(inline){const appchat=new f.Element('div');appchat.id='app-chat-panel';f.body.append(appchat);appchat.append(f.host);}
 await f.c.window.openOnboarding(inline?{host:f.host}:{});
 const root=f.c.window.onboardingRoot();return {root,holder:root.querySelector('.onboarding-heading h2'),content:root.querySelector('.onboarding-content'),continue:root.querySelector('#onboarding-continue')};
}
test('actual async Continue holds focus before native disabling and moves to the new step heading in modal and inline setup',async()=>{
 for(const inline of [false,true]){
  const f=onboardingHostFixture(),ui=await openFocusSetup(f,inline),write=pauseSetupPreference(f);ui.continue.focus();ui.continue.onclick();assert.equal(f.document.activeElement,ui.holder);assert.equal(ui.continue.disabled,true);assert.equal(ui.holder.tabIndex,-1);ui.continue.onclick();await f.tick();assert.equal(write.writes(),1);write.resolve();await f.tick();
  const heading=ui.content.querySelector('h3');assert.equal(heading.textContent,'Make yourself at home');assert.equal(f.document.activeElement,heading);assert.equal(heading.tabIndex,-1);assert.equal(heading.focusOptions.preventScroll,false);assert.equal(ui.continue.disabled,false);assert.equal(f.calls.filter(call=>call.path==='/api/settings'&&call.payload).length,1);
 }
});
test('actual same-step theme saves and rejections restore the exact enabled selector without resetting its draft',async()=>{
 for(const inline of [false,true])for(const rejected of [false,true]){
  const f=onboardingHostFixture({step:'theme'}),ui=await openFocusSetup(f,inline),write=pauseSetupPreference(f),select=ui.content.querySelector('select');select.value='midnight';select.focus();select.onchange();assert.equal(f.document.activeElement,ui.holder);assert.equal(select.disabled,true);await f.tick();if(rejected)write.reject(Error('Settings changed. Review your choice.'));else write.resolve();await f.tick();
  assert.equal(f.document.activeElement,select);assert.equal(select.disabled,false);assert.equal(select.value,'midnight');assert.equal(select.focusOptions.preventScroll,true);assert.equal(write.writes(),1);if(rejected)assert.match(ui.root.querySelector('.onboarding-error').textContent,/Settings changed/);
 }
});
test('async setup completion never steals outside, terminal or replacement-dialog focus',async()=>{
 for(const inline of [false,true])for(const where of ['input','terminal','dialog']){
  const f=onboardingHostFixture(),ui=await openFocusSetup(f,inline),write=pauseSetupPreference(f);ui.continue.focus();ui.continue.onclick();await f.tick();const outside=new f.Element('input');if(where==='dialog'){f.settings.append(outside);f.settings.showModal();}else if(where==='terminal'){const terminal=new f.Element('div');terminal.className='xterm';terminal.append(outside);f.body.append(terminal);}else f.body.append(outside);outside.focus();write.resolve();await f.tick();assert.equal(f.document.activeElement,outside,inline+' '+where);
 }
});
test('an action started with outside focus never acquires setup focus even when the new step paints',async()=>{
 for(const inline of [false,true]){const f=onboardingHostFixture(),ui=await openFocusSetup(f,inline),write=pauseSetupPreference(f),outside=new f.Element('input');f.body.append(outside);outside.focus();ui.continue.onclick();await f.tick();write.resolve();await f.tick();assert.equal(f.document.activeElement,outside);assert.equal(ui.content.querySelector('h3').textContent,'Make yourself at home');}
});
test('old async setup completion cannot restore focus after close, same-host reopen or a child Settings handoff',async()=>{
 for(const inline of [false,true])for(const mode of ['close','reopen']){
  const f=onboardingHostFixture(),ui=await openFocusSetup(f,inline),write=pauseSetupPreference(f);ui.continue.focus();ui.continue.onclick();await f.tick();f.c.window.closeOnboarding();if(mode==='reopen'){await f.c.window.openOnboarding(inline?{host:f.host}:{});f.c.window.onboardingRoot().querySelector('#onboarding-continue').focus();}else{const outside=new f.Element('input');f.body.append(outside);outside.focus();}const active=f.document.activeElement;write.resolve();await f.tick();assert.equal(f.document.activeElement,active,inline+' '+mode);
 }
 const f=onboardingHostFixture({step:'company'}),ui=await openFocusSetup(f,true),sharing=ui.content.querySelectorAll('button').find(button=>button.textContent==='Open sharing setup');sharing.focus();sharing.onclick();await f.tick();const childInput=new f.Element('input');f.settings.append(childInput);childInput.focus();await f.tick();assert.equal(f.settings.open,true);assert.equal(f.host.hidden,true);assert.equal(f.document.activeElement,childInput);
});
test('a removed same-step control falls back to its heading only while the action holder still owns focus',async()=>{
 const f=onboardingHostFixture({step:'theme'}),ui=await openFocusSetup(f,true),write=pauseSetupPreference(f),select=ui.content.querySelector('select');select.focus();select.onchange();await f.tick();const heading=ui.content.querySelector('h3');ui.content.replaceChildren(heading);write.resolve();await f.tick();assert.equal(select.isConnected,false);assert.equal(f.document.activeElement,heading);assert.equal(heading.tabIndex,-1);
});
test('a newly opened modal blocks settling even if browser focus still reports the setup holder',async()=>{
 const f=onboardingHostFixture({step:'theme'}),ui=await openFocusSetup(f,true),write=pauseSetupPreference(f),select=ui.content.querySelector('select');select.focus();select.onchange();await f.tick();f.settings.showModal();assert.equal(f.document.activeElement,ui.holder);write.resolve();await f.tick();assert.equal(f.document.activeElement,ui.holder);assert.equal(select.disabled,false);
});

test('setup navigation veto preserves the existing guarded modal before any loading or shell disposal',async()=>{
 const f=onboardingHostFixture();f.settings.showModal();f.settings.addEventListener('cancel',event=>event.preventDefault());const input=new f.Element('input');f.settings.append(input);input.value='Private pending draft';input.focus();
 assert.equal(await f.c.window.openOnboarding({host:f.host}),false);assert.equal(f.calls.length,0);assert.equal(f.settings.open,true);assert.equal(f.document.activeElement,input);assert.equal(input.value,'Private pending draft');assert.equal(f.host.children.length,0);
});
test('setup awaited progress/settings reads never show over a newly opened guarded dialog',async()=>{
 for(const phase of ['progress','settings']){
  const f=onboardingHostFixture(),original=f.c.api;let release,settingsCommits=0;const pause=new Promise(resolve=>release=resolve);
  f.c.api=async(path,payload)=>{const response=await original(path,payload);if(path==='/api/onboarding'&&phase==='progress')await pause;return response;};
  f.c.loadAppSettings=async({canApply})=>{if(phase==='settings')await pause;if(canApply())settingsCommits++;};
  const pending=f.c.window.openOnboarding({host:f.host});await f.tick();f.settings.showModal();f.settings.addEventListener('cancel',event=>event.preventDefault());const input=new f.Element('input');f.settings.append(input);input.focus();release();assert.equal(await pending,false);assert.equal(settingsCommits,0);assert.equal(f.settings.open,true);assert.equal(f.document.activeElement,input);assert.equal(f.host.hidden,true);
 }
});

test('setup opening rejects queued empty-modal ABA and disconnects its admission observer',async()=>{
 const f=onboardingHostFixture(),original=f.c.api;let release,commits=0;const pause=new Promise(resolve=>release=resolve);f.c.api=async(path,payload)=>{const response=await original(path,payload);if(path==='/api/onboarding')await pause;return response;};f.c.loadAppSettings=async({canApply})=>{if(canApply())commits++;};
 const pending=f.c.window.openOnboarding({host:f.host});await f.tick();f.settings.showModal();f.settings.close();release();assert.equal(await pending,false);assert.equal(commits,0);assert.equal(f.host.hidden,true);assert.equal(f.c.MutationObserver.activeCount,0);
});

test('refused or stale Settings handoff restores setup only while its original page and modal ownership survive',async()=>{
 for(const mode of ['failed','existing-settings','other-modal','page']){
  const f=onboardingHostFixture({step:'company'});await f.c.window.openOnboarding({host:f.host});let resolve,options;f.c.openPreferences=value=>{options=value;return new Promise(yes=>resolve=yes);};const sharing=f.host.querySelectorAll('button').find(node=>node.textContent==='Open sharing setup');sharing.onclick();for(let spin=0;!resolve&&spin<20;spin++)await f.tick();assert.ok(resolve);assert.equal(options.category,'sharing');assert.equal(f.host.hidden,true);assert.equal(f.c.settingsCategory,'appearance');let outside;
  if(mode==='existing-settings'){f.settings.showModal();outside=new f.Element('input');f.settings.append(outside);outside.focus();}if(mode==='other-modal'){const modal=new f.Element('dialog');f.body.append(modal);modal.showModal();outside=new f.Element('input');modal.append(outside);outside.focus();}if(mode==='page')f.c.source='notes';resolve(false);await f.tick();await f.tick();assert.equal(f.host.hidden,mode!=='failed');assert.equal(f.c.settingsCategory,'appearance');if(outside)assert.equal(f.document.activeElement,outside);if(mode==='failed')assert.match(f.host.querySelector('.onboarding-error').textContent,/Settings could not open.*progress is saved/);if(mode==='existing-settings'){f.settings.close();await f.tick();assert.equal(f.host.hidden,true,'Refusal must not attach returnAfter to an unrelated already-open Settings lifetime');}
 }
});
