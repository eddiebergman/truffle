const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),workspace=fs.readFileSync('static/workspace.js','utf8'),sessions=fs.readFileSync('static/sessions.js','utf8');
function fixture(){
 const calls=[],messages=[],targets=new Map(),record={id:'saved-chat',title:'Original',hidden:false,archivedAt:null,privateExtension:{retained:true}},candidate={recordId:'saved-chat'},undo=[];let fail=false,renders=0,terminalFocus=0;
 class Node{constructor(tag,cls='',text=''){this.tag=tag;this.textContent=text;this.children=[];this.dataset={};this.value='';}append(...nodes){this.children.push(...nodes);}setAttribute(){}remove(){this.removed=true;}focus(){this.focused=true;}select(){}setCustomValidity(value){this.validation=value;}reportValidity(){}}
 const $=selector=>{if(!targets.has(selector))targets.set(selector,new Node('div'));return targets.get(selector);};
 const c=vm.createContext({source:'agents',agentRename:null,state:{sessions:[record],unrelated:'kept'},selectedSession:record,selectedAgentKey:'record:saved-chat',currentItem:null,document:{createElementNS:()=>new Node('svg'),querySelectorAll:()=>[]},$ ,el:(...args)=>new Node(...args),liveCandidates:()=>[candidate],recordForCandidate:()=>record,candidateKey:()=> 'record:saved-chat',terminalViews:new Map([['saved-chat',{terminal:{focus(){terminalFocus++;}}}]]),render(){renders++;},toast:(message,callback)=>{messages.push(message);if(callback)undo.push(callback);},api:async(path,payload)=>{calls.push({path,payload});if(fail)throw new Error('Metadata failed');const previous=c.state.sessions[0],args=payload.args,chat={id:previous.id,title:args.title??previous.title,hidden:args.hidden??previous.hidden,archivedAt:Object.hasOwn(args,'archived')?(args.archived?123:null):previous.archivedAt,updatedAt:124};return {chat,scope:'local'};}});
 vm.runInContext(app.slice(app.indexOf('function localWriteMessage('),app.indexOf('async function mutateChatMetadata(')),c);
 vm.runInContext(app.slice(app.indexOf('async function mutateChatMetadata('),app.indexOf('function workStatusLabel(')),c);
 return {c,calls,messages,record,undo,$,Node,fail(){fail=true;},renders:()=>renders,terminalFocus:()=>terminalFocus};
}
test('chat visibility and Undo use commands while immutable receipts preserve captured state and other metadata',async()=>{
 const f=fixture();vm.runInContext(app.slice(app.indexOf('function visibilityButton('),app.indexOf('function renderHiddenItems(')),f.c);const button=f.c.visibilityButton(f.record,true);await button.onclick();
 assert.equal(f.calls[0].path,'/api/commands/invoke');assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].payload)),{name:'chats.visibility',args:{id:'saved-chat',hidden:true}});assert.equal(f.record.hidden,false);assert.equal(f.c.state.sessions[0].hidden,true);assert.equal(f.c.state.sessions[0].privateExtension,f.record.privateExtension);assert.equal(f.c.state.unrelated,'kept');assert.equal(f.messages[0],'Hidden.');
 await f.undo[0]();assert.equal(f.calls[1].payload.args.hidden,false);assert.equal(f.c.state.sessions[0].hidden,false);f.fail();await button.onclick();assert.equal(button.disabled,false);assert.equal(f.c.state.sessions[0].hidden,false);assert.equal(f.messages.at(-1),'Metadata failed');
});
test('chat archive and restore use explicit command booleans without closing the terminal',async()=>{
 const f=fixture();f.$('#session-dialog').close=()=>{};const start=sessions.indexOf("$('#session-archive').onclick=");vm.runInContext(sessions.slice(start,sessions.indexOf('for(const button of document.querySelectorAll',start)),f.c);
 await f.$('#session-archive').onclick();assert.equal(f.calls[0].payload.name,'chats.archive');assert.equal(f.calls[0].payload.args.archived,true);assert.equal(f.c.state.sessions[0].archivedAt,123);assert.match(f.messages[0],/Archived/);
 f.c.selectedSession=f.c.state.sessions[0];await f.$('#session-archive').onclick();assert.equal(f.calls[1].payload.args.archived,false);assert.equal(f.c.state.sessions[0].archivedAt,null);assert.equal(f.messages[1],'Session restored.');assert.equal(f.terminalFocus(),0);assert.ok(f.calls.every(call=>call.path==='/api/commands/invoke'));
});
test('inline chat rename uses the command receipt, retains edits on failure and focuses terminal only after success',async()=>{
 const f=fixture();vm.runInContext(workspace.slice(workspace.indexOf('function beginAgentRename('),workspace.indexOf('function agentActionsMenu(')),f.c);f.c.beginAgentRename();const input=f.c.agentRename.input;input.value='Renamed';await input.onkeydown({key:'Enter',preventDefault(){},stopPropagation(){}});
 assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].payload)),{name:'chats.rename',args:{id:'saved-chat',title:'Renamed'}});assert.equal(f.c.state.sessions[0].title,'Renamed');assert.equal(f.record.title,'Original');assert.equal(f.c.agentRename,null);assert.equal(f.terminalFocus(),1);assert.equal(f.renders(),1);
 f.fail();f.c.beginAgentRename();const retry=f.c.agentRename.input;retry.value='Keep draft';await retry.onkeydown({key:'Enter',preventDefault(){},stopPropagation(){}});assert.equal(retry.value,'Keep draft');assert.equal(retry.disabled,false);assert.equal(retry.focused,true);assert.equal(f.c.state.sessions[0].title,'Renamed');assert.equal(f.terminalFocus(),1);
});

function delayed(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
test('out-of-order independent metadata receipts update only command-owned fields',async()=>{
 const f=fixture(),requests=[];f.c.api=(_path,payload)=>{const pending=delayed();requests.push({payload,...pending});return pending.promise;};
 const hiding=f.c.mutateChatMetadata('chats.visibility',{id:'saved-chat',hidden:true}),renaming=f.c.mutateChatMetadata('chats.rename',{id:'saved-chat',title:'Newer title'}),archiving=f.c.mutateChatMetadata('chats.archive',{id:'saved-chat',archived:true});
 requests[1].resolve({chat:{id:'saved-chat',title:'Newer title',hidden:false,archivedAt:null,updatedAt:200}});await renaming;
 requests[2].resolve({chat:{id:'saved-chat',title:'Stale title',hidden:false,archivedAt:300,updatedAt:100}});await archiving;
 requests[0].resolve({chat:{id:'saved-chat',title:'Original',hidden:true,archivedAt:null,updatedAt:50}});await hiding;
 assert.equal(f.c.state.sessions[0].title,'Newer title');assert.equal(f.c.state.sessions[0].updatedAt,200);assert.equal(f.c.state.sessions[0].archivedAt,300);assert.equal(f.c.state.sessions[0].hidden,true);assert.equal(f.c.state.sessions[0].privateExtension,f.record.privateExtension);
});
test('pending archive retains its original target and intent without closing a newly selected chat dialog',async()=>{
 const f=fixture(),pending=delayed();let closed=0;const dialog=f.$('#session-dialog');dialog.open=true;dialog.close=()=>{closed++;dialog.open=false;};f.c.api=()=>pending.promise;
 const start=sessions.indexOf("$('#session-archive').onclick=");vm.runInContext(sessions.slice(start,sessions.indexOf('for(const button of document.querySelectorAll',start)),f.c);
 const archiving=f.$('#session-archive').onclick();f.c.selectedSession={id:'other',archivedAt:999};pending.resolve({chat:{id:'saved-chat',archivedAt:123,title:'Original',hidden:false,updatedAt:124}});await archiving;
 assert.equal(closed,0);assert.equal(dialog.open,true);assert.equal(f.c.selectedSession.id,'other');assert.match(f.messages.at(-1),/^Archived/);assert.equal(f.c.state.sessions[0].archivedAt,123);
});
test('pending rename completion preserves newer editor identity and does not focus the former terminal',async()=>{
 for(const replaced of [false,true]){
  const f=fixture(),pending=delayed();f.c.api=()=>pending.promise;vm.runInContext(workspace.slice(workspace.indexOf('function beginAgentRename('),workspace.indexOf('function agentActionsMenu(')),f.c);f.c.beginAgentRename();const editor=f.c.agentRename;editor.input.value='Renamed';const saving=editor.input.onkeydown({key:'Enter',preventDefault(){},stopPropagation(){}});
  f.c.selectedAgentKey='record:other';const newer={input:new f.Node('input'),key:'record:other'};if(replaced)f.c.agentRename=newer;
  pending.resolve({chat:{id:'saved-chat',title:'Renamed',hidden:false,archivedAt:null,updatedAt:200}});await saving;
  assert.equal(f.terminalFocus(),0);assert.equal(f.c.state.sessions[0].title,'Renamed');assert.equal(f.c.agentRename,replaced?newer:null);assert.equal(editor.input.removed,true);
 }
});
test('failed rename after a chat switch does not focus its detached editor or cancel another edit',async()=>{
 const f=fixture(),pending=delayed();f.c.api=()=>pending.promise;vm.runInContext(workspace.slice(workspace.indexOf('function beginAgentRename('),workspace.indexOf('function agentActionsMenu(')),f.c);f.c.beginAgentRename();const editor=f.c.agentRename;editor.input.value='Keep draft';const saving=editor.input.onkeydown({key:'Enter',preventDefault(){},stopPropagation(){}});
 const newer={input:new f.Node('input'),key:'record:other'};f.c.selectedAgentKey='record:other';f.c.agentRename=newer;editor.input.focused=false;pending.reject(new Error('Rename failed'));await saving;
 assert.equal(editor.input.focused,false);assert.equal(editor.input.value,'Keep draft');assert.equal(editor.input.disabled,false);assert.equal(f.c.agentRename,newer);assert.equal(f.terminalFocus(),0);assert.equal(f.c.state.sessions[0].title,'Original');
});
