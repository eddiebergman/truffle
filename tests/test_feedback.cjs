'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/feedback.js','utf8'),chat=fs.readFileSync('static/app-chat.js','utf8');
async function fixture({draft='',pending=false,attachments=0,scrollTop=500,setup=false,hidden=false,viewport=500,starEligible=false,page='all',candidates=[],record=null}={}){
 const closeEvents=[];
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.value='';this.children=[];this.attrs={};this.hidden=false;this.open=false;this.listeners={};}
  append(...nodes){for(const node of nodes){node.parent=this;this.children.push(node);}}
  replaceChildren(...nodes){this.children=[];this.append(...nodes);}
  setAttribute(key,value){this.attrs[key]=value;}
  getAttribute(key){return this.attrs[key]||null;}
  querySelector(selector){return this.children.flatMap(node=>[node,...node.descendants()]).find(node=>node.matches(selector))||null;}
  descendants(){return this.children.flatMap(node=>[node,...node.descendants()]);}
  matches(selector){if(selector==='dialog[open]')return this.tagName==='DIALOG'&&this.open;if(selector.startsWith('#'))return this.id===selector.slice(1);if(selector.startsWith('.'))return this.className.split(' ').includes(selector.slice(1));return this.tagName===selector.toUpperCase();}
  before(node){const index=this.parent.children.indexOf(this);node.parent=this.parent;this.parent.children.splice(index,0,node);}
  remove(){this.parent.children=this.parent.children.filter(node=>node!==this);}
  showModal(){this.open=true;}
  addEventListener(type,callback,options){(this.listeners[type]??=[]).push({callback,once:options?.once});}
  close(){this.open=false;if(document.activeElement)document.activeElement.focused=false;document.activeElement=body;closeEvents.push(()=>{for(const listener of this.listeners.close||[])listener.callback();this.listeners.close=(this.listeners.close||[]).filter(listener=>!listener.once);});}
  focus(){if(document.activeElement)document.activeElement.focused=false;this.focused=true;document.activeElement=this;}
  closest(selector){for(let node=this;node;node=node.parent)if(selector.split(',').some(part=>node.matches(part)))return node;return null;}
 }
 const body=new Node('body'),panel=new Node('section');panel.id='app-chat-panel';panel.hidden=hidden;
 const conversation=new Node('div','app-chat-conversation'),messages=new Node('div','app-chat-messages'),assistant=new Node('article','is-assistant'),inline=new Node('section');inline.id='app-chat-setup';inline.hidden=!setup;messages.setAttribute('aria-busy','false');messages.append(assistant);conversation.append(messages,inline);
 const form=new Node('form','app-chat-form'),input=new Node('textarea');input.value=draft;form.append(input);const newChat=new Node('button','app-chat-new');panel.append(newChat,conversation,form);body.append(panel);
 let position=scrollTop,writes=0;Object.defineProperties(conversation,{scrollHeight:{get:()=>1000},clientHeight:{get:()=>viewport-(body.querySelector('.app-feedback-hint')?80:0)},scrollTop:{get:()=>Math.min(position,1000-conversation.clientHeight),set:value=>{writes++;position=Math.max(0,Math.min(value,1000-conversation.clientHeight));}}});
 let clock=1000000;const calls=[],intervals=[],prepared=[];const feedback={progress:{revision:1,hint:'unseen',star:'unseen'},config:{repository:'https://example.invalid/repo',feedbackText:'Help improve Truffle'},starEligible};
 const document={body,hidden:false,activeElement:null,querySelector:selector=>body.querySelector(selector),addEventListener(){}};
 const c=vm.createContext({window:{truffleChatImages:{count:()=>attachments}},document,Date:{now:()=>clock},TextEncoder,URLSearchParams,source:page,liveCandidates:()=>candidates,currentAgentRecord:()=>record,pending,open:false,input,showError:()=>{},update:()=>prepared.push('update'),api:async(path,payload)=>{calls.push({path,payload});if(payload)feedback.progress={...feedback.progress,revision:2,hint:'dismissed'};return structuredClone(feedback);},el:(tag,cls,text)=>new Node(tag,cls,text),featureButton:(text,onclick,cls)=>Object.assign(new Node('button',cls,text),{onclick}),$:selector=>document.querySelector(selector),toast(){},setInterval:fn=>intervals.push(fn)});
 vm.runInContext(chat.slice(chat.indexOf('  window.prepareAppChatMessage='),chat.indexOf('  window.openAppChatSetup=')),c);vm.runInContext(source,c);await Promise.resolve();await Promise.resolve();
 const openDraft=(title='Feedback title',description='Feedback body')=>{c.window.openFeedbackDraft({title,body:description});const dialog=document.querySelector('#feedback-draft');return {dialog,title:dialog.querySelector('input'),body:dialog.querySelector('textarea'),ask:dialog.descendants().find(node=>node.textContent==='Ask the app assistant'),error:dialog.descendants().find(node=>node.attrs.role==='alert')};};
 return {c,document,panel,conversation,input,inline,calls,prepared,openDraft,advance:milliseconds=>{clock+=milliseconds;},flushClose:()=>{for(const event of closeEvents.splice(0))event();},check:intervals[0],get writes(){return writes;}};
}
test('feedback Ask prepares a full unsent draft through the existing helper and closes only after success',async()=>{
 const f=await fixture(),draft=f.openDraft('A precise title','Reproduction\nwith a second line.'),calls=f.calls.length;draft.ask.onclick();f.flushClose();assert.equal(f.input.value,'Help me draft a Truffle issue about: A precise title\n\nReproduction\nwith a second line.');assert.equal(f.c.open,true);assert.equal(f.input.focused,true);assert.equal(draft.dialog.open,false);assert.equal(f.calls.length,calls);assert.equal(draft.title.value,'A precise title');assert.equal(draft.body.value,'Reproduction\nwith a second line.');
});
test('feedback Ask preserves source, draft and attachments when preparation is rejected',async()=>{
 for(const options of [{draft:'Existing unsent message'},{pending:true},{attachments:1}]){
  const f=await fixture(options),draft=f.openDraft(),calls=f.calls.length;draft.ask.onclick();assert.equal(draft.dialog.open,true);assert.equal(draft.title.value,'Feedback title');assert.equal(draft.body.value,'Feedback body');assert.equal(f.input.value,options.draft||'');assert.match(draft.error.textContent,/Wait for|Send or clear/);assert.equal(f.c.window.truffleChatImages.count(),options.attachments||0);assert.equal(f.calls.length,calls);assert.deepEqual(f.prepared,[]);
 }
});
test('feedback Ask rejects complete oversized UTF8 messages without truncating or closing the source',async()=>{
 const f=await fixture(),text='é'.repeat(2100),draft=f.openDraft('Long report',text),calls=f.calls.length;draft.ask.onclick();assert.equal(draft.dialog.open,true);assert.equal(draft.body.value,text);assert.equal(f.input.value,'');assert.match(draft.error.textContent,/too long/);assert.equal(f.calls.length,calls);
 delete f.c.window.prepareAppChatMessage;draft.ask.onclick();assert.match(draft.error.textContent,/unavailable/);assert.equal(draft.dialog.open,true);assert.equal(draft.body.value,text);
});
test('feedback hint insertion and removal retain follow-bottom only within two pixels',async()=>{
 for(const near of [500,498]){
  const f=await fixture({scrollTop:near});await f.check();assert.equal(f.conversation.scrollTop,580);assert.equal(f.writes,1);const hint=f.document.querySelector('.app-feedback-hint'),close=hint.descendants().find(node=>node.attrs['aria-label']==='Dismiss feedback suggestion');await close.onclick();assert.equal(f.conversation.scrollTop,500);assert.equal(f.writes,2);assert.equal(f.document.querySelector('.app-feedback-hint'),null);
 }
});
test('feedback hint keeps reader position and inline setup; hidden or zero-height panels do not count as following',async()=>{
 for(const options of [{scrollTop:497},{scrollTop:120},{setup:true},{hidden:true},{viewport:0}]){
  const f=await fixture(options);const before=f.conversation.scrollTop;await f.check();assert.equal(f.writes,0);assert.equal(f.conversation.scrollTop,before);const hint=f.document.querySelector('.app-feedback-hint');if(hint){const close=hint.descendants().find(node=>node.attrs['aria-label']==='Dismiss feedback suggestion');await close.onclick();assert.equal(f.writes,0);assert.equal(f.conversation.scrollTop,before);}
 }
});

test('feedback focuses the prepared composer after native dialog close, without stealing focus from another dialog',async()=>{
 const f=await fixture(),draft=f.openDraft();draft.ask.onclick();assert.notEqual(f.document.activeElement,f.input);f.flushClose();assert.equal(f.document.activeElement,f.input);
 for(const blocked of ['hidden','other-dialog']){
  const next=await fixture(),report=next.openDraft();report.ask.onclick();
  if(blocked==='hidden')next.panel.hidden=true;else next.openDraft('Another report');
  const active=next.document.activeElement;next.flushClose();assert.equal(next.document.activeElement,active,blocked);
 }
});


test('two-day star prompt reaches idle empty Agents without interrupting live or selected terminal owners',async()=>{
 for(const scenario of [
  {name:'empty Agents',page:'agents',shown:true},
  {name:'working main terminal',page:'agents',candidates:[{live:true,lifecycle:'working'}],shown:false},
  {name:'unselected ready terminal',page:'agents',candidates:[{live:true,lifecycle:'idle'}],shown:false},
  {name:'selected chat with companion shell',page:'agents',record:{id:'saved'},shown:false},
  {name:'other page',page:'all',shown:true}
 ]){const f=await fixture({...scenario,hidden:true,starEligible:true});f.advance(20000);await f.check();assert.equal(!!f.document.querySelector('#feedback-star'),scenario.shown,scenario.name);assert.equal(f.calls.filter(call=>call.payload).length,0,'showing the prompt does not write or launch anything');}
});

test('eligible empty Agents retains idle, typing, modal, App chat and terminal-focus protections',async()=>{
 for(const guard of ['recent interaction','typing','modal','App chat','main terminal focus','secondary terminal focus']){
  const f=await fixture({hidden:true,starEligible:true,page:'agents'});if(guard!=='recent interaction')f.advance(20000);
  if(guard==='App chat')f.panel.hidden=false;
  if(guard==='modal')f.openDraft();
  if(guard==='typing')f.input.focus();
  if(guard==='main terminal focus'||guard==='secondary terminal focus'){const terminal=f.c.el('div','embedded-terminal'),focused=f.c.el('div','xterm-screen');terminal.id=guard==='main terminal focus'?'main-terminal':'companion-terminal';terminal.append(focused);f.document.body.append(terminal);focused.focus();}
  await f.check();assert.equal(f.document.querySelector('#feedback-star'),null,guard);
 }
});
