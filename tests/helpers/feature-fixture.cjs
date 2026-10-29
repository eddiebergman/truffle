'use strict';
// Feature write fixture shared by the tests that drive app.js and features.js dialogs.
const {browserContext,loadScripts,rawContext}=require('./static-script.cjs');
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
// Load more whole scripts into a fixture. Their load-time requests never settle and
// their load-time wiring lands on a separate fake document, except on the nodes in
// `elements` (selector to fixture node). Names in `keepHost` use the fixture's
// host value during load, as in loadScripts. Names in `drop` are
// fixture stubs that the real scripts replace. loadScripts restores the context
// properties, which go stale once a loaded script's top-level let shadows them, so
// refresh them from the live bindings first.
function loadMore(f,files,{drop=[],elements={},keepHost=[]}={}){
 const raw=rawContext(f.c),api=f.c.api;for(const name of drop)delete raw[name];
 for(const name of Object.keys(raw))if(/^[A-Za-z_$][\w$]*$/.test(name))raw[name]=f.c[name];
 raw.api=()=>new Promise(()=>{});
 try{return loadScripts(f.c,files,{elements,keepHost});}finally{f.c.api=api;}
}
function scriptFixture({handler,renderFailure=false,prefs={}}={}){
 const {MutationObserver,recordOpenChange}=require('./dialog-mutations.cjs')();
 let document,c,serial=0,renders=0;const nodes=new Map(),calls=[],messages=[],opened=[],pendingClose=[];
 const match=(node,selector)=>{selector=selector.trim();if(selector.startsWith('#'))return node.id===selector.slice(1);if(selector==='dialog[open]')return node.tagName==='dialog'&&node.open;if(selector==='[data-feature-write]')return !!node.dataset.featureWrite;if(selector.startsWith('.'))return node.className.split(' ').includes(selector.slice(1));const tag=selector.match(/^\w+/)?.[0];if(tag&&node.tagName!==tag)return false;if(selector.includes(':not(:disabled)')&&node.disabled)return false;if(selector.includes(':not([type="hidden"])')&&node.type==='hidden')return false;if(selector.includes('[type="submit"]')&&node.type!=='submit')return false;if(selector.includes('[href]')&&!node.href)return false;return !!tag;};
 class Node{
  constructor(tag='div',cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.events={};this.value='';this.open=false;this.hidden=false;this._disabled=false;this.classList={contains:name=>this.className.split(' ').includes(name),add:name=>{this.className+=' '+name;}};}
  get open(){return this._open||false;}set open(value){if(this._open!==value){this._open=value;recordOpenChange(this);}}
  get textContent(){return this._text||'';}set textContent(value){this._text=value;if(this.children)for(const child of [...this.children])child.remove();}
  get isConnected(){return this===document.body||!!this.parentElement?.isConnected;}set disabled(value){this._disabled=value;if(value&&document?.activeElement===this)document.activeElement=document.body;}get disabled(){return this._disabled;}
  append(...children){for(const child of children){child.parentElement=this;this.children.push(child);}}replaceChildren(...children){for(const child of this.children)child.remove();this.children=[];this.append(...children);if(this.tagName==='select')this.value=children[0]?.value||'';}add(child){this.append(child);}get childElementCount(){return this.children.length;}
  remove(){if(this.contains(document.activeElement))document.activeElement=document.body;if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(child=>child!==this);this.parentElement=null;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}matches(selector){return selector.split(',').some(part=>match(this,part));}closest(selector){let node=this;while(node){if(node.matches(selector))return node;node=node.parentElement;}return null;}
  querySelectorAll(selector){return this.children.flatMap(node=>[...(node.matches(selector)?[node]:[]),...node.querySelectorAll(selector)]);}querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  setAttribute(key,value){this.attrs[key]=value;}addEventListener(type,fn){(this.events[type]??=[]).push(fn);}removeEventListener(type,fn){this.events[type]=(this.events[type]||[]).filter(listener=>listener!==fn);}dispatchEvent(event){for(const fn of this.events[event.type]||[])fn(event);return !event.defaultPrevented;}
  focus(options){if(!this.disabled){document.activeElement=this;this.focusOptions=options;}}showModal(){this.open=true;}close(){if(!this.open)return;this.open=false;if(this.contains(document.activeElement))document.activeElement=document.body;for(const fn of this.events.close||[])fn();}
 }
 document={body:new Node('body'),activeElement:null,createElement:tag=>new Node(tag),querySelectorAll:selector=>document.body.querySelectorAll(selector),querySelector:selector=>/^#[\w-]+$/.test(selector)?$(selector):document.body.querySelector(selector),listeners:{},addEventListener(type,fn){(this.listeners[type]??=[]).push(fn);}};document.activeElement=document.body;
 const $=id=>{if(!nodes.has(id)){const node=new Node();node.id=id.slice(1);nodes.set(id,node);document.body.append(node);}return nodes.get(id);};
 const makeForm=(name,names)=>{const dialog=$('#'+name+'-dialog');dialog.tagName='dialog';const form=$('#'+name+'-form');form.tagName='form';form.remove();dialog.append(form);form.elements={};const close=new Node('button','close-dialog'),save=new Node('button');save.type='submit';form.append(close);for(const field of names){const control=new Node(field==='record'||field==='duration'?'select':['body','done','blocked','next'].includes(field)?'textarea':'input');control.name=field;control.type='text';form.elements[field]=control;form.append(control);}form.append(save);form.reset=()=>{for(const [key,node]of Object.entries(form.elements))node.value=key==='duration'?'day':['days','minUsers','minEvents'].includes(key)?'0':'';};form.reset();return {dialog,form,close,save};};
 const sentry=makeForm('sentry-filter',['minUsers','minEvents','days','level','category','environment','viewName']),briefing=makeForm('briefing',['record','body']),checkpoint=makeForm('checkpoint',['done','blocked','next']),snooze=makeForm('snooze',['duration','until']);sentry.form.append($('#sentry-clear'),$('#sentry-delete'));$('#sentry-delete').tagName='button';$('#sentry-clear').tagName='button';
 let canonical={revision:1,sources:{sentry:{disabled:false}},sentryEnvironments:{},items:[],notes:[],sessions:[{id:'chat:a',title:'A',workflow:'work-review',agent:'claude',createdAt:1,items:[]},{id:'chat:b',title:'B',workflow:'work-review',agent:'codex',createdAt:2,items:[]}],sentryViews:[],snoozes:{},changes:[],briefings:[]};
 const clone=value=>JSON.parse(JSON.stringify(value));
 const apply=payload=>{canonical={...canonical,revision:canonical.revision+1};if(payload.name==='changes.mark-read')canonical.changes=canonical.changes.map(change=>payload.args.ids.includes(change.id)?{...change,read:true}:change);if(payload.action==='save-sentry-view')canonical.sentryViews=[...canonical.sentryViews.filter(row=>row.id!==payload.id),{id:payload.id,name:payload.name,filters:clone(payload.filters)}];if(payload.action==='delete-sentry-view')canonical.sentryViews=canonical.sentryViews.filter(row=>row.id!==payload.id);if(payload.action==='save-briefing')canonical.briefings.push({id:'report',sessionId:payload.id,body:payload.body});if(payload.action==='checkpoint')canonical.sessions=canonical.sessions.map(row=>row.id===payload.id?{...row,checkpoint:payload}:row);return clone(canonical);};
 c=browserContext({settingsOpenGeneration:0,document,localStorage:{getItem:key=>key==='truffle-view'?JSON.stringify(prefs):null,setItem(){}},Event,MutationObserver,WeakSet,Map,Date,JSON,navigator:{platform:'Mac'},prefs,source:'sentry',state:clone(canonical),currentItem:null,crypto:{randomUUID:()=>`00000000-0000-4000-8000-${String(++serial).padStart(12,'0')}`},Option:class extends Node{constructor(text,value){super('option','',text);this.value=value;}},toast:message=>messages.push(message),persist(){},render:()=>{renders++;if(renderFailure)throw Error('Render unavailable');},loadInventory(){},allItems:()=>c.state.items,workReference:item=>item.reference||item.id,githubRole:()=>null,ciInfo:()=>({label:'Checks'}),agentName:agent=>agent,sessionRuntime:()=>({label:'Saved'}),relative:()=> 'now',scopeKey:()=>'',matchesSource:()=>true,matchesScope:()=>true,showSession:session=>opened.push({session:session.id}),openSavedSession(){},quickStart(){},api:async(path,payload)=>{calls.push({path,payload:payload&&clone(payload)});if(handler)return handler(path,payload,{apply,snapshot:()=>clone(canonical)});return path==='/api/edit'||path==='/api/commands/invoke'?apply(payload):clone(canonical);}});
 // Load-time lookups use the fixture document, so the scripts wire the fixture nodes.
 loadScripts(c,['static/client.js','static/app.js','static/features.js'],{keepHost:['document','Event','MutationObserver','Option','navigator','localStorage']});
 // Stubbed after load, so tests of the real opener can restore it.
 const actual={openItem:c.openItem};c.openItem=item=>opened.push({item:item.id});
 const submit=part=>part.form.onsubmit({preventDefault(){},currentTarget:part.form});
 const recovery=part=>{const region=part.form.querySelector('[data-feature-write]');return {region,message:region?.querySelector('p'),button:text=>region?.querySelectorAll('button').find(button=>button.textContent===text)};};
 return {c,actual,Node,$,sentry,briefing,checkpoint,snooze,document,calls,messages,opened,submit,recovery,pendingClose,get renders(){return renders;},get uuids(){return serial;},setCanonical:next=>{canonical=clone(next);c.state=clone(next);},snapshot:()=>clone(canonical)};
}
module.exports={scriptFixture,loadMore,deferred,tick};
