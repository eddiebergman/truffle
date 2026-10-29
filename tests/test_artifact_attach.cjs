const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const workspace=fs.readFileSync('static/workspace.js','utf8'),app=fs.readFileSync('static/app.js','utf8');
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
function fixture({command,library}={}){
 class Node{constructor(tag,cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.isConnected=true;this.value='';this.hidden=false;this.disabled=false;this.open=true;}append(...children){this.children.push(...children);for(const child of children)child.parentElement=this;}set disabled(value){this._disabled=value;if(value&&document.activeElement===this)document.activeElement=null;}get disabled(){return this._disabled;}setAttribute(name,value){this.attrs[name]=value;}focus(options){document.activeElement=this;this.focusOptions=options;}contains(node){return this===node||this.children.some(child=>child.contains(node));}querySelectorAll(selector){const tags=selector.split(',');return this.children.flatMap(child=>[...(tags.includes(child.tagName)?[child]:[]),...child.querySelectorAll(selector)]);}querySelector(selector){return this.querySelectorAll(selector)[0]||null;}}
 const document={activeElement:null,querySelectorAll:()=>[]};
 const calls=[],messages=[],reads=[];let renders=0;
 const c=vm.createContext({document,state:{revision:'one',sessions:[{id:'chat',title:'Old'}]},el:(...a)=>new Node(...a),agentIcon:()=>new Node('i'),featureButton:(label,onclick)=>Object.assign(new Node('button','',label),{onclick}),render:()=>renders++,toast:message=>messages.push(message),loadArtifactLibrary:library|| (async()=>({accepted:true})),api:async(path,args)=>{calls.push({path,args});if(path==='/api/commands/invoke')return command?command():{artifactId:'exact-artifact',recordId:'chat',saved:true};const read=deferred();reads.push(read);return read.promise;}});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);
 const sessions=fs.readFileSync('static/sessions.js','utf8');vm.runInContext(sessions.slice(sessions.indexOf('const pendingSessionResumes='),sessions.indexOf('let recoveryOriginal=')),c);
 vm.runInContext(workspace.slice(workspace.indexOf('function inlineAttach('),workspace.indexOf('function renderHistory(')),c);
 const details=c.inlineAttach({id:'chat',items:[]},'artifact'),form=details.querySelector('form'),[input,title]=form.querySelectorAll('input'),submit=form.querySelectorAll('button').find(node=>node.type==='submit'),error=form.querySelector('p'),retry=form.querySelectorAll('button').find(node=>node.textContent==='Refresh results'),cancel=form.querySelectorAll('button').find(node=>node.textContent==='Cancel');input.value='/project/report.html';title.value='Report';
 return {c,details,form,input,title,submit,error,retry,cancel,document,Node,calls,reads,messages,get renders(){return renders;},send:()=>form.onsubmit({preventDefault(){}})};
}
test('attachment uses strict named file/url commands, freezes pending input and prevents duplicate writes',async()=>{
 for(const url of [false,true]){const pending=deferred(),f=fixture({command:()=>pending.promise});if(url)f.input.value='https://example.invalid/report';const action=f.send();await tick();assert.equal(f.input.disabled,true);assert.equal(f.title.disabled,true);assert.equal(f.submit.disabled,true);await f.send();assert.equal(f.calls.length,1);assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])),{path:'/api/commands/invoke',args:{name:url?'artifacts.attach-url':'artifacts.attach-file',args:{recordId:'chat',title:'Report',...(url?{url:'https://example.invalid/report'}:{path:'/project/report.html'})}}});pending.resolve({artifactId:'exact-artifact',saved:true});await tick();f.reads[0].resolve({revision:'two',sessions:[]});await action;assert.equal(f.details.open,false);assert.equal(f.renders,1);}
});
test('rejected attachment preserves editable draft and does not start refresh reads',async()=>{
 const f=fixture({command:async()=>{throw Error('File is outside the project');}});await f.send();assert.equal(f.input.value,'/project/report.html');assert.equal(f.title.value,'Report');assert.equal(f.submit.disabled,false);assert.equal(f.input.disabled,false);assert.match(f.error.textContent,/outside/);assert.equal(f.reads.length,0);assert.equal(f.retry.hidden,true);
});
test('accepted attachment refresh failure offers read-only recovery without replaying the saved command',async()=>{
 let libraries=0;const f=fixture({library:async()=>++libraries===1?{accepted:false,error:'Library unavailable'}:{accepted:true}});const action=f.send();await tick();f.reads[0].resolve({revision:'two',sessions:[]});await action;assert.match(f.error.textContent,/exact-artifact.*saved locally.*Library unavailable/);assert.equal(f.retry.hidden,false);assert.equal(f.submit.disabled,true);void f.send();await tick();assert.equal(f.calls.filter(x=>x.path==='/api/commands/invoke').length,1);const retry=f.retry.onclick();await tick();f.reads[1].resolve({revision:'three',sessions:[]});await retry;assert.equal(libraries,2);assert.equal(f.calls.filter(x=>x.path==='/api/commands/invoke').length,1);assert.equal(f.details.open,false);
});
test('attachment refresh retries stale snapshots twice without overwriting newer chat metadata',async()=>{
 const f=fixture(),action=f.send();await tick();f.c.state={revision:'newer',sessions:[{id:'chat',title:'New title'}]};f.reads[0].resolve({revision:'old',sessions:[{id:'chat',title:'Old'}]});await tick();assert.equal(f.c.state.sessions[0].title,'New title');assert.equal(f.reads.length,2);f.reads[1].resolve({revision:'latest',sessions:[{id:'chat',title:'New title'}]});await action;assert.equal(f.c.state.sessions[0].title,'New title');assert.equal(f.calls.filter(x=>x.path==='/api/commands/invoke').length,1);
});
test('detached attachment completion never repaints or closes a replacement component',async()=>{
 const f=fixture(),action=f.send();await tick();f.details.isConnected=false;f.reads[0].resolve({revision:'two',sessions:[]});await action;assert.equal(f.renders,0);assert.equal(f.details.open,true);
});
test('actual library final render retains the open attachment form and its saved recovery receipt',async()=>{
 const f=fixture(),context={contains:()=>false,querySelector:()=>f.details.open?f.details:null,querySelectorAll:()=>[],replaceChildren(){f.details.isConnected=false;}},plain=()=>({replaceChildren(){},append(){},classList:{contains:()=>false,toggle(){},remove(){}}}),pane=plain(),heading=plain(),conversation=plain();
 Object.assign(f.c,{source:'agents',selectedAgentKey:'record:chat',agentViewSignature:'record:chat',agentSidebarCollapsed:false,selectedSession:null,document:{activeElement:null,body:plain()},visibleAgentCandidates:()=>[],pendingSessionResumes:new Set(),inventory:{},$:(selector)=>({'#agent-workspace':pane,'.agent-heading':heading,'.agent-conversation':conversation,'.agent-context-grid':context,'#work-list':plain()}[selector])});
 vm.runInContext(workspace.slice(workspace.indexOf('function renderAgentWorkspace('),workspace.indexOf('function collapsibleContextSection(')),f.c);
 vm.runInContext(fs.readFileSync('static/newsletters.js','utf8').split('function isSkillOutput(')[0],f.c);
 let paints=0;f.c.render=()=>{paints++;f.c.renderAgentWorkspace();};
 const originalApi=f.c.api;f.c.api=async(path,args)=>{if(path==='/api/library')throw Error('Library unavailable');return originalApi(path,args);};
 const action=f.send();await tick();assert.equal(paints,1);assert.equal(f.details.isConnected,true);assert.equal(f.details.open,true);assert.equal(f.submit.disabled,true);
 f.reads[0].resolve({revision:'two',sessions:[{id:'chat',title:'Old'}]});await action;
 assert.equal(f.details.isConnected,true);assert.equal(f.retry.hidden,false);assert.match(f.error.textContent,/exact-artifact.*saved locally/);void f.send();await tick();assert.equal(f.calls.filter(call=>call.path==='/api/commands/invoke').length,1);
});

test('leaving during an accepted attachment failure gives global recovery instructions without a missing button',async()=>{
 const f=fixture({library:async()=>({accepted:false,error:'Library unavailable'})}),action=f.send();await tick();f.details.isConnected=false;f.reads[0].resolve({revision:'two',sessions:[]});await action;assert.match(f.messages[0],/exact-artifact.*saved locally.*Refresh the dashboard/);assert.doesNotMatch(f.messages[0],/Use Refresh results/);assert.equal(f.renders,0);
});

test('Cancel and Escape restore the exact surviving attachment opener after its context redraw',()=>{
 for(const action of ['cancel','escape']){
  const f=fixture(),replacement=f.c.inlineAttach({id:'chat',items:[]},'artifact'),otherChat=f.c.inlineAttach({id:'other',items:[]},'artifact'),otherKind=f.c.inlineAttach({id:'chat',items:[]},'work');
  f.document.querySelectorAll=()=>[otherChat,otherKind,replacement];f.c.render=()=>{f.details.isConnected=false;};f.input.focus();let prevented=false,stopped=false;
  if(action==='cancel'){f.cancel.focus();f.cancel.onclick();}else f.details.onkeydown({key:'Escape',preventDefault(){prevented=true;},stopPropagation(){stopped=true;}});
  assert.equal(f.details.open,false);assert.equal(f.document.activeElement,replacement.querySelector('summary'));assert.equal(f.document.activeElement.focusOptions.preventScroll,true);assert.equal(f.calls.length,0);if(action==='escape'){assert.equal(prevented,true);assert.equal(stopped,true);}
 }
});
test('programmatic dismissal preserves outside focus and composing Escape leaves the form open',()=>{
 const f=fixture(),outside=new f.Node('input');outside.focus();f.details.onkeydown({key:'Escape',isComposing:true,preventDefault(){throw Error('IME intercepted');},stopPropagation(){}});assert.equal(f.details.open,true);f.cancel.onclick();assert.equal(f.document.activeElement,outside);
});
test('pending artifact write and presentation cannot be dismissed and accepted recovery never resends',async()=>{
 const pending=deferred(),f=fixture({command:()=>pending.promise}),summary=f.details.querySelector('summary'),group=f.form.querySelector('div');f.input.focus();const action=f.send();await tick();
 assert.equal(f.document.activeElement,summary);assert.equal(summary.focusOptions.preventScroll,true);assert.equal(group.attrs.role,'group');assert.equal(group.attrs['aria-label'],'Attach result actions');assert.match(group.className,/ui-actiongroup/);assert.match(f.input.className,/ui-field/);assert.equal(f.cancel.disabled,true);assert.equal(f.error.attrs.role,'status');assert.equal(f.error.hidden,false);assert.match(f.error.textContent,/Attaching/);
 const escape=()=>f.details.onkeydown({key:'Escape',preventDefault(){},stopPropagation(){}});f.cancel.onclick();escape();summary.onclick({preventDefault(){}});assert.equal(f.details.open,true);f.details.open=false;f.details.ontoggle();assert.equal(f.details.open,true);
 pending.resolve({artifactId:'accepted',recordId:'chat',saved:true});await tick();escape();assert.equal(f.details.open,true);f.reads[0].reject(Error('Read failed'));await action;assert.equal(f.cancel.disabled,false);assert.equal(f.submit.disabled,true);assert.equal(f.retry.hidden,false);assert.match(f.error.textContent,/saved locally/);f.cancel.focus();f.cancel.onclick();assert.equal(f.details.open,false);await f.send();assert.equal(f.calls.filter(call=>call.path==='/api/commands/invoke').length,1);
});

test('successful save and read-only recovery restore the replacement opener without stealing outside focus',async()=>{
 for(const recovery of [false,true])for(const outside of [false,true]){
  let libraries=0;const f=fixture({library:async()=>({accepted:!recovery||++libraries>1,error:'Library failed'})}),replacement=f.c.inlineAttach({id:'chat',items:[]},'artifact');f.document.querySelectorAll=()=>[replacement];f.c.render=()=>{f.details.isConnected=false;};f.input.focus();const first=f.send();await tick();
  if(recovery){f.reads[0].resolve({revision:'two',sessions:[]});await first;f.retry.focus();const retry=f.retry.onclick();await tick();const external=new f.Node('input');if(outside)external.focus();f.reads[1].resolve({revision:'three',sessions:[]});await retry;assert.equal(f.document.activeElement,outside?external:replacement.querySelector('summary'));}
  else{const external=new f.Node('input');if(outside)external.focus();f.reads[0].resolve({revision:'two',sessions:[]});await first;assert.equal(f.document.activeElement,outside?external:replacement.querySelector('summary'));}
  assert.equal(f.details.open,false);assert.equal(f.calls.filter(call=>call.path==='/api/commands/invoke').length,1);
 }
});
test('actual workspace redraw preserves a dismissed opener through subsequent polls without stealing other focus',()=>{
 for(const kind of ['work','artifact'])for(const action of ['escape','cancel']){
  const f=fixture(),body=new f.Node('body'),pane=new f.Node('section'),heading=new f.Node('div'),conversation=new f.Node('div'),context=new f.Node('div');f.document.body=body;body.append(pane);pane.append(heading,conversation,context);
  const descendants=node=>node.children.flatMap(child=>[child,...descendants(child)]),matches=(node,selector)=>selector==='[data-focus-key]'?!!node.dataset.focusKey:selector.startsWith('.')?node.className.split(' ').includes(selector.slice(1).replace('[open]',''))&&(!selector.includes('[open]')||node.open):node.tagName===selector;
  f.Node.prototype.querySelectorAll=function(selector){return descendants(this).filter(node=>matches(node,selector));};f.Node.prototype.querySelector=function(selector){return this.querySelectorAll(selector)[0]||null;};
  f.Node.prototype.replaceChildren=function(...children){for(const child of descendants(this))child.isConnected=false;if(this.contains(f.document.activeElement))f.document.activeElement=body;this.children=[];this.append(...children);};
  f.document.querySelectorAll=selector=>context.querySelectorAll(selector);
  Object.defineProperty(f.Node.prototype,'classList',{get(){return {contains:()=>false,toggle(){},remove(){}};}});
  const record={id:'chat',items:[]};f.c.state={sessions:[record],artifacts:[]};Object.assign(f.c,{el:(...args)=>{const node=new f.Node(...args);if(node.tagName==='details')node.open=false;return node;},source:'agents',selectedAgentKey:'record:chat',agentViewSignature:'record:chat',agentSidebarCollapsed:false,visibleAgentCandidates:()=>[],pendingSessionResumes:new Set(),inventory:{},agentWork:()=>[],linkedWorkCollapsed:false,agentContextReveals:new Set(['chat:work','chat:artifact']),artifactLibrary:null,collapsibleContextSection:section=>section,persist(){},$:(selector)=>({'#agent-workspace':pane,'.agent-heading':heading,'.agent-conversation':conversation,'.agent-context-grid':context,'#work-list':body}[selector])});
  vm.runInContext(workspace.slice(workspace.indexOf('function renderAgentWorkspace('),workspace.indexOf('function collapsibleContextSection(')),f.c);f.c.render=()=>f.c.renderAgentWorkspace();f.c.render();
  let details=context.querySelectorAll('.inline-attach').find(node=>node.dataset.attachKind===kind);details.open=true;details.querySelector('input').focus();
  if(action==='escape')details.onkeydown({key:'Escape',preventDefault(){},stopPropagation(){}});else details.querySelectorAll('button').find(node=>node.textContent==='Cancel').onclick();
  const key='chat:attach:'+kind;assert.equal(f.document.activeElement.dataset.focusKey,key);const prior=f.document.activeElement;f.c.render();assert.notEqual(f.document.activeElement,prior);assert.equal(f.document.activeElement.dataset.focusKey,key);assert.equal(f.document.activeElement.focusOptions.preventScroll,true);
  const outside=new f.Node('input');outside.focus();f.c.render();assert.equal(f.document.activeElement,outside);
  const terminal=new f.Node('textarea');terminal.focus();f.c.render();assert.equal(f.document.activeElement,terminal);
  context.querySelectorAll('[data-focus-key]').find(node=>node.dataset.focusKey===key).focus();record.id='other';f.c.selectedAgentKey='record:other';f.c.agentViewSignature='record:other';f.c.agentContextReveals=new Set(['other:work','other:artifact']);f.c.render();assert.equal(f.document.activeElement,body);
 }
});
