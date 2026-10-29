const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require.resolve('../static/workspace.js'),'utf8');
function fn(name,next,globals={}){const start=source.indexOf('function '+name+'('),end=source.indexOf('function '+next+'(',start);const context=vm.createContext(globals);vm.runInContext(source.slice(start,end),context);return context[name];}
test('CSV results preserve escaped quotes, quoted newlines, CRLF and empty cells',()=>{const parse=fn('parseCSV','appendResultTable');assert.equal(JSON.stringify(parse('name,value\r\n"a,b","two\nlines"\r\n"quote ""here""",\r\n')),JSON.stringify([['name','value'],['a,b','two\nlines'],['quote "here"','']]));assert.throws(()=>parse('a\n"unfinished'),/unterminated/);});
test('active bar excludes closed and journal-only records while retaining verified live agents',()=>{const filter=fn('liveCandidates','recordForCandidate',{inventory:{candidates:[{id:1,live:false,running:true,active:true},{id:2,live:true},{id:3,running:true,active:false},{id:4,running:true,active:true}]}});assert.equal(JSON.stringify(filter().map(x=>x.id)),JSON.stringify([2,4]));});
test('agent workspace does not inflate direct attachments through related bundles',()=>{
  const items=[{id:'one',title:'Direct'},{id:'two',title:'Related'}],record={items:[{id:'one'}]};
  const work=fn('agentWork','renderAgentWorkspace',{state:{relatedItems:[],bundles:[{itemIds:['one','two']}]},allItems:()=>items});
  assert.deepEqual(Array.from(work(record),x=>x.id),['one']);
});
test('runtime labels retain unknown counts and do not trust stale lifecycle reports',()=>{const info=fn('runtimeInfo','workSummary');assert.equal(info({lifecycle:'idle'},{runtime:{state:'working',subagents:2,at:0}}).label,'Waiting');assert.equal(info({lifecycle:'idle'},null).subagents,null);assert.equal(info({lifecycle:'running'},{runtime:{state:'needs-input',subagents:3,at:Date.now()/1000}}).subagents,3);});

test('embedded lifecycle stays known while the same process is alive and resets for a newer process',()=>{const info=fn('runtimeInfo','workSummary');const report={runtime:{state:'waiting',subagents:0,at:200}};assert.equal(info({host:'embedded',startedAt:100},report).label,'Waiting');assert.equal(info({host:'embedded',startedAt:300},report).kind,'unknown');});

test('attention dots exclude active work and monitored shells while status includes their counts',()=>{
  const info=fn('runtimeInfo','workSummary'),candidate={host:'embedded',startedAt:100};
  const report=state=>({runtime:{state,subagents:0,at:200}});
  assert.equal(info(candidate,report('working')).attention,false);
  assert.equal(info(candidate,report('waiting')).attention,true);
  assert.equal(info(candidate,report('needs-input')).attention,true);
  const background=report('waiting');Object.assign(background.runtime,{backgroundCounts:{shell:2,monitor:1,subagent:3},scheduledWakeups:1});
  const value=info(candidate,background);assert.equal(value.attention,false);assert.equal(value.label,'Background');assert.equal(value.subagents,3);assert.match(value.summary,/2 shells/);assert.match(value.summary,/1 monitor/);assert.match(value.summary,/1 timer/);
  background.runtime.state='needs-input';assert.equal(info(candidate,background).attention,true);
  assert.equal(info({...candidate,startedAt:300},background).attention,false);
});

test('title bar sheds the app title before notifications as agent tabs fill it',()=>{
  let width=1100,count=4;const flags={};
  const context=vm.createContext({$:()=>({get clientWidth(){return width;},classList:{toggle:(key,value)=>flags[key]=value}}),visibleAgentCandidates:()=>Array(count).fill({}),liveCandidates:()=>[],isAgentSnoozed:()=>false,serviceClient:{isDesktop:true}});
  vm.runInContext(source.slice(source.indexOf('function updateTabDensity('),source.indexOf('const titlebarObserver=')),context);
  context.updateTabDensity();assert.equal(flags['tabs-condensed'],false);assert.equal(flags['tabs-crowded'],false);
  width=800;context.updateTabDensity();assert.equal(flags['tabs-condensed'],true);assert.equal(flags['tabs-crowded'],false);
  count=8;context.updateTabDensity();assert.equal(flags['tabs-crowded'],true);
  count=1;context.updateTabDensity();assert.equal(flags['tabs-condensed'],false);assert.equal(flags['tabs-crowded'],false);
});
test('closing a tab targets its process generation and retains saved conversation data',async()=>{
  const calls=[],candidate={host:'embedded',recordId:'one',generation:'process-one'};
  const state={sessions:[{id:'one',title:'Saved conversation'}]};
  const context=vm.createContext({api:async(path,payload)=>calls.push({path,payload}),inventory:{candidates:[candidate,{host:'embedded',recordId:'two',generation:'process-two'}]},selectedAgentKey:'embedded:one',candidateKey:c=>'embedded:'+c.recordId,persist(){},render(){},loadInventory:async()=>{},rememberClosedTab(){},toast(){},state});
  vm.runInContext(source.slice(source.indexOf('async function closeAgentTab('),source.indexOf('function updateTabDensity(')),context);
  await context.closeAgentTab(candidate,{disabled:false});assert.equal(calls[0].path,'/api/terminal/close');assert.equal(calls[0].payload.generation,'process-one');assert.equal(calls[0].payload.id,'one');assert.equal(context.inventory.candidates[0].recordId,'two');assert.equal(state.sessions.length,1);
});

test('reopening closed tabs uses a persisted LIFO stack and queues repeated presses',async()=>{
  const opened=[],live=new Set(),saved=[];let release;
  const gate=new Promise(resolve=>release=resolve);
  const c=vm.createContext({document:{querySelector:()=>null},state:{sessions:[{id:'one'},{id:'two'},{id:'three'}]},candidateForRecord:r=>live.has(r.id)?{}:null,selectAgent(){},openSavedSession:async r=>{opened.push(r.id);if(r.id==='three')await gate;live.add(r.id);},loadInventory:async()=>{},persist(){saved.push(true);},toast(){}});
  vm.runInContext('let closedAgentTabs=["one","two","three"],reopeningTab=false,reopenRequests=0;'+source.slice(source.indexOf('function rememberClosedTab('),source.indexOf('async function closeAgentTab(')),c);
  const first=c.runTabCommand('reopen');await c.runTabCommand('reopen');await c.runTabCommand('reopen');release();await first;
  assert.deepEqual(opened,['three','two','one']);assert.equal(saved.length,3);
});

test('saved GitHub subviews retain independent authored/review PR roles',()=>{
 const c=vm.createContext({TextEncoder,localStorage:{getItem:()=>null},prefs:{}});vm.runInContext(fs.readFileSync(require.resolve('../static/views.js'),'utf8'),c);
 const views=JSON.parse(fs.readFileSync(require.resolve('../static/config/views.json'),'utf8')).views,mine=views.find(view=>view.id==='github-mine'),review=views.find(view=>view.id==='github-review');
 const item={source:'github',kind:'Pull request',relations:['authored','reviews'],status:'todo'};
 assert.equal(c.savedViewMatches(mine,item),true);assert.equal(c.savedViewMatches(review,item),true);
 for(const view of [mine,review]){assert.equal(c.savedViewMatches(view,{...item,relations:['assigned']}),false);assert.equal(c.savedViewMatches(view,{...item,kind:'Issue'}),false);assert.equal(c.savedViewMatches(view,{...item,status:'done'}),false);}
});

test('Command-W dismisses an inline new-session picker before considering a live terminal',async()=>{
  const options={open:true},prompt={hidden:false};let focused=0,closed=0;
  const c=vm.createContext({document:{querySelector:()=>null},$:id=>id==='#launch-options'?options:prompt,liveCandidates:()=>[{recordId:'one'}],candidateKey:()=> 'one',selectedAgentKey:'one',terminalViews:new Map([['one',{terminal:{focus(){focused++;}}}]]),closeAgentTab(){closed++;}});
  vm.runInContext(source.slice(source.indexOf('async function runTabCommand('),source.indexOf('async function closeAgentTab(')),c);
  await c.runTabCommand('close');assert.equal(options.open,false);assert.equal(prompt.hidden,true);assert.equal(closed,0);assert.equal(focused,1);
});

test('Shift-Enter sends one multiline sequence while preserving normal submit, app shortcuts and IME',()=>{
  const handle=fn('terminalMultilineKey','ensureTerminal'),sent=[];const terminal={input:(data,user)=>sent.push({data,user})};let prevented=0;
  const event=(extra={})=>({key:'Enter',shiftKey:true,type:'keydown',preventDefault(){prevented++;},...extra});
  assert.equal(handle(terminal,event()),false);assert.deepEqual(sent,[{data:'\x1b\r',user:true}]);
  assert.equal(handle(terminal,event({type:'keyup'})),false);assert.equal(sent.length,1);
  for(const extra of [{shiftKey:false},{metaKey:true},{ctrlKey:true},{altKey:true},{isComposing:true},{key:'a'}])assert.equal(handle(terminal,event(extra)),true);
  assert.equal(sent.length,1);assert.equal(prevented,2);
});

function snoozeContext(extra={}){
  const candidate={host:'embedded',recordId:'one',title:'First'},other={host:'embedded',recordId:'two',title:'Second'};
  const c=vm.createContext({window:{},source:'agents',pendingSessionResumes:new Set(),document:{querySelector:()=>null},snoozedAgentTabs:{},selectedAgentKey:'embedded:one',agentRename:null,inventory:{candidates:[candidate,other]},state:{sessions:[{id:'one',title:'First'}]},candidateKey:c=>'embedded:'+c.recordId,recordForCandidate:c=>({id:c.recordId,title:c.title}),liveCandidates:()=>[candidate,other],persist(){},render(){},toast(){},...extra});
  vm.runInContext(source.slice(source.indexOf('let agentNavigationEpoch='),source.indexOf('function selectAgent(')),c);vm.runInContext(source.slice(source.indexOf('function isAgentSnoozed('),source.indexOf('function runtimeInfo(')),c);return {c,candidate,other};
}
test('snoozing only changes presentation, keeps all terminals live and removes visible shortcut slots',()=>{
  let saved=0;const {c,candidate}=snoozeContext({persist(){saved++;}});
  c.snoozeAgentTab(candidate);assert.equal(c.selectedAgentKey,'embedded:two');assert.equal(c.snoozedAgentTabs['embedded:one'].recordId,'one');assert.equal(saved,1);
  assert.deepEqual(Array.from(c.visibleAgentCandidates(),x=>x.recordId),['two']);assert.equal(c.liveCandidates().length,2);
  c.snoozeAgentTab(c.liveCandidates()[1]);assert.equal(c.visibleAgentCandidates().length,0);
});
test('restoring a live snoozed agent selects it without launching another process',async()=>{
  let selected=0;const menu={open:true};const {c,candidate}=snoozeContext({$:()=>menu,selectAgent(record,value){assert.equal(value.recordId,'one');selected++;},openSavedSession(){assert.fail('Live agents must not be relaunched');}});
  c.snoozeAgentTab(candidate);await c.restoreSnoozedTab('embedded:one',{});assert.equal(selected,1);assert.equal(menu.open,false);
});
test('restoring a saved snoozed chat resumes its record and retains it if launch fails',async()=>{
  const menu={open:true},button={disabled:false};let opened=0,toasted='';
  const {c,candidate}=snoozeContext({$:()=>menu,liveCandidates:()=>[],candidateForRecord:()=>null,openSavedSession:async record=>{opened++;assert.equal(record.id,'one');throw new Error('Launch failed');},toast:message=>toasted=message});
  c.snoozeAgentTab(candidate);await c.restoreSnoozedTab('embedded:one',button);assert.equal(opened,1);assert.equal(button.disabled,false);assert.equal(toasted,'Launch failed');assert.ok(c.snoozedAgentTabs['embedded:one']);assert.equal(menu.open,true);
});
test('selecting an agent through a work link restores its snoozed tab',()=>{
  let saved=0,rendered=0;const c=vm.createContext({source:'agents',selectedAgentKey:'',agentRename:null,snoozedAgentTabs:{'embedded:one':{at:1}},candidateKey:c=>'embedded:'+c.recordId,persist(){saved++;},render(){rendered++;},$:()=>null});
  vm.runInContext(source.slice(source.indexOf('let agentNavigationEpoch='),source.indexOf('async function attachCandidate(')),c);
  c.selectAgent({id:'one'},{host:'embedded',recordId:'one'});assert.equal(Object.keys(c.snoozedAgentTabs).length,0);assert.equal(c.selectedAgentKey,'embedded:one');assert.equal(saved,1);assert.equal(rendered,1);
});

function tabKeyboardFixture(){
 const candidates=[{recordId:'one'},{recordId:'three'},{recordId:'four'}],selected=[],focused=[],events={};let blocked='';
 const node=(tag,cls='',parent=null)=>({tag,cls,parent,matches(selector){return selector==='textarea.xterm-helper-textarea'&&this.tag==='textarea'&&this.cls.split(' ').includes('xterm-helper-textarea');},closest(selector){for(let current=this;current;current=current.parent)if(selector==='.xterm'&&current.cls.split(' ').includes('xterm'))return current;return null;}});
 const body=node('body'),terminal=node('div','xterm',body),input=node('textarea','xterm-helper-textarea',terminal);
 const document={body,querySelector:selector=>(blocked==='dialog'&&selector.includes('dialog[open]')||['launch-options','snoozed-agents'].includes(blocked)&&selector.includes('#agent-topbar details[open]')||blocked==='agent-menu'&&selector.includes('.agent-menu[open]'))?{}:null,addEventListener:(name,handler)=>events[name]=handler};
 const c=vm.createContext({source:'agents',selectedAgentKey:'one',document,navigator:{platform:'MacIntel'},appSettings:{},workSearchSources:new Set(),visibleAgentCandidates:()=>candidates,candidateKey:x=>x.recordId,recordForCandidate:x=>x,selectAgent:(r,x)=>{selected.push(x.recordId);c.selectedAgentKey=x.recordId;},terminalViews:new Map(candidates.map(x=>[x.recordId,{terminal:{focus:()=>focused.push(x.recordId)}}]))});
 vm.runInContext(source.slice(source.indexOf('function cycleAgentTab('),source.indexOf('function updateTabDensity(')),c);
 const begin=source.indexOf("document.addEventListener('keydown',event=>{\n  if(event.isComposing||event.altKey)return;");vm.runInContext(source.slice(begin,source.indexOf("document.addEventListener('click',event=>",begin)),c);
 const press=(target=input,flags={})=>{let prevented=0,stopped=0;events.keydown({key:'Tab',target,preventDefault(){prevented++;},stopImmediatePropagation(){stopped++;},...flags});return {prevented,stopped};};
 return {c,node,body,terminal,input,candidates,selected,focused,press,block:value=>blocked=value};
}
test('actual keydown route cycles chats only from terminal and body origins in both directions with wrap',()=>{
 const f=tabKeyboardFixture();for(const target of [f.input,f.body,f.terminal])assert.deepEqual(f.press(target),{prevented:1,stopped:1});assert.deepEqual(f.selected,['three','four','one']);assert.deepEqual(f.focused,f.selected);
 assert.deepEqual(f.press(f.input,{shiftKey:true}),{prevented:1,stopped:1});assert.equal(f.selected.at(-1),'four');assert.deepEqual(f.press(f.body,{shiftKey:true}),{prevented:1,stopped:1});assert.equal(f.selected.at(-1),'three');
 f.c.selectedAgentKey='missing';f.press(f.input);assert.equal(f.selected.at(-1),'one');f.c.selectedAgentKey='missing';f.press(f.input,{shiftKey:true});assert.equal(f.selected.at(-1),'four');
});
test('actual Tab route preserves native traversal for sidebar, tab actions, toolbar, editable and custom focusable controls',()=>{
 const f=tabKeyboardFixture();const controls=[f.node('button','live-agent',f.body),f.node('button','agent-tab-close',f.body),f.node('button','agent-tab-snooze',f.body),f.node('summary','icon-button',f.body),f.node('a','nav-item',f.body),f.node('button','page-settings-gear',f.body),f.node('input','ui-field',f.body),f.node('textarea','',f.body),f.node('select','',f.body),f.node('div','contenteditable',f.body),f.node('div','custom-focusable',f.body),f.node('button','toolbar-action',f.terminal),f.node('a','',f.terminal),f.node('input','',f.terminal),f.node('textarea','ordinary-editor',f.terminal),f.node('div','custom-focusable',f.terminal),f.node('div','contenteditable',f.terminal),f.node('button','xterm-helper-textarea',f.terminal)];
 for(const control of controls)for(const shiftKey of [false,true]){assert.deepEqual(f.press(control,{shiftKey}),{prevented:0,stopped:0},control.tag+' '+control.cls);const child=f.node('span','icon',control);assert.deepEqual(f.press(child,{shiftKey}),{prevented:0,stopped:0});}assert.deepEqual(f.selected,[]);assert.deepEqual(f.focused,[]);
});
test('actual chat Tab cycling preserves modifier, composition, page, dialog, menu and empty-inventory guards',()=>{
 const f=tabKeyboardFixture();for(const flags of [{metaKey:true},{ctrlKey:true},{altKey:true},{isComposing:true},{key:'x'}])for(const shiftKey of [false,true])assert.deepEqual(f.press(f.input,{...flags,shiftKey}),{prevented:0,stopped:0});
 for(const kind of ['dialog','launch-options','snoozed-agents','agent-menu']){f.block(kind);assert.deepEqual(f.press(f.body),{prevented:0,stopped:0});assert.deepEqual(f.press(f.input,{shiftKey:true}),{prevented:0,stopped:0});}f.block('');f.c.source='github';assert.deepEqual(f.press(f.input),{prevented:0,stopped:0});f.c.source='agents';f.candidates.length=0;assert.deepEqual(f.press(f.input),{prevented:0,stopped:0});assert.deepEqual(f.selected,[]);assert.deepEqual(f.focused,[]);
});

test('agent tabs show an evidenced attention reason without guessing from an idle process',()=>{
  const info=fn('runtimeInfo','workSummary'),candidate={host:'embedded',startedAt:100};
  for(const [reason,label]of [['finished','Finished'],['approval','Approval'],['question','Question'],['failed','Failed']])assert.equal(info(candidate,{runtime:{state:'waiting',reason,at:200}}).label,label);
  assert.equal(info(candidate,{runtime:{state:'waiting',at:200}}).label,'Waiting');
  assert.equal(info(candidate,{runtime:{state:'working',reason:'failed',at:200}}).label,'Working');
});

test('inventory waiters share the pending request and a fresh request waits for it',async()=>{
 const text=fs.readFileSync('static/sessions.js','utf8'),section=text.slice(text.indexOf('function loadInventory('),text.indexOf('function linkedSessions'));
 let release,requests=0;const gate=new Promise(resolve=>release=resolve);
 const c=vm.createContext({inventoryLoading:null,inventory:{candidates:[],directories:[]},state:null,document:{querySelector:()=>null},Option:function(){},$:()=>({replaceChildren(){}}),api:async()=>{requests++;if(requests===1)await gate;return {candidates:requests===1?[]:[{recordId:'chat',live:true}],directories:[]};}});
 vm.runInContext(section,c);
 const first=c.loadInventory(),second=c.loadInventory();let finished=false;second.then(()=>finished=true);await Promise.resolve();assert.equal(finished,false);assert.equal(requests,1);
 const fresh=c.loadInventory(true);release();await Promise.all([first,second,fresh]);assert.equal(requests,2);assert.equal(c.inventory.candidates[0].live,true);
});

function titlebarEditorHarness(){
  const nodes=new Map(),launcher={id:'launcher'},children=[{id:'tabs'},launcher,{id:'skills'}];
  const header={insertBefore(node,before){children.splice(children.indexOf(before),0,node);nodes.set('#'+node.id,node);},append(node){const index=children.indexOf(node);if(index>=0)children.splice(index,1);children.push(node);}};
  const opened=[],errors=[],record={id:'one',cwd:'/chat'},candidate={recordId:'one'},serviceClient={isDesktop:true,openDiff:async directory=>opened.push(directory)};
  const context=vm.createContext({$:selector=>selector==='#agent-topbar'?header:selector==='.compact-launcher'?launcher:nodes.get(selector),featureButton:(text,onclick)=>({dataset:{},append(){},setAttribute(name,value){this[name]=value;},onclick}),agentIcon:()=>({}),liveCandidates:()=>[candidate],candidateKey:c=>'embedded:'+c.recordId,recordForCandidate:()=>record,state:{sessions:[record]},selectedAgentKey:'embedded:one',source:'agents',selectedGitDirectory:()=>'/selected/worktree',serviceClient,toast:message=>errors.push(message)});
  vm.runInContext(source.slice(source.indexOf('function renderTitlebarEditor('),source.indexOf('function renderLiveBar(')),context);
  return {context,children,launcher,opened,errors,serviceClient,button:()=>nodes.get('#titlebar-open-editor')};
}
test('titlebar editor follows the selected worktree and stays disabled outside chat',async()=>{
  const h=titlebarEditorHarness();h.context.renderTitlebarEditor();
  assert.equal(h.button().disabled,false);assert.equal(h.button().dataset.directory,'/selected/worktree');
  assert.equal(h.children.at(-1),h.launcher);
  await h.button().onclick();assert.deepEqual(h.opened,['/selected/worktree']);
  h.context.source='github';h.context.renderTitlebarEditor();
  assert.equal(h.button().disabled,true);assert.equal(h.button().dataset.directory,'');
  await h.button().onclick();assert.equal(h.opened.length,1);
  h.context.source='agents';h.context.selectedAgentKey='record:one';h.context.renderTitlebarEditor();assert.equal(h.button().disabled,false);
  h.context.selectedAgentKey='record:missing';h.context.renderTitlebarEditor();assert.equal(h.button().disabled,true);
  h.context.selectedAgentKey='embedded:one';h.serviceClient.isDesktop=false;h.context.renderTitlebarEditor();assert.equal(h.button().disabled,true);
  assert.equal(h.children.filter(node=>node.id==='titlebar-open-editor').length,1);
});
test('titlebar editor reports failures and restores its availability',async()=>{
  const h=titlebarEditorHarness();h.serviceClient.openDiff=async()=>{assert.equal(h.button().disabled,true);throw new Error('Editor unavailable');};
  h.context.renderTitlebarEditor();await h.button().onclick();
  assert.deepEqual(h.errors,['Editor unavailable']);assert.equal(h.button().disabled,false);
});

test('Skills Add delegates to the shared initializer without resetting a separate form',()=>{
 const node=()=>({children:[],append(...children){this.children.push(...children);},replaceChildren(...children){this.children=children;}}),list=node(),controls=node();let opens=0;const c=vm.createContext({$:selector=>({'#work-list':list,'#feature-controls':controls,'#search':{value:''}})[selector],featureButton:(text,onclick)=>({text,onclick}),el:node,artifactLibrary:{presets:[],artifacts:[]},openSkillEditor:()=>opens++,loadArtifactLibrary:()=>{}});
 vm.runInContext(source.slice(source.indexOf('function renderSkills('),source.indexOf("$('#skills-inbox').onclick")),c);c.renderSkills();controls.children[0].onclick();assert.equal(opens,1);
});

function liveBarFixture(){
 let document;const nodes=new Map(),actions=[];
 const matches=(node,selector)=>selector.split(',').some(part=>{part=part.trim();if(part==='[hidden]')return node.hidden;if(part==='dialog[open]')return node.tagName==='DIALOG'&&node.open;if(part==='[data-agent-control]')return !!node.dataset.agentControl;return part.startsWith('.')&&node.className.split(' ').includes(part.slice(1));});
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.open=false;this.value='';this.disabled=false;this.classList={add:name=>{if(!this.className.split(' ').includes(name))this.className+=' '+name;},contains:name=>this.className.split(' ').includes(name),toggle:(name,enabled)=>{this.className=this.className.split(' ').filter(value=>value!==name).join(' ');if(enabled)this.className+=' '+name;}};}
  get isConnected(){return this===document.body||!!this.parentElement?.isConnected;}contains(node){return this===node||this.children.some(child=>child.contains(node));}
  append(...children){for(const child of children){child.remove();child.parentElement=this;this.children.push(child);}}
  remove(){if(this.parentElement){if(this.contains(document.activeElement))document.activeElement=document.body;this.parentElement.children=this.parentElement.children.filter(child=>child!==this);this.parentElement=null;}}
  replaceChildren(...children){if(this.contains(document.activeElement))document.activeElement=document.body;for(const child of this.children)child.parentElement=null;this.children=[];this.append(...children);}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(matches(child,selector)?[child]:[]),...child.querySelectorAll(selector)]);}querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  closest(selector){for(let node=this;node;node=node.parentElement)if(matches(node,selector))return node;return null;}
  setAttribute(key,value){this.attrs[key]=value;}
  getClientRects(){if(!this.isConnected||this.closest('[hidden]'))return [];for(let child=this,parent=this.parentElement;parent;child=parent,parent=parent.parentElement){if(parent.tagName==='DIALOG'&&!parent.open||parent.tagName==='DETAILS'&&!parent.open&&child!==parent.children.find(node=>node.tagName==='SUMMARY'))return [];}return [{}];}
  focus(options){if(!this.disabled&&this.getClientRects().length){document.activeElement=this;this.focusOptions=options;}}
  select(){this.selectionStart=0;this.selectionEnd=this.value.length;}setCustomValidity(){}
 }
 document={body:null,activeElement:null,querySelector:selector=>document.body.querySelector(selector),querySelectorAll:selector=>document.body.querySelectorAll(selector)};document.body=new Node('body');document.activeElement=document.body;
 for(const [id,tag]of [['live-agent-bar','div'],['live-agent-count','span'],['skills-unread','span'],['new-note','button']]){const node=new Node(tag);nodes.set('#'+id,node);document.body.append(node);}
 const candidates=['one','two','three'].map((id,index)=>({host:'embedded',agent:'codex',recordId:id,sessionId:null,live:true,generation:'generation-'+id,startedAt:100+index})),state={sessions:candidates.map(candidate=>({id:candidate.recordId,title:candidate.recordId,items:[],runtime:{state:'working',subagents:0,at:200}}))};
 const c=vm.createContext({document,Date,state,source:'agents',selectedAgentKey:'embedded:one',liveBarSignature:'',agentRename:null,pendingSessionResumes:new Set(),snoozedAgentTabs:{},inventory:{candidates,warning:''},agentContextReveals:new Set(),agentSidebarCollapsed:true,contextSectionsCollapsed:{},linkedWorkCollapsed:true,el:(...args)=>new Node(...args),$:(selector,root)=>{if(root)return root.querySelector(selector);const tab=selector.match(/^#live-agent-bar \.agent-tab\[data-agent-key="([^"]+)"\]$/);if(tab)return nodes.get('#live-agent-bar').children.find(node=>node.dataset.agentKey===tab[1]);return nodes.get(selector)||document.body.querySelector(selector);},agentIcon:()=>new Node('svg'),renderTitlebarEditor(){},visibleAgentCandidates:()=>candidates,liveCandidates:()=>candidates,isAgentSnoozed:()=>false,recordForCandidate:candidate=>c.state.sessions.find(row=>row.id===candidate.recordId),agentName:()=> 'Codex',workSummary:()=>'',renderSnoozedAgents(){},updateTabDensity(){},skillInboxCounts:()=>({total:0}),openLaunch:()=>actions.push('manage'),snoozeAgentTab:candidate=>actions.push('snooze:'+candidate.recordId),closeAgentTab:candidate=>actions.push('close:'+candidate.recordId),selectAgent:(record,candidate)=>actions.push('select:'+candidate.recordId),persist(){},toast(){},terminalViews:new Map()});
 const features=fs.readFileSync('static/features.js','utf8');vm.runInContext(features.slice(features.indexOf('function featureButton('),features.indexOf('// Fixed feature forms')),c);
 vm.runInContext(source.slice(source.indexOf('function candidateKey('),source.indexOf('function isAgentSnoozed(')),c);
 vm.runInContext(source.slice(source.indexOf('function runtimeInfo('),source.indexOf('function workSummary(')),c);
 vm.runInContext(source.slice(source.indexOf('function beginAgentRename('),source.indexOf('let agentNavigationEpoch=')),c);
 vm.runInContext(source.slice(source.indexOf('function renderLiveBar('),source.indexOf('function rememberClosedTab(')),c);
 c.render=()=>c.renderLiveBar();c.renderLiveBar();const bar=nodes.get('#live-agent-bar'),tab=id=>bar.children.find(node=>node.dataset.agentKey==='embedded:'+id),control=(id,role)=>tab(id)?.querySelectorAll('[data-agent-control]').find(node=>node.dataset.agentControl===role),menu=id=>tab(id)?.querySelector('.agent-menu');
 const bump=()=>{state.sessions[0].runtime.subagents++;c.renderLiveBar();};return {c,document,Node,state,candidates,actions,bar,tab,control,menu,bump,launcher:nodes.get('#new-note')};
}
test('actual live bar keeps focused tab actions across runtime, title and subagent refreshes without executing them',()=>{
 for(const role of ['tab','snooze','close','actions','manage','attach-work','attach-artifact']){
  const f=liveBarFixture();if(['manage','attach-work','attach-artifact'].includes(role))f.menu('one').open=true;const old=f.control('one',role);old.focus();assert.equal(f.document.activeElement,old);f.state.sessions[0].title='Updated title';f.bump();const next=f.control('one',role);assert.notEqual(next,old);assert.equal(old.isConnected,false);assert.equal(f.document.activeElement,next,role);assert.equal(next.focusOptions.preventScroll,true);assert.deepEqual(f.actions,[]);
 }
});
test('actual live bar preserves only currently open selected actions for the same process, not intentional action closes',()=>{
 const f=liveBarFixture();f.menu('one').open=true;f.bump();assert.equal(f.menu('one').open,true);f.menu('one').open=false;f.bump();assert.equal(f.menu('one').open,false);
 f.menu('one').open=true;f.control('one','manage').focus();f.control('one','manage').onclick();f.bump();assert.deepEqual(f.actions,['manage']);assert.equal(f.menu('one').open,false);assert.equal(f.document.activeElement,f.control('one','tab'));
 for(const role of ['attach-work','attach-artifact']){const kind=role.slice(7),section=new f.Node('section',kind==='work'?'agent-work-section':'agent-results-section'),attach=new f.Node('details','inline-attach');section.append(attach);f.document.body.append(section);f.menu('one').open=true;f.control('one',role).focus();f.control('one',role).onclick();assert.equal(f.menu('one').open,false);assert.equal(attach.open,true);assert.equal(f.c.agentContextReveals.has('one:'+kind),true);assert.equal(f.c.agentSidebarCollapsed,false);assert.equal(f.c.linkedWorkCollapsed,false);assert.equal(f.c.contextSectionsCollapsed.results,false);}
 f.menu('one').open=true;f.c.selectedAgentKey='embedded:two';f.bump();assert.equal(f.menu('one'),null);assert.equal(f.menu('two').open,false);
});
test('live bar reordering keeps exact controls; removed tabs or changed process generations fall back non-destructively',()=>{
 const f=liveBarFixture();f.control('two','close').focus();f.candidates.reverse();f.bump();assert.equal(f.document.activeElement,f.control('two','close'));
 f.candidates.splice(f.candidates.findIndex(candidate=>candidate.recordId==='two'),1);f.bump();assert.equal(f.document.activeElement,f.control('one','tab'));assert.notEqual(f.document.activeElement.dataset.agentControl,'close');
 f.menu('one').open=true;f.control('one','manage').focus();f.candidates.find(candidate=>candidate.recordId==='one').generation='replacement-generation';f.c.renderLiveBar();assert.equal(f.menu('one').open,false);assert.equal(f.document.activeElement,f.control('three','tab'));
 f.candidates.splice(0);f.bump();assert.equal(f.document.activeElement,f.launcher);assert.deepEqual(f.actions,[]);
});
test('generation-only and session identity changes trigger refresh and never revive old action focus or menus',()=>{
 for(const field of ['generation','agent']){
  const f=liveBarFixture();f.menu('one').open=true;f.control('one','manage').focus();const old=f.control('one','manage');f.candidates[0][field]=field==='startedAt'?999:'changed';f.c.renderLiveBar();assert.equal(old.isConnected,false);assert.equal(f.menu('one').open,false);assert.equal(f.document.activeElement,f.control('two','tab'));
 }
});
test('live bar never takes outside or terminal focus, nor focuses behind a modal or changed navigation owner',()=>{
 for(const mode of ['outside','terminal','modal','during-outside','during-modal','during-modal-body','during-page','during-selection','during-host']){
  const f=liveBarFixture(),outside=new f.Node('input'),terminal=new f.Node('textarea','xterm-helper-textarea');f.document.body.append(outside,terminal);let expected;
  const change=()=>{if(mode==='during-modal-body'){const child=new f.Node('dialog');child.open=true;f.document.body.append(child);f.document.activeElement=f.document.body;expected=f.document.body;}else if(mode.endsWith('outside')){outside.focus();expected=outside;}else if(mode.endsWith('terminal')){terminal.focus();expected=terminal;}else if(mode.endsWith('modal')){const child=new f.Node('dialog');child.open=true;f.document.body.append(child);child.focus();expected=child;}else if(mode==='during-page'){f.c.source='notes';expected=f.document.body;}else if(mode==='during-selection'){f.c.selectedAgentKey='embedded:two';expected=f.document.body;}else if(mode==='during-host'){f.bar.remove();expected=f.document.body;}};
  f.control('one','snooze').focus();if(mode.startsWith('during-'))f.c.renderSnoozedAgents=change;else change();f.bump();assert.equal(f.document.activeElement,expected,mode);assert.deepEqual(f.actions,[]);
 }
});
test('live bar retains actual rename input node, value and caret without restoring it across a new process',()=>{
 const f=liveBarFixture();f.c.beginAgentRename();const input=f.c.agentRename.input;input.value='Unsaved title';input.selectionStart=3;input.selectionEnd=7;input.focus();f.bump();assert.equal(f.c.agentRename.input,input);assert.equal(f.document.activeElement,input);assert.equal(input.value,'Unsaved title');assert.equal(input.selectionStart,3);assert.equal(input.selectionEnd,7);
 const outside=new f.Node('input');f.document.body.append(outside);outside.focus();f.bump();assert.equal(f.document.activeElement,outside);input.focus();f.candidates[0].generation='new-process';f.c.renderLiveBar();assert.notEqual(f.document.activeElement,input);assert.equal(input.value,'Unsaved title');assert.equal(f.document.activeElement,f.control('two','tab'));
});

test('live bar retains the same strong-generation owner through late session registration and uses start identity only as fallback',()=>{
 const f=liveBarFixture();f.menu('one').open=true;f.control('one','manage').focus();f.candidates[0].sessionId='registered-session';f.candidates[0].startedAt=999;f.state.sessions[0].title='Registered title';f.c.renderLiveBar();assert.equal(f.menu('one').open,true);assert.equal(f.document.activeElement,f.control('one','manage'));
 for(const field of ['startedAt','sessionId']){const g=liveBarFixture();delete g.candidates[0].generation;g.c.renderLiveBar();g.menu('one').open=true;g.control('one','manage').focus();g.candidates[0][field]=field==='startedAt'?999:'registered';g.c.renderLiveBar();assert.equal(g.menu('one').open,false);assert.equal(g.document.activeElement,g.control('two','tab'));}
});
test('deliberate source or selection navigation does not revive old menu ownership',()=>{
 for(const mode of ['page','selection']){const f=liveBarFixture();f.menu('one').open=true;f.control('one','manage').focus();if(mode==='page')f.c.source='notes';else f.c.selectedAgentKey='embedded:two';f.c.renderLiveBar();assert.equal(f.document.activeElement,f.document.body);assert.equal(f.menu(mode==='page'?'one':'two').open,false);}
});

test('removed action control falls back to its own tab, while missing process identity never revives a menu',()=>{
 const f=liveBarFixture();f.menu('one').open=true;f.control('one','manage').focus();f.state.sessions.splice(0,1);f.c.renderLiveBar();assert.equal(f.menu('one'),null);assert.equal(f.document.activeElement,f.control('one','tab'));
 const g=liveBarFixture();delete g.candidates[0].generation;delete g.candidates[0].startedAt;g.c.renderLiveBar();g.menu('one').open=true;g.control('one','manage').focus();g.state.sessions[0].title='Unverified process';g.c.renderLiveBar();assert.equal(g.menu('one').open,false);assert.equal(g.document.activeElement,g.control('two','tab'));
});
