const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),review=fs.readFileSync('static/diff-review.js','utf8'),git=fs.readFileSync('static/git-diff.js','utf8'),workspace=fs.readFileSync('static/workspace.js','utf8');
function fixture(){
 const queued=[];let document;
 const matches=(node,selector)=>selector.split(',').some(part=>{
  part=part.trim();if(part==='[hidden]')return node.hidden;if(part==='dialog:not([open])')return node.tagName==='dialog'&&!node.open;
  if(part==='[data-focus-key]')return !!node.dataset.focusKey;if(part.startsWith('#'))return node.id===part.slice(1);
  const open=part.endsWith('[open]');if(open)part=part.slice(0,-6);return (!open||node.open)&&(part.startsWith('.')?node.className.split(' ').includes(part.slice(1)):node.tagName===part);
 });
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.dataset={};this.children=[];this.attrs={};this.style={};this.hidden=false;this.disabled=false;this.open=false;this.events={};this.classList={contains:name=>this.className.split(' ').includes(name),toggle(){},remove(){}};}
  get isConnected(){return this===document.body||!!this.parentElement?.isConnected;}
  get firstChild(){return this.children[0];}get parentNode(){return this.parentElement;}
  append(...nodes){for(const node of nodes){node.remove();this.children.push(node);node.parentElement=this;}}
  insertBefore(node,before){node.remove();const index=this.children.indexOf(before);if(index<0)this.children.push(node);else this.children.splice(index,0,node);node.parentElement=this;}
  prepend(...nodes){for(const node of nodes.reverse()){node.remove();this.children.unshift(node);node.parentElement=this;}}
  remove(){if(this.parentElement){this.parentElement.children=this.parentElement.children.filter(node=>node!==this);this.parentElement=null;}}
  replaceChildren(...nodes){if(this.contains(document.activeElement))document.activeElement=document.body;for(const child of this.children)child.parentElement=null;this.children=[];this.append(...nodes);}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(matches(child,selector)?[child]:[]),...child.querySelectorAll(selector)]);}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  matches(selector){return selector===':disabled'?this.disabled:matches(this,selector);}
  closest(selector){let node=this;while(node){if(matches(node,selector))return node;node=node.parentElement;}return null;}
  getClientRects(){if(!this.isConnected||this.closest('[hidden],dialog:not([open])'))return [];let child=this,parent=this.parentElement;while(parent){if(parent.tagName==='details'&&!parent.open&&child.tagName!=='summary')return [];child=parent;parent=parent.parentElement;}return [{}];}
  setAttribute(key,value){this.attrs[key]=value;}removeAttribute(key){delete this.attrs[key];}
  focus(options){document.activeElement=this;this.focusOptions=options;}
  dispatchEvent(event){for(const fn of this.events[event.type]||[])fn(event);return !event.defaultPrevented;}
  add(node){this.append(node);}addEventListener(name,listener){(this.events[name]??=[]).push(listener);}
  show(){this.open=true;this.focus();}showModal(){this.show();}
  close(){if(!this.open)return;this.open=false;if(this.contains(document.activeElement))document.activeElement=document.body;queued.push(()=>{for(const fn of this.events.close||[])fn();});}
 }
 document={body:null,activeElement:null,hidden:false,querySelectorAll:selector=>document.body.querySelectorAll(selector),querySelector:selector=>document.body.querySelector(selector)};document.body=new Node('body');document.activeElement=document.body;
 const pane=new Node('section'),heading=new Node('div'),conversation=new Node('div'),context=new Node('div'),dialog=new Node('dialog'),internal=new Node('button');pane.id='agent-workspace';heading.className='agent-heading';conversation.className='agent-conversation';context.className='agent-context-grid';dialog.id='diff-review-dialog';pane.append(heading,conversation,context);dialog.append(internal);document.body.append(pane,dialog);
 const ids=new Map([['#agent-workspace',pane],['#work-list',document.body],['#diff-review-dialog',dialog]]);
 const $=(selector,root)=>{if(root)return root.querySelector(selector);if(['.agent-heading','.agent-conversation','.agent-context-grid'].includes(selector))return pane.querySelector(selector);if(!ids.has(selector)){const node=new Node('div');node.id=selector.slice(1);dialog.append(node);ids.set(selector,node);}return ids.get(selector);};
 const record={id:'chat',title:'Chat',cwd:'/repo',items:[]};
 const c=vm.createContext({document,Event,$,getComputedStyle:node=>{let current=node;while(current){if(current.style.visibility==='hidden')return {visibility:'hidden'};current=current.parentElement;}return {visibility:'visible'};},source:'agents',selectedAgentKey:'record:chat',state:{sessions:[record],artifacts:[]},prefs:{},WeakMap,Map,Set,JSON,requestAnimationFrame(){},queueMicrotask(){},setInterval(){},visibleAgentCandidates:()=>[],liveCandidates:()=>[],candidateKey:()=>'',recordForCandidate:()=>null,agentViewSignature:'record:chat',agentSidebarCollapsed:false,pendingSessionResumes:new Set(),savedSessionOpenState:()=>null,savedSessionOpenAction:()=>'Check terminal',inventory:{},agentWork:()=>[],linkedWorkCollapsed:false,agentContextReveals:new Set(),contextSectionsCollapsed:{'git-diff':false},artifactLibrary:null,loadArtifactLibrary(){},inlineAttach:()=>new Node('details','inline-attach'),el:(...args)=>new Node(...args),featureButton:(label,onclick,cls)=>Object.assign(new Node('button',cls,label),{onclick}),agentIcon:()=>new Node('svg'),reviewDrafts:()=>({}),workReference:()=>'',persist(){},Option:class extends Node{constructor(text,value){super('option','',text);this.value=value;}},reviewState:null,rememberReviewUI(){},disposeReview(){},setting:(_key,fallback)=>fallback,localStorage:{getItem:()=>null},savedReviewUI:()=>({}),sharedReviewDrafts:new Map(),hideReviewComposer(){},paintReviewComments(){},setReviewMode(){},positionQuickReview(){},ResizeObserver:class{observe(){}},});
 vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog')")),c);
 vm.runInContext(app.slice(app.indexOf('const dialogReturnFocus='),app.indexOf('function requestDialogClose(')),c);
 vm.runInContext(git,c);
 vm.runInContext(workspace.slice(workspace.indexOf('function renderAgentWorkspace('),workspace.indexOf('function inlineAttach(')),c);
 c.render=()=>c.renderAgentWorkspace();
 const start=review.indexOf('async function openDiffReview('),end=review.indexOf('  try{\n    const filePromise',start);vm.runInContext(review.slice(start,end)+'return current;}',c);
 const closeLine=review.split('\n').find(line=>line.startsWith("$('#diff-review-dialog').addEventListener('close'"));vm.runInContext(closeLine,c);
 vm.runInContext("gitDiffViews.set('chat\\0/repo',{data:{files:[{path:'file.py',additions:1,deletions:0}],root:'/repo',baseRef:'main'},at:0,pending:false});",c);
 c.render();
 return {c,Node,document,dialog,internal,record,context,$,flush:()=>{while(queued.length)queued.shift()();},reviewButton:()=>context.querySelectorAll('[data-focus-key]').find(node=>node.dataset.focusKey===c.gitReviewFocusKey(record,record.cwd)),fileButton:()=>context.querySelector('.git-file-row')};
}
test('actual review close renders a replacement opener and later workspace polls preserve its focus',async()=>{
 for(const quick of [false,true]){const f=fixture(),opener=quick?f.fileButton():f.reviewButton(),key=opener.dataset.focusKey;opener.focus();await f.c.openDiffReview(f.record,quick?'file.py':undefined,'/repo',quick?opener:null);f.internal.focus();f.c.render();assert.equal(opener.isConnected,false);f.dialog.close();f.flush();assert.equal(f.document.activeElement.dataset.focusKey,key);assert.equal(f.document.activeElement.className,quick?'git-file-row':'text-button');assert.equal(f.document.activeElement.focusOptions.preventScroll,true);const replacement=f.document.activeElement;f.c.render();assert.notEqual(f.document.activeElement,replacement);assert.equal(f.document.activeElement.dataset.focusKey,key);}
});
test('quick to full preserves original file return and queued old close cannot consume its token',async()=>{
 const f=fixture(),opener=f.fileButton(),key=opener.dataset.focusKey;opener.focus();await f.c.openDiffReview(f.record,'file.py','/repo',opener);f.internal.focus();await f.c.openDiffReview(f.record,'file.py','/repo','full');f.flush();assert.equal(f.dialog.open,true);f.c.restoreDialogReturn(f.dialog);f.internal.focus();f.dialog.close();f.flush();assert.equal(f.document.activeElement.dataset.focusKey,key);
});
test('closed command palette uses explicit Review fallback for keyboard opening',async()=>{
 const f=fixture(),palette=new f.Node('dialog'),query=new f.Node('input');palette.append(query);f.document.body.append(palette);query.focus();await f.c.openDiffReview(f.record,undefined,'/repo');f.internal.focus();f.dialog.close();f.flush();assert.equal(f.document.activeElement.dataset.focusKey,f.c.gitReviewFocusKey(f.record,'/repo'));
});
test('actual Review click captures its explicit opener even when WebKit leaves terminal focus unchanged',async()=>{
 const f=fixture(),terminal=new f.Node('textarea');f.document.body.append(terminal);terminal.focus();const opener=f.reviewButton(),key=opener.dataset.focusKey;await opener.onclick({currentTarget:opener});f.internal.focus();f.c.render();f.dialog.close();f.flush();assert.equal(f.document.activeElement.dataset.focusKey,key);assert.notEqual(f.document.activeElement,terminal);
});
test('actual feedback success closes review after its snapshot render and returns to the replacement Review control',async()=>{
 const f=fixture(),opener=f.reviewButton(),key=opener.dataset.focusKey;opener.focus();await opener.onclick({currentTarget:opener});f.internal.focus();Object.assign(f.c,{saveReviewNote:async()=>{},submitChatDispatch:async()=>({finalized:true,message:'Submitted.'}),reviewAction:async()=>{},loadStateSnapshot:async()=>({accepted:true}),refreshChatDispatch:async(_receipt,{canPresent,refresh})=>{await refresh();if(canPresent())f.c.render();return true;},window:{truffleGuidance:{complete(){}}},toast(){}});
 vm.runInContext(review.slice(review.indexOf('function reviewDispatchCurrent('),review.indexOf('function paintReviewDispatch('))+review.slice(review.indexOf('async function sendReviewFeedback('),review.indexOf('let reviewBackgroundPoll=')),f.c);
 await f.c.sendReviewFeedback();f.flush();assert.equal(f.dialog.open,false);assert.equal(opener.isConnected,false);assert.equal(f.document.activeElement.dataset.focusKey,key);f.c.render();assert.equal(f.document.activeElement.dataset.focusKey,key);
});
test('review close never steals external, terminal or new-modal focus',async()=>{
 for(const tag of ['input','textarea','dialog']){const f=fixture();f.reviewButton().focus();await f.c.openDiffReview(f.record,undefined,'/repo');const outside=new f.Node(tag);f.document.body.append(outside);if(tag==='dialog')outside.showModal();else outside.focus();f.dialog.close();f.flush();assert.equal(f.document.activeElement,outside);}
});
test('source, chat and worktree changes invalidate the old review return scope',async()=>{
 for(const change of [f=>{f.c.source='github';},f=>{f.c.selectedAgentKey='record:other';f.c.agentViewSignature='record:other';f.c.state.sessions.push({id:'other',cwd:'/other',items:[]});},f=>{f.record.cwd='/other';}]){const f=fixture();f.reviewButton().focus();await f.c.openDiffReview(f.record,undefined,'/repo');f.internal.focus();change(f);f.dialog.close();f.flush();assert.equal(f.document.activeElement,f.document.body);}
});
test('changing worktree inside review returns only to the explicitly selected current worktree',async()=>{
 const f=fixture();f.record.worktrees=[{path:'/other'}];f.reviewButton().focus();await f.c.openDiffReview(f.record,undefined,'/repo');f.internal.focus();vm.runInContext("gitDiffSelections.chat='/other'",f.c);await f.c.openDiffReview(f.record,undefined,'/other');f.flush();assert.equal(f.dialog.open,true);f.internal.focus();f.dialog.close();f.flush();assert.equal(f.document.activeElement.dataset.focusKey,f.c.gitReviewFocusKey(f.record,'/other'));
});
test('nested return is restricted to an open original parent and visible enabled targets',()=>{
 for(const state of ['open','closed','hidden','css-hidden','disabled','new-modal']){const f=fixture(),parent=new f.Node('dialog'),trigger=new f.Node('button');trigger.dataset.focusKey='nested';parent.append(trigger);f.document.body.append(parent);parent.showModal();trigger.focus();f.c.captureDialogReturn(f.dialog,{scope:'nested',fallbackKey:'nested'});f.dialog.showModal();f.internal.focus();trigger.remove();const replacement=new f.Node('button');replacement.dataset.focusKey='nested';parent.append(replacement);if(state==='closed')parent.open=false;if(state==='hidden')parent.hidden=true;if(state==='css-hidden')parent.style.visibility='hidden';if(state==='disabled')replacement.disabled=true;if(state==='new-modal'){const next=new f.Node('dialog');f.document.body.append(next);next.open=true;}f.dialog.close();f.c.restoreDialogReturn(f.dialog);assert.equal(f.document.activeElement,state==='open'?replacement:f.document.body);}
});
test('changed Git data repaint preserves the exact returned quick-file key without scrolling',async()=>{
 const f=fixture(),opener=f.fileButton(),key=opener.dataset.focusKey;opener.focus();await opener.onclick({currentTarget:opener});f.internal.focus();f.dialog.close();f.flush();const before=f.document.activeElement;
 vm.runInContext("const view=gitDiffViews.get('chat\\0/repo');view.data.files.push({path:'new.py',additions:2,deletions:0});view.paint();",f.c);assert.notEqual(f.document.activeElement,before);assert.equal(f.document.activeElement.dataset.focusKey,key);assert.equal(f.document.activeElement.focusOptions.preventScroll,true);
});
test('local diff repaint has no arbitrary fallback for removed or hidden file rows',()=>{
 for(const hidden of [false,true]){const f=fixture();vm.runInContext("const view=gitDiffViews.get('chat\\0/repo');view.data.files=[{path:'src/file.py',additions:1,deletions:0}];view.paint();",f.c);f.fileButton().focus();vm.runInContext(hidden?"view.collapsedFolders.add('src');view.paint();":"view.data.files=[{path:'different.py',additions:1,deletions:0}];view.paint();",f.c);assert.equal(f.document.activeElement,f.document.body);}
});
test('local diff repaint preserves outside focus and refuses stale chat, root, source or modal ownership',()=>{
 for(const change of ['outside','terminal','chat','root','source','modal','detached']){const f=fixture(),file=f.fileButton();file.focus();let expected=f.document.body;
  if(['outside','terminal'].includes(change)){expected=new f.Node(change==='terminal'?'textarea':'input');f.document.body.append(expected);expected.focus();}
  if(change==='chat')f.c.selectedAgentKey='record:other';if(change==='root')f.record.cwd='/other';if(change==='source')f.c.source='github';if(change==='modal'){const modal=new f.Node('dialog');f.document.body.append(modal);modal.open=true;}
  if(change==='detached')f.context.remove();vm.runInContext("gitDiffViews.get('chat\\0/repo').paint();",f.c);if(change==='detached')assert.equal(f.document.activeElement,file);else assert.equal(f.document.activeElement,expected);
 }
});
