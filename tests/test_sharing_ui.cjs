const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/sharing.js','utf8'),app=fs.readFileSync('static/app.js','utf8');
function fixture(){
 let document;const calls=[],hooks=new Map(),intervals=[],settingsCalls=[],selected=[],messages=[];
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.children=[];this.parentNode=null;this.dataset={};this.attrs={};this.value='';this.hidden=false;this.disabled=false;this.listeners=new Map();this._open=false;}
  append(...children){for(const child of children){if(child.parentNode)child.parentNode.children=child.parentNode.children.filter(n=>n!==child);child.parentNode=this;this.children.push(child);}}
  replaceChildren(...children){if(this.contains(document.activeElement))document.activeElement=document.body;for(const child of this.children)child.parentNode=null;this.children=[];this.append(...children);}
  all(){return this.children.flatMap(child=>[child,...child.all()]);}
  contains(child){return child===this||this.all().includes(child);}
  get childElementCount(){return this.children.length;}get lastChild(){return this.children.at(-1);}get options(){return this.children;}
  add(option){this.append(option);}setAttribute(name,value){this.attrs[name]=String(value);}getAttribute(name){return this.attrs[name]??null;}
  matches(selector){if(selector.startsWith('#'))return this.id===selector.slice(1);if(selector.startsWith('.'))return this.className.split(/\s+/).includes(selector.slice(1));return this.tagName===selector.toUpperCase();}
  querySelectorAll(selector){return this.all().filter(node=>node.matches(selector));}querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  addEventListener(type,fn,options={}){this.listeners.set(type,[...(this.listeners.get(type)||[]),{fn,once:options.once}]);}
  removeEventListener(type,fn){this.listeners.set(type,(this.listeners.get(type)||[]).filter(entry=>entry.fn!==fn));}
  dispatch(type,event={}){event.target??=this;for(const entry of [...this.listeners.get(type)||[]]){if(entry.once)this.removeEventListener(type,entry.fn);entry.fn(event);}}
  get open(){return this._open;}set open(value){if(this._open===value)return;this._open=value;this.dispatch('toggle');}
  showModal(){this.open=true;}close(){this.open=false;this.dispatch('close');}
  focus(){if(document.activeElement===this)return;document.activeElement=this;this.onfocus?.();}
 }
 document={body:new Node('body'),hidden:false,activeElement:null,createElement:tag=>new Node(tag),createTextNode:text=>new Node('#text','',text),querySelector(selector){return this.body.querySelector(selector);},addEventListener(){}};document.activeElement=document.body;
 const make=(tag,id,parent=document.body)=>{const n=new Node(tag);n.id=id;parent.append(n);return n;};
 const menu=make('details','sharing-menu'),launcher=make('summary','sharing-launcher',menu),panel=make('div','sharing-popover',menu),badge=make('span','sharing-unread',launcher);
 const settings=make('dialog','settings-dialog');make('input','settings-search');make('div','settings-body');make('select','quick-agent').value='codex';
 const server={config:{root:'/Volumes/Team',handle:'sam'},people:[{id:'alex',name:'Alex'}],entries:[],jobs:[],unread:0};const handlers={};
 const c=vm.createContext({document,console,Option:function(text,value){const n=new Node('option','',text);n.value=value;return n;},$:selector=>document.querySelector(selector),el:(...args)=>new Node(...args),state:{sessions:[{id:'a',title:'Chat A'},{id:'b',title:'Chat B'}]},selectedAgentKey:'record:a',source:'agents',liveCandidates:()=>[],recordForCandidate:()=>null,candidateKey:()=>'',candidateForRecord:()=>null,inventory:{directories:['/work/project']},settingsDirty:false,settingsModifiedOnly:true,settingsCategory:'appearance',openPreferences:async({category}={})=>{if(!settings.open){settingsCalls.push('open');settings.showModal();}if(category!==undefined){c.settingsCategory=category;c.resetSettingsFilter();c.filterSettings();}return true;},resetSettingsFilter:()=>{document.querySelector('#settings-search').value='';c.settingsModifiedOnly=false;settingsCalls.push('clear-filters');},filterSettings:()=>settingsCalls.push('filter'),toast:message=>messages.push(message),selectAgent:(record)=>{selected.push(record.id);c.selectedAgentKey='record:'+record.id;},navigator:{clipboard:{writeText:async()=>{}}},setInterval:callback=>intervals.push(callback),registerRenderHook:(name,fn)=>hooks.set(name,fn),localStorage:{getItem(){throw Error('Sharing drafts must stay in memory');},setItem(){throw Error('Sharing drafts must stay in memory');}},api:async(path,payload)=>{if(path!=='/api/commands/invoke'){calls.push({path,args:payload});if(handlers[path])return handlers[path](payload);if(path==='/api/state')return JSON.parse(JSON.stringify(c.state));if(path==='/api/agents')return c.inventory;assert.fail('Unexpected API '+path);}calls.push({name:payload.name,args:JSON.parse(JSON.stringify(payload.args))});if(handlers[payload.name])return handlers[payload.name](payload.args);if(payload.name==='chats.open')return {status:'accepted',recordId:payload.args.recordId,target:'embedded',localSaved:true,terminal:{state:'acknowledged'}};if(payload.name==='sharing.job')return {preview:{title:'Prepared context',resources:[]}};return JSON.parse(JSON.stringify(server));}});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);
 const sessions=fs.readFileSync('static/sessions.js','utf8');c.render=()=>{};c.loadInventory=async()=>{const next=await c.api('/api/agents');c.inventory=next;return next;};vm.runInContext(sessions.slice(sessions.indexOf('const pendingSessionResumes='),sessions.indexOf('let recoveryOriginal=')),c);
 vm.runInContext(source,c);
 const tick=()=>new Promise(resolve=>setImmediate(resolve));const find=label=>panel.all().find(n=>n.tagName==='BUTTON'&&(n.textContent===label||n.attrs['aria-label']===label));
 const input=(id,value)=>{const node=document.querySelector('#'+id);node.value=value;node.oninput?.();return node;};
 const open=async()=>{menu.open=true;await tick();await tick();};
 const refresh=async()=>{await find('Refresh').onclick();await tick();};
 const openDelivery=title=>panel.all().find(n=>n.className.includes('sharing-entry-open')&&n.children[0]?.textContent===title).onclick();
 return {c,server,handlers,calls,hooks,settingsCalls,selected,messages,Node,document,menu,panel,settings,launcher,badge,find,input,open,refresh,tick,openDelivery};
}
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}

test('sharing uses shared controls, labels and Settings owner without a duplicate configuration form',async()=>{
 const f=fixture();await f.open();assert.equal(f.document.querySelector('#sharing-config'),null);
 for(const id of ['sharing-recipient','sharing-message'])assert.ok(f.document.querySelector('#'+id).className.includes('ui-field'));
 assert.equal(f.find('Close sharing').dataset.size,'icon');assert.equal(f.find('Sharing settings').dataset.size,'icon');assert.equal(f.document.querySelector('#sharing-status').attrs.role,'status');assert.equal(f.document.querySelector('#sharing-error').attrs.role,'status');
 assert.ok(f.panel.all().some(n=>n.tagName==='LABEL'&&n.children[0]?.textContent==='Message (optional)'));assert.ok(f.panel.all().some(n=>n.className.includes('ui-actiongroup')&&n.attrs.role==='group'));
 await f.find('Sharing settings').onclick();assert.equal(f.c.settingsCategory,'sharing');assert.deepEqual(f.settingsCalls,['open','clear-filters','filter']);assert.equal(f.c.settingsModifiedOnly,false);assert.equal(f.menu.open,false);assert.equal(f.calls.some(call=>call.name==='sharing.configure'),false);
});
test('Settings gear preserves dirty drafts and reuses an existing Settings dialog without resetting it',async()=>{
 const f=fixture();await f.open();f.c.settingsDirty=true;f.document.querySelector('#settings-search').value='keep query';await f.find('Sharing settings').onclick();assert.equal(f.c.settingsModifiedOnly,true);assert.equal(f.document.querySelector('#settings-search').value,'keep query');assert.equal(f.settings.open,false);assert.deepEqual(f.settingsCalls,[]);assert.match(f.document.querySelector('#sharing-error').children[0].textContent,/Save or discard/);
 f.c.settingsDirty=false;f.settings.showModal();await f.find('Sharing settings').onclick();assert.deepEqual(f.settingsCalls,['clear-filters','filter']);assert.equal(f.c.settingsModifiedOnly,false);assert.equal(f.document.querySelector('#settings-search').value,'');
});
test('share recipient, message and recent choice survive view changes, same-view click and close/reopen per chat',async()=>{
 const f=fixture();await f.open();const recipient=f.input('sharing-recipient','/alex');f.input('sharing-message','Draft for A');const recent=f.document.querySelector('#sharing-recent');recent.checked=false;recent.onchange();
 await f.find('Share').onclick();assert.equal(f.document.querySelector('#sharing-recipient'),recipient);
 f.find('Inbox').focus();f.find('Inbox').onclick();assert.equal(f.document.activeElement,f.find('Inbox'));f.find('Share').onclick();assert.equal(f.document.activeElement,f.find('Share'));
 assert.equal(f.document.querySelector('#sharing-message').value,'Draft for A');assert.equal(f.document.querySelector('#sharing-recipient').value,'/alex');assert.equal(f.document.querySelector('#sharing-recent').checked,false);
 f.menu.open=false;await f.open();assert.equal(f.document.querySelector('#sharing-message').value,'Draft for A');
 f.c.selectedAgentKey='record:b';f.hooks.get('sharing:initial-load')();assert.equal(f.document.querySelector('#sharing-message').value,'');f.input('sharing-message','Draft for B');f.c.selectedAgentKey='record:a';f.hooks.get('sharing:initial-load')();assert.equal(f.document.querySelector('#sharing-message').value,'Draft for A');
});
test('server previous sends seed only untouched new drafts and never overwrite edited messages after polling or reopen',async()=>{
 const f=fixture();f.server.jobs=[{id:'prepared-a',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'},{id:'sent-a',kind:'send',draftId:'prepared-a',status:'ready',recipients:['old'],message:'Old sent message'}];await f.open();assert.equal(f.document.querySelector('#sharing-message').value,'Old sent message');
 f.input('sharing-recipient','/alex');f.input('sharing-message','New unsent message');f.menu.open=false;await f.open();assert.equal(f.document.querySelector('#sharing-message').value,'New unsent message');assert.equal(f.document.querySelector('#sharing-recipient').value,'/alex');assert.equal(f.find('Send').disabled,false);
});
test('Escape dismisses recipient suggestions first, updates expanded state and retains the sharing surface',async()=>{
 const f=fixture();await f.open();const input=f.input('sharing-recipient','al');input.focus();const people=f.document.querySelector('#sharing-people');assert.ok(people.children.length);assert.equal(input.attrs['aria-expanded'],'true');people.children[0].focus();let stopped=false,prevented=false;
 people.onkeydown({key:'Escape',preventDefault(){prevented=true;},stopPropagation(){stopped=true;}});assert.equal(stopped,true);assert.equal(prevented,true);assert.equal(f.menu.open,true);assert.equal(people.children.length,0);assert.equal(input.attrs['aria-expanded'],'false');assert.equal(f.document.activeElement,input);
});
test('pending prepare resolves only to its original chat while Inbox or another chat is displayed',async()=>{
 const f=fixture(),pending=deferred();f.handlers['sharing.prepare']=()=>pending.promise;await f.open();f.input('sharing-recipient','/alex');f.input('sharing-message','A context');const prepare=f.find('Prepare handover').onclick();assert.equal(f.find('Prepare handover').disabled,true);f.find('Inbox').onclick();
 f.server.jobs=[{id:'prepared-a',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'}];pending.resolve({...f.server,jobId:'prepared-a'});await prepare;await f.tick();assert.ok(f.document.querySelector('#sharing-entries'));f.c.selectedAgentKey='record:b';f.find('Share').onclick();assert.equal(f.find('Send').disabled,true);f.c.selectedAgentKey='record:a';f.hooks.get('sharing:initial-load')();assert.equal(f.find('Send').disabled,false);assert.equal(f.document.querySelector('#sharing-message').value,'A context');
});
test('pending send locks its captured inputs across tabs/chats and allows explicit edits after completion',async()=>{
 const f=fixture(),pending=deferred();f.server.jobs=[{id:'prepared-a',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'}];f.handlers['sharing.send']=()=>pending.promise;await f.open();f.input('sharing-recipient','/alex');f.input('sharing-message','Original message');const send=f.find('Send').onclick();assert.equal(f.document.querySelector('#sharing-message').disabled,true);f.input('sharing-message','Blocked edit');assert.equal(f.document.querySelector('#sharing-message').value,'Original message');f.find('Inbox').onclick();f.c.selectedAgentKey='record:b';f.find('Share').onclick();f.input('sharing-message','B message');
 f.server.jobs.push({id:'sent-a',kind:'send',draftId:'prepared-a',status:'ready'});pending.resolve({...f.server,jobId:'sent-a'});await send;assert.equal(f.document.querySelector('#sharing-message').value,'B message');
 const request=f.calls.find(call=>call.name==='sharing.send');assert.deepEqual(request.args,{draftId:'prepared-a',recipients:['alex'],message:'Original message'});f.c.selectedAgentKey='record:a';f.hooks.get('sharing:initial-load')();assert.equal(f.document.querySelector('#sharing-message').value,'Original message');assert.equal(f.find('Sent').disabled,true);assert.equal(f.document.querySelector('#sharing-message').disabled,false);f.input('sharing-message','Later unsent edit');assert.equal(f.find('Send').disabled,false);
});
test('Inbox archive and restore controls name both the action and the shared entry',async()=>{
 const f=fixture();f.server.entries=[{id:'delivery',title:'Research handover',dismissed:false}];await f.open();f.find('Inbox').onclick();assert.equal(f.find('Archive Research handover').dataset.size,'icon');
 f.server.entries[0].dismissed=true;f.find('Show archived').onclick();await f.refresh();assert.ok(f.find('Restore Research handover'));assert.ok(f.find('Close sharing'));
});

test('pending send completion retains its own Sent state after Inbox switching and close/reopen',async()=>{
 const f=fixture(),pending=deferred();f.server.jobs=[{id:'prepared-a',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'}];f.handlers['sharing.send']=()=>pending.promise;await f.open();f.input('sharing-recipient','/alex');f.input('sharing-message','Ready message');const sending=f.find('Send').onclick();f.find('Inbox').onclick();f.menu.open=false;await f.open();
 f.server.jobs.push({id:'sent-a',kind:'send',draftId:'prepared-a',status:'ready'});pending.resolve({...f.server,jobId:'sent-a'});await sending;f.find('Share').onclick();assert.equal(f.find('Sent').disabled,true);assert.equal(f.document.querySelector('#sharing-message').value,'Ready message');
});
test('a failed prepare stays associated with its originating chat rather than the newly selected chat',async()=>{
 const f=fixture(),pending=deferred();f.handlers['sharing.prepare']=()=>pending.promise;await f.open();const preparing=f.find('Prepare handover').onclick();f.c.selectedAgentKey='record:b';f.hooks.get('sharing:initial-load')();pending.reject(new Error('Chat A preparation failed'));await preparing;
 assert.equal(f.document.querySelector('#sharing-error').children.length,0);f.c.selectedAgentKey='record:a';f.hooks.get('sharing:initial-load')();assert.match(f.document.querySelector('#sharing-error').children[0].textContent,/Chat A preparation failed/);
});
test('receiving assistant and folder survive Inbox switches and reopen for the same delivery',async()=>{
 const f=fixture();f.server.entries=[{id:'incoming-a',title:'Incoming A'},{id:'incoming-b',title:'Incoming B'}];
 f.handlers['sharing.preview']=args=>{const id='preview-'+args.deliveryId;f.server.jobs.push({id,kind:'preview',status:'ready',recommendedCwd:args.deliveryId==='incoming-a'?'/work/project':''});return {...f.server,jobId:id};};
 await f.open();f.find('Inbox').onclick();const openEntry=title=>f.panel.all().find(n=>n.className.includes('sharing-entry-open')&&n.children[0]?.textContent===title).onclick();await openEntry('Incoming A');await f.tick();
 const agent=f.document.querySelector('#sharing-agent');agent.value='claude';agent.onchange();const folder=f.document.querySelector('#sharing-folder');folder.value='/work/project';folder.onchange();f.find('Share').onclick();f.find('Inbox').onclick();assert.equal(f.document.querySelector('#sharing-agent').value,'claude');assert.equal(f.document.querySelector('#sharing-folder').value,'/work/project');
 f.menu.open=false;await f.open();assert.equal(f.document.querySelector('#sharing-agent').value,'claude');await openEntry('Incoming B');assert.equal(f.document.querySelector('#sharing-agent').value,'codex');assert.equal(f.document.querySelector('#sharing-folder').value,'');assert.equal(f.find('Import into new chat').disabled,true);await openEntry('Incoming A');assert.equal(f.document.querySelector('#sharing-agent').value,'claude');
});
test('draft cache keeps bounded recent chat state without using persistent storage',async()=>{
 const f=fixture();await f.open();for(let index=0;index<22;index++){const id='chat-'+index;f.c.state.sessions.push({id,title:id});f.c.selectedAgentKey='record:'+id;f.hooks.get('sharing:initial-load')();f.input('sharing-message','Draft '+index);}
 f.c.selectedAgentKey='record:chat-0';f.hooks.get('sharing:initial-load')();assert.equal(f.document.querySelector('#sharing-message').value,'');f.c.selectedAgentKey='record:chat-21';f.hooks.get('sharing:initial-load')();assert.equal(f.document.querySelector('#sharing-message').value,'Draft 21');
});

test('changing Recent conversation after a ready preparation invalidates it until explicitly prepared again',async()=>{
 const f=fixture();f.server.jobs=[{id:'with-conversation',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'}];f.handlers['sharing.prepare']=args=>{f.server.jobs.push({id:'without-conversation',kind:'prepare',recordId:'a',status:'ready'});return {...f.server,jobId:'without-conversation'};};await f.open();f.input('sharing-recipient','/alex');assert.equal(f.find('Send').disabled,false);
 const recent=f.document.querySelector('#sharing-recent');recent.checked=false;recent.onchange();assert.equal(f.find('Send').disabled,true);await f.find('Prepare handover').onclick();assert.equal(f.find('Send').disabled,false);assert.deepEqual(f.calls.find(call=>call.name==='sharing.prepare').args,{recordId:'a',recentConversation:false});await f.find('Send').onclick();assert.equal(f.calls.find(call=>call.name==='sharing.send').args.draftId,'without-conversation');
});
test('Recent conversation stays locked through pending preparation and the backend working job',async()=>{
 const f=fixture(),pending=deferred();f.handlers['sharing.prepare']=()=>pending.promise;await f.open();const recent=f.document.querySelector('#sharing-recent');recent.checked=false;recent.onchange();const preparing=f.find('Prepare handover').onclick();assert.equal(recent.disabled,true);recent.checked=true;recent.onchange();assert.equal(recent.checked,false);
 f.server.jobs=[{id:'pending-prepare',kind:'prepare',recordId:'a',status:'working'}];pending.resolve({...f.server,jobId:'pending-prepare'});await preparing;assert.equal(recent.disabled,true);recent.checked=true;recent.onchange();assert.equal(recent.checked,false);assert.equal(f.find('Send').disabled,true);
 f.find('Inbox').onclick();f.server.jobs[0].status='ready';await f.refresh();f.find('Share').onclick();assert.equal(f.document.querySelector('#sharing-recent').disabled,false);assert.equal(f.document.querySelector('#sharing-recent').checked,false);
});
test('unknown previous preparation content is not silently attached to a new Recent conversation choice',async()=>{
 const f=fixture();f.server.jobs=[{id:'old-prepare',kind:'prepare',recordId:'a',status:'ready'},{id:'old-send',kind:'send',draftId:'old-prepare',recipients:['alex'],message:'Prior message',status:'ready'}];await f.open();assert.equal(f.document.querySelector('#sharing-message').value,'Prior message');assert.equal(f.find('Send').disabled,true);assert.match(f.document.querySelector('#sharing-draft').children[0].textContent,/Prepare a handover/);
});
test('API-pending and backend-working sends block edits and duplicate sends until completion',async()=>{
 const f=fixture(),pending=deferred();f.server.jobs=[{id:'prepared-a',kind:'prepare',recordId:'a',recentConversation:true,status:'ready'}];f.handlers['sharing.send']=()=>pending.promise;await f.open();f.input('sharing-recipient','/alex');f.input('sharing-message','Original');const sending=f.find('Send').onclick();const blockedSend=f.find('Sending…');f.input('sharing-recipient','/other');assert.equal(f.document.querySelector('#sharing-recipient').value,'/alex');await blockedSend.onclick();assert.equal(f.calls.filter(call=>call.name==='sharing.send').length,1);
 f.server.jobs.push({id:'working-send',kind:'send',draftId:'prepared-a',status:'working'});pending.resolve({...f.server,jobId:'working-send'});await sending;assert.equal(f.document.querySelector('#sharing-message').disabled,true);f.input('sharing-message','Blocked');assert.equal(f.document.querySelector('#sharing-message').value,'Original');const recent=f.document.querySelector('#sharing-recent');recent.checked=false;recent.onchange();assert.equal(recent.checked,true);await f.find('Sending…').onclick();assert.equal(f.calls.filter(call=>call.name==='sharing.send').length,1);
 f.find('Inbox').onclick();f.server.jobs.at(-1).status='ready';await f.refresh();f.find('Share').onclick();assert.equal(f.find('Sent').disabled,true);assert.equal(f.document.querySelector('#sharing-message').disabled,false);f.input('sharing-message','Next message');assert.equal(f.find('Send').disabled,false);
});

async function receivingFixture(){
 const f=fixture();f.server.entries=[{id:'incoming-a',title:'Incoming A'},{id:'incoming-b',title:'Incoming B'}];
 f.handlers['sharing.preview']=args=>{const id='preview-'+args.deliveryId;f.server.jobs=f.server.jobs.filter(job=>job.id!==id);f.server.jobs.push({id,kind:'preview',status:'ready',recommendedCwd:'/work/project'});return {...f.server,jobId:id};};
 f.c.candidateForRecord=record=>({recordId:record.id});await f.open();f.find('Inbox').onclick();await f.openDelivery('Incoming A');await f.tick();return f;
}
test('pending imports keep captured delivery/options and errors separate from another selected entry',async()=>{
 const f=await receivingFixture(),pending=deferred();f.handlers['sharing.import']=()=>pending.promise;
 const agent=f.document.querySelector('#sharing-agent');agent.value='claude';agent.onchange();
 const importing=f.find('Import into new chat').onclick();assert.equal(f.find('Importing…').disabled,true);
 await f.find('Importing…').onclick();assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,1);
 await f.openDelivery('Incoming B');await f.tick();assert.equal(f.find('Import into new chat').disabled,false);assert.equal(f.find('Importing…'),undefined);
 pending.reject(new Error('Only A import failed'));await importing;
 assert.equal(f.panel.all().some(node=>node.textContent==='Only A import failed'),false);assert.equal(f.find('Import into new chat').disabled,false);
 await f.openDelivery('Incoming A');await f.tick();assert.ok(f.panel.all().some(node=>node.textContent==='Only A import failed'));
 assert.deepEqual(f.calls.find(call=>call.name==='sharing.import').args,{deliveryId:'incoming-a',agent:'claude',cwd:'/work/project'});assert.deepEqual(f.selected,[]);
});
test('background import completion never opens over another entry and provides an explicit Open action',async()=>{
 for(const leave of ['delivery','tab','close']){
  const f=await receivingFixture(),pending=deferred();f.handlers['sharing.import']=()=>pending.promise;
  const importing=f.find('Import into new chat').onclick();
  if(leave==='delivery')await f.openDelivery('Incoming B');else if(leave==='tab')f.find('Share').onclick();else f.menu.open=false;
  f.server.jobs.push({id:'import-a',kind:'import',status:'ready',sessionId:'imported-a'});f.c.state={...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a',title:'Imported A'}]};
  pending.resolve({...f.server,jobId:'import-a'});await importing;await f.tick();
  assert.deepEqual(f.selected,[]);assert.equal(f.calls.some(call=>call.path==='/api/state'),false);
  if(leave==='close')await f.open();if(leave==='tab')f.find('Inbox').onclick();
  assert.deepEqual(f.selected,[]);const open=f.find('Open imported chat: Incoming A');assert.ok(open);await open.onclick();
  assert.deepEqual(f.selected,['imported-a']);assert.equal(f.menu.open,false);assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,1);
 }
});
test('working import progress belongs only to its delivery and repeated clicks do not replay it',async()=>{
 const f=await receivingFixture();f.handlers['sharing.import']=()=>{f.server.jobs.push({id:'import-a',kind:'import',status:'working'});return {...f.server,jobId:'import-a'};};
 await f.find('Import into new chat').onclick();assert.equal(f.find('Importing…').disabled,true);
 await f.openDelivery('Incoming B');await f.tick();assert.equal(f.find('Importing…'),undefined);assert.equal(f.find('Import into new chat').disabled,false);
 await f.openDelivery('Incoming A');await f.tick();const folder=f.document.querySelector('#sharing-folder');folder.onchange();assert.equal(f.find('Importing…').disabled,true);await f.find('Importing…').onclick();assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,1);
});
test('changing delivery while imported chat reads are pending cancels navigation and terminal open',async()=>{
 const f=await receivingFixture(),read=deferred();f.server.entries[0].sessionId='imported-a';f.server.entries[0].imported=true;await f.refresh();
 f.handlers['/api/state']=()=>read.promise;f.c.candidateForRecord=()=>null;
 const opening=f.find('Open imported chat').onclick();await f.tick();await f.openDelivery('Incoming B');
 read.resolve({...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a',title:'Imported A'}]});await opening;
 assert.deepEqual(f.selected,[]);assert.equal(f.calls.some(call=>call.name==='chats.open'),false);assert.equal(f.menu.open,true);assert.equal(f.panel.all().some(node=>node.textContent==='Imported A'),false);
});
test('changing tab, closing the menu or selecting another chat during import opening cancels navigation',async()=>{
 for(const change of ['tab','close','chat','chat-return','page','page-return']){
  const f=await receivingFixture(),read=deferred();f.server.entries[0].sessionId='imported-a';f.server.entries[0].imported=true;await f.refresh();
  f.handlers['/api/state']=()=>read.promise;const opening=f.find('Open imported chat').onclick();await f.tick();
  if(change==='tab')f.find('Share').onclick();else if(change==='close')f.menu.open=false;else if(change.startsWith('page')){f.c.source='github';f.hooks.get('sharing:initial-load')();if(change==='page-return'){f.c.source='agents';f.hooks.get('sharing:initial-load')();}}else{f.c.selectedAgentKey='record:b';f.hooks.get('sharing:initial-load')();if(change==='chat-return'){f.c.selectedAgentKey='record:a';f.hooks.get('sharing:initial-load')();}}
  read.resolve({...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a'}]});await opening;
  assert.deepEqual(f.selected,[]);assert.equal(f.calls.some(call=>call.name==='chats.open'),false);if(change==='chat')assert.equal(f.c.selectedAgentKey,'record:b');if(change==='page')assert.equal(f.c.source,'github');
 }
});
test('imported chat opening rejects stale snapshots, retries bounded reads and preserves newer metadata',async()=>{
 for(const exhausted of [false,true]){
  const f=await receivingFixture(),reads=[];f.server.entries[0].sessionId='imported-a';f.server.entries[0].imported=true;await f.refresh();
  f.handlers['/api/state']=()=>{const read=deferred();reads.push(read);return read.promise;};
  const opening=f.find('Open imported chat').onclick();await f.tick();
  f.c.state={...f.c.state,sessions:f.c.state.sessions.map(record=>record.id==='a'?{...record,title:'Newer A'}:record)};
  reads[0].resolve({revision:'old',sessions:[{id:'a',title:'Old A'},{id:'imported-a'}]});await f.tick();assert.equal(reads.length,2);assert.equal(f.c.state.sessions[0].title,'Newer A');
  if(exhausted)f.c.state={...f.c.state,sessions:f.c.state.sessions.map(record=>record.id==='a'?{...record,title:'Newest A'}:record)};
  reads[1].resolve({revision:'fresh',sessions:[{id:'a',title:'Newer A'},{id:'imported-a',title:'Imported A'}]});await opening;
  assert.equal(reads.length,2);assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,0);
  if(exhausted){assert.equal(f.c.state.sessions[0].title,'Newest A');assert.deepEqual(f.selected,[]);assert.equal(f.menu.open,true);assert.ok(f.panel.all().some(node=>node.textContent.includes('Dashboard changed while opening')));assert.equal(f.find('Open imported chat: Incoming A').disabled,false);}
  else{assert.deepEqual(f.selected,['imported-a']);assert.equal(f.c.state.sessions[0].title,'Newer A');}
 }
});
test('terminal-open response is not assigned as state and its followup read is guarded',async()=>{
 const f=await receivingFixture(),opened=deferred();let candidate=false;f.server.entries[0].sessionId='imported-a';f.server.entries[0].imported=true;await f.refresh();
 f.c.state={...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a',title:'Imported A'}]};f.c.candidateForRecord=record=>candidate?{recordId:record.id}:null;
 f.handlers['chats.open']=()=>opened.promise;
 const opening=f.find('Open imported chat').onclick();await f.tick();assert.equal(f.calls.filter(call=>call.name==='chats.open').length,1);
 f.c.state={...f.c.state,sessions:f.c.state.sessions.map(record=>record.id==='a'?{...record,title:'Newest A'}:record)};candidate=true;
 f.c.inventory={candidates:[{recordId:'imported-a',host:'embedded',live:true}],directories:[]};opened.resolve({status:'accepted',recordId:'imported-a',target:'embedded',localSaved:true,terminal:{state:'acknowledged'},revision:'old',sessions:[{id:'a',title:'Old A'},{id:'imported-a',title:'Old imported A'}]});await opening;
 assert.equal(f.c.state.sessions[0].title,'Newest A');assert.deepEqual(f.selected,['imported-a']);assert.equal(f.calls.filter(call=>call.name==='chats.open').length,1);
});
test('ready import job open receipt is adopted before delayed inventory and never causes another open request',async()=>{
 for(const status of ['accepted','unknown']){const f=await receivingFixture();f.c.candidateForRecord=()=>null;f.c.inventory={candidates:[],directories:['/work/project']};f.handlers['sharing.import']=()=>{f.c.state={...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a',title:'Imported A'}]};f.server.jobs.push({id:'new-import',kind:'import',status:'ready',sessionId:'imported-a',openReceipt:{status,recordId:'imported-a',target:'embedded',localSaved:status==='accepted',terminal:{state:status==='accepted'?'acknowledged':'unconfirmed'}}});return {...f.server,jobId:'new-import'};};await f.find('Import into new chat').onclick();await f.tick();assert.equal(f.calls.filter(call=>call.name==='chats.open').length,0);assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,1);assert.deepEqual(f.selected,[]);await f.find('Open imported chat').onclick();assert.equal(f.calls.filter(call=>call.name==='chats.open').length,0);assert.equal(f.calls.filter(call=>call.name==='sharing.import').length,1);assert.equal(f.menu.open,true);}
});
test('persisted imported entry receipt remains authoritative after reopening the inbox with no live candidate',async()=>{
 const f=await receivingFixture();f.server.entries[0].sessionId='imported-a';f.server.entries[0].imported=true;f.server.entries[0].openReceipt={status:'unknown',recordId:'imported-a',target:'embedded',localSaved:false,terminal:{state:'unconfirmed'}};f.c.state={...f.c.state,sessions:[...f.c.state.sessions,{id:'imported-a',title:'Imported A'}]};f.c.candidateForRecord=()=>null;f.c.inventory={candidates:[],directories:[]};await f.refresh();await f.find('Open imported chat').onclick();assert.equal(f.calls.filter(call=>call.name==='chats.open').length,0);assert.deepEqual(f.selected,[]);assert.ok(f.panel.all().some(node=>node.textContent.includes('could not be confirmed')));
});
