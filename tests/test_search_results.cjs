const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),vm=require('node:vm');
function fixture(items=[]){
 const rendered=[],hooks=[];
 const c=vm.createContext({source:'chat-results',selectedAgentKey:'',state:{sessions:[]},sessionStorage:{getItem:()=>null},registerRenderHook:(name,run,options)=>hooks.push({name,run,options}),allItems:()=>items,workReference:item=>item.reference||'',renderRow:item=>{rendered.push(item);return item;}});
 const app=fs.readFileSync('static/app.js','utf8');
 vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function workViewAllows(')),c);
 const results=fs.readFileSync('static/chat-results.js','utf8');
 vm.runInContext(results.slice(0,results.indexOf('new MutationObserver(')),c);
 return {c,rendered,hooks,match:(row,query)=>c.resultSearchMatches(row,query)};
}
const work=item=>({type:'work',id:item.id,item});
test('selected work results retain canonical relation, source status and detail matches across providers',()=>{
 const cases=[
  ['reviews',{id:'github:pr',source:'github',title:'API',relations:['reviews']}],
  ['approval',{id:'github:status',source:'github',title:'API',sourceStatus:'Awaiting approval'}],
  ['thinkingv2',{id:'linear:ticket',source:'linear',title:'Investigate regression',detail:'Failure in thinking-v2'}],
  ['thinkingv2',{id:'sentry:error',source:'sentry',title:'Unhandled error',detail:'thinking-v2 worker crashed'}],
  ['thinkingv2',{id:'notes:project',source:'notes',title:'Optimization project',detail:'Research thinking-v2'}]
 ];
 const f=fixture();
 for(const [query,item]of cases){assert.equal(Number.isFinite(f.c.workSearchScore(query,item)),true);assert.equal(f.match(work(item),query),true,item.id);assert.equal(f.match(work(item),'unrelatedzzzz'),false,item.id);}
});
test('filtering and rendering use the same current effective work item rather than stale search projections',()=>{
 const current={id:'github:pr',source:'github',title:'Current title',sourceStatus:'Awaiting approval',relations:['reviews'],status:'blocked',overridden:true};
 const stale={...current,title:'Old title',sourceStatus:'Merged',relations:[],status:'done',overridden:false};
 const f=fixture([current]),result=work(stale);
 assert.equal(f.match(result,'reviews'),true);assert.equal(f.match(result,'approval'),true);assert.equal(f.match(result,'merged'),false);
 assert.equal(f.c.renderResultItem(result),current);assert.equal(f.rendered[0],current);
});
test('archived work absent from the current catalog keeps its verified projection for filtering and rendering',()=>{
 const item={id:'linear:archived',source:'linear',title:'Prior issue',detail:'thinkingv2 regression',archivedAt:123};
 const f=fixture(),result=work(item);
 assert.equal(f.match(result,'thinkingv2'),true);assert.equal(f.c.renderResultItem(result),item);
});
test('chat result filtering preserves title, folder and existing metadata matching without work lookup',()=>{
 const f=fixture(),result={type:'chat',id:'chat1',item:{id:'chat1',title:'Optimizer investigation',cwd:'/work/thinking-v2',project:'Research',reference:'EXP-42',source:'codex'}};
 for(const query of ['','optimizer','thinkingv2','Research','EXP-42','codex'])assert.equal(f.match(result,query),true,query);
 assert.equal(f.match(result,'unrelatedzzzz'),false);
});
test('displayed results actually retain a detail-only result when the filter input changes',()=>{
 class Node{
  constructor(tag,cls,text){this.tag=tag;this.isConnected=true;this.className=cls;this.textContent=text;this.children=[];this.classList={toggle(){}};}
  append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}
  replaceChildren(...nodes){for(const node of this.children)node.parentElement=null;this.children=[];this.append(...nodes);}
  setAttribute(){}
 }
 const item={id:'linear:ticket',source:'linear',title:'Investigate regression',detail:'Failure in thinking-v2'},f=fixture([item]);
 const list=new Node('main'),title=new Node('h1');
 Object.assign(f.c,{$:selector=>selector==='#work-list'?list:selector==='#page-title'?title:null,el:(...args)=>new Node(...args),requestAnimationFrame:callback=>callback(),renderRow:row=>{const node=new Node('article');node.workId=row.id;return node;}});
 f.c.result={type:'display-items',title:'Thinking investigation',items:[work(item)],recommended:null,reason:''};
 vm.runInContext('chatSearchPage={command:result,scroll:0};renderChatSearchResults();',f.c);
 const form=list.children[0],input=form.children[0],matches=list.children[2];
 assert.equal(matches.children.length,1);input.value='thinkingv2';input.oninput();assert.equal(matches.children.length,1);assert.equal(matches.children[0].workId,item.id);
 input.value='unrelatedzzzz';input.oninput();assert.equal(matches.children.length,0);
});

test('work queries match each word independently across metadata and detail fields',()=>{
 const f=fixture();
 const cases=[
  ['thinkingv2 latency',{title:'Fix latency',detail:'The thinking-v2 worker regressed'}],
  ['ENG42 approval',{reference:'ENG-42',sourceStatus:'Awaiting approval'}],
  ['reviews latency',{relations:['reviews'],detail:'Latency regressed'}],
  ['pltfrm pymt',{project:'Platform',title:'Payments changes'}],
  ['thinkingv2 retry',{title:'Retry request',detail:'thinking-v2 failed'}]
 ];
 for(const [query,item]of cases){assert.equal(Number.isFinite(f.c.workSearchScore(query,item)),true,query);assert.equal(f.match(work({id:query,...item}),query),true,query);}
});
test('work query membership never creates matches across field boundaries or fuzzes detail text',()=>{
 const f=fixture();
 for(const [query,item]of [
  ['abcdef',{title:'abc',project:'def'}],
  ['pymt',{detail:'Payments'}],
  ['thinkingv2 missingzzzz',{title:'thinking-v2',detail:'Existing text'}]
 ])assert.equal(f.c.workSearchScore(query,item),-Infinity,query);
 assert.equal(f.c.workSearchScore(' --- ',{title:'anything'}),0);
});
test('work ranking preserves exact matches above fuzzy matches and metadata field priorities',()=>{
 const f=fixture(),score=item=>f.c.workSearchScore('review',item);
 assert.ok(score({title:'Review'})>score({title:'Revenue indexing workflow'}));
 assert.ok(score({title:'Review'})>score({reference:'Review'}));
 assert.ok(score({reference:'Review'})>score({project:'Review'}));
 assert.ok(score({project:'Review'})>score({sourceStatus:'Review'}));
 assert.ok(score({sourceStatus:'Review'})>score({detail:'Review'}));
 assert.ok(score({detail:'Review'})>score({title:'Revenue indexing workflow'}));
 assert.ok(score({title:'Review changes'})>score({title:'Needs review'}));
});

test('mixed chat results match linked work titles, references, URLs and projects despite unrelated chat metadata',()=>{
 const f=fixture(),item={id:'chat-linked',title:'Daily notes',cwd:'/work/sandbox',items:[{title:'Thinking-v2 regression',reference:'ENG-42',url:'https://github.com/team/optimizer/pull/123',project:'Research Platform'}]},result={type:'chat',id:item.id,item};
 for(const query of ['thinkingv2','ENG42','optimizer/pull/123','Research Platform','thinkingv2 ENG42'])assert.equal(f.match(result,query),true,query);
 assert.equal(f.match(result,'unrelatedzzzz'),false);
});
test('mixed chat filtering and displayed references share current linked records instead of stale projections',()=>{
 const f=fixture(),fresh=[{title:'Thinking-v2 regression',reference:'ENG-42',url:'https://github.com/team/current/pull/2'}];
 f.c.state.sessions=[{id:'chat-linked',items:fresh}];
 const result={type:'chat',id:'chat-linked',item:{id:'chat-linked',title:'Daily notes',cwd:'/work/sandbox',items:[{title:'Superseded ticket',reference:'STALE-999',url:'https://example.com/removed-link'}]}};
 assert.equal(f.c.resultChatLinks(result),fresh);assert.equal(f.match(result,'thinkingv2 ENG42'),true);assert.equal(f.match(result,'STALE999'),false);assert.equal(f.match(result,'removed-link'),false);
 f.c.state.sessions[0].items=[];
 assert.equal(f.match(result,'thinkingv2'),false);assert.equal(f.match(result,'STALE999'),false);
});
test('chat projections retain linked-work search when no current record exists, without inspecting runtime text',()=>{
 const f=fixture(),result={type:'chat',id:'archived-chat',item:{id:'archived-chat',title:'Daily notes',items:[{title:'Thinking-v2 regression',reference:'ENG-42'}],runtime:{text:'private-runtime-sentinel'},terminalText:'private-terminal-sentinel'}};
 assert.equal(f.c.resultChatLinks(result),result.item.items);assert.equal(f.match(result,'thinkingv2 ENG42'),true);
 assert.equal(f.match(result,'private-runtime-sentinel'),false);assert.equal(f.match(result,'private-terminal-sentinel'),false);
});
test('mixed chat cards display the same current linked references used by filtering',()=>{
 const f=fixture();class Node{constructor(tag,cls,text){this.tag=tag;this.isConnected=true;this.textContent=text;this.children=[];this.dataset={};}append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}}
 Object.assign(f.c,{el:(...args)=>new Node(...args),candidateForRecord:()=>null,liveCandidates:()=>[],agentName:agent=>agent});
 f.c.state.sessions=[{id:'chat-linked',items:[{title:'Thinking-v2 regression',reference:'ENG-42'}]}];
 const result={type:'chat',id:'chat-linked',item:{id:'chat-linked',title:'Daily notes',agent:'codex',cwd:'/work/sandbox',items:[{reference:'STALE-999'}]}};
 const card=f.c.renderResultItem(result),context=card.children[2];
 assert.equal(context.children[1].textContent,'ENG-42');assert.equal(f.match(result,'thinkingv2 ENG42'),true);assert.equal(f.match(result,'STALE999'),false);
});

test('mixed result Back restores its local filter and scroll without rewriting the verified command',async()=>{
 class Node{
  constructor(tag,cls,text){this.tag=tag;this.isConnected=true;this.className=cls;this.textContent=text;this.children=[];this.classList={toggle(){}};this.dataset={};this.scrollTop=0;}
  append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}
  replaceChildren(...nodes){for(const node of this.children)node.parentElement=null;this.children=[];this.append(...nodes);}
  setAttribute(){}
 }
 const f=fixture(),list=new Node('main'),title=new Node('h1'),search={value:''},session={id:'chat-thinking',title:'Daily notes',cwd:'/work/thinking-v2'};
 const command={type:'display-items',title:'Selected investigation',items:[{type:'chat',id:session.id,item:session},work({id:'linear:unrelated',title:'Other ticket'})],recommended:null,reason:''};
 const descendants=node=>[node,...node.children.flatMap(descendants)];let saved;
 Object.assign(f.c,{source:'chat-results',appChatCommand:value=>value,loadInventory:async()=>{},candidateForRecord:()=>null,liveCandidates:()=>[],selectAgent:record=>{assert.equal(record,session);f.c.source='agents';},updateChatResultsBack:()=>{},persist(){},render:()=>f.c.renderChatSearchResults(),$:selector=>selector==='#work-list'?list:selector==='#page-title'?title:selector==='#search'?search:descendants(list).find(node=>'#'+node.id===selector),el:(...args)=>new Node(...args),renderRow:()=>new Node('article'),agentName:agent=>agent||'Agent',requestAnimationFrame:callback=>callback()});
 f.c.state.sessions=[session];f.c.sessionStorage.setItem=(_,value)=>{saved=JSON.parse(value);};
 f.c.showChatResults(command);const initial=JSON.stringify(command),input=list.children[0].children[0];
 input.value='thinkingv2';input.oninput();let matches=list.children[2];assert.equal(matches.children.length,1);assert.equal(saved.filter,'thinkingv2');assert.equal(saved.command.query,undefined);
 matches.scrollTop=87;await f.c.openChatSearchTarget(session.id);assert.equal(f.c.source,'agents');
 f.c.showChatResults();assert.equal(list.children[0].children[0].value,'thinkingv2');matches=list.children[2];assert.equal(matches.children.length,1);assert.equal(matches.scrollTop,87);assert.equal(JSON.stringify(command),initial);
 // A subsequent visit from the top must not reuse the previously stored nonzero offset.
 matches.scrollTop=0;await f.c.openChatSearchTarget(session.id);f.c.showChatResults();assert.equal(list.children[2].scrollTop,0);
});

function pendingResultsFixture(){
 class Node{
  constructor(tag,cls,text){this.tag=tag;this.textContent=text;this.children=[];this.isConnected=true;this.scrollTop=0;}
  append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}
  replaceChildren(...nodes){for(const node of this.children)node.parentElement=null;this.children=[];this.append(...nodes);}
  setAttribute(){}
 }
 const f=fixture(),list=new Node('main'),title=new Node('h1'),search={value:''},requests=[],opened=[];
 Object.assign(f.c,{source:'chat-results',appChatCommand:value=>value,persist(){},render:()=>f.c.renderChatSearchResults(),$:selector=>selector==='#work-list'?list:selector==='#page-title'?title:selector==='#search'?search:null,el:(...args)=>new Node(...args),requestAnimationFrame:callback=>callback(),api:(_,args)=>new Promise((resolve,reject)=>requests.push({args,resolve,reject})),candidateForRecord:()=>null,liveCandidates:()=>[],updateChatResultsBack(){},selectAgent:record=>opened.push(record.id)});
 f.c.showChatResults({type:'chat-results',query:'alpha',chats:[],total:0,recommendedId:'',reason:''});
 return {...f,list,requests,opened,form:list.children[0],input:list.children[0].children[0],status:list.children[1],submit:()=>list.children[0].onsubmit({preventDefault(){}})};
}
test('a pending chat search captures its query and cannot replace a newer page or edit',async()=>{
 for(const change of ['navigate','edit','replace','detach']){
  const f=pendingResultsFixture(),pending=f.submit();assert.equal(f.requests[0].args.args.query,'alpha');
  if(change==='navigate')f.c.source='github';
  if(change==='edit')f.input.value='beta';
  if(change==='replace')f.c.showChatResults({type:'chat-results',query:'new',chats:[],total:0,recommendedId:'',reason:''});
  if(change==='detach')f.form.isConnected=false;
  f.requests[0].resolve({chats:[{id:'alpha-chat',title:'Alpha',cwd:'/work'}],total:1});await pending;
  assert.equal(f.c.source,change==='navigate'?'github':'chat-results');
  assert.notEqual(vm.runInContext('chatSearchPage.command.chats[0]?.id',f.c),'alpha-chat',change);
  if(change==='edit')assert.equal(f.input.value,'beta');
 }
});
test('a current chat search renders its result while obsolete errors stay off the page',async()=>{
 const current=pendingResultsFixture(),pending=current.submit();current.requests[0].resolve({chats:[{id:'alpha-chat',title:'Alpha',cwd:'/work'}],total:1});await pending;
 assert.equal(vm.runInContext('chatSearchPage.command.query',current.c),'alpha');assert.equal(vm.runInContext('chatSearchPage.command.chats[0].id',current.c),'alpha-chat');
 for(const obsolete of [false,true]){
  const f=pendingResultsFixture(),failed=f.submit();if(obsolete)f.input.value='beta';f.requests[0].reject(new Error('Search unavailable'));await failed;
  assert.equal(f.status.textContent.includes('Search unavailable'),!obsolete);assert.equal(f.form.children[1].disabled,false);
 }
});
test('pending chat opens preserve navigation intent and only the latest selection opens',async()=>{
 for(const change of ['navigate','replace','newer-open','inventory-render','select-chat']){
  const f=pendingResultsFixture(),loads=[];f.c.state.sessions=[{id:'a'},{id:'b'}];f.c.loadInventory=()=>new Promise(resolve=>loads.push(resolve));
  if(change==='select-chat'){f.c.source='agents';f.c.selectedAgentKey='first';}
  const first=f.c.openChatSearchTarget('a');
  let second;
  if(change==='navigate')f.c.source='linear';
  if(change==='select-chat')f.c.selectedAgentKey='second';
  if(change==='replace')f.c.showChatResults({type:'chat-results',query:'replacement',chats:[],total:0,recommendedId:'',reason:''});
  if(change==='newer-open')second=f.c.openChatSearchTarget('b');
  if(change==='inventory-render')f.c.render();
  loads[0]();await first;
  if(second){loads[1]();await second;}
  assert.deepEqual(f.opened,change==='newer-open'?['b']:change==='inventory-render'?['a']:[],change);
 }
});

function stableResultsFixture(items=[]){
 const f=fixture(items),nodes=new Map(),requests=[],frames=[],stored=new Map(),document={activeElement:null};
 class Node{
  constructor(tag,cls='',text=''){Object.assign(this,{tagName:tag.toUpperCase(),className:cls,textContent:text||'',children:[],parentElement:null,dataset:{},attrs:{},scrollTop:0,_value:'',selectionStart:0,selectionEnd:0,valueWrites:0,compositionActive:false,disabled:false});this.classList={toggle(){}};}
  get isConnected(){return this===document.body||Boolean(this.parentElement?.isConnected);}
  get value(){return this._value;}
  set value(value){this._value=String(value);this.selectionStart=this.selectionEnd=this._value.length;this.valueWrites++;this.compositionActive=false;}
  append(...children){for(const child of children){if(child.parentElement)child.parentElement.children=child.parentElement.children.filter(node=>node!==child);child.parentElement=this;this.children.push(child);}}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  replaceChildren(...children){if(this.children.some(child=>child.contains(document.activeElement)))document.activeElement=document.body;for(const child of this.children)child.parentElement=null;this.children=[];this.scrollTop=0;this.append(...children);}
  remove(){if(this.contains(document.activeElement))document.activeElement=document.body;if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(node=>node!==this);this.parentElement=null;}
  setAttribute(key,value){this.attrs[key]=value;}
  focus(){if(this.isConnected&&!this.disabled)document.activeElement=this;}
  setSelectionRange(start,end){this.selectionStart=start;this.selectionEnd=end;}
 }
 document.body=new Node('body');document.activeElement=document.body;
 for(const [id,tag]of [['work-list','main'],['page-title','h1'],['search','input']]){const node=new Node(tag);node.id=id;nodes.set('#'+id,node);document.body.append(node);}
 const visit=node=>[node,...node.children.flatMap(visit)],$=selector=>nodes.get(selector)||visit(document.body).find(node=>'#'+node.id===selector)||null;
 Object.assign(f.c,{document,$,appChatCommand:command=>command,el:(...args)=>new Node(...args),persist(){},render:()=>{for(const hook of f.hooks)if(hook.options.phase==='before')hook.run();if(f.c.source==='chat-results')f.c.renderChatSearchResults();},requestAnimationFrame:callback=>frames.push(callback),api:async(path,payload)=>new Promise((resolve,reject)=>requests.push({path,payload,resolve,reject})),renderRow:item=>{const row=new Node('article','work-row',item.title);row.dataset.workId=item.id;row.status=item.status;return row;},sessionStorage:{getItem:()=>null,setItem:(key,value)=>stored.set(key,value)},candidateForRecord:()=>null,liveCandidates:()=>[],agentName:agent=>agent||'Agent'});
 return {...f,Node,document,nodes,$,requests,stored,flush:()=>{while(frames.length)frames.shift()();},show:command=>{f.c.showChatResults(command);while(frames.length)frames.shift()();},form:()=>$('#work-list').children[0],input:()=>$('#work-list').children[0].children[0],submit:()=>$('#work-list').children[0].onsubmit({preventDefault(){}})};
}
const emptySearch=query=>({type:'chat-results',query,chats:[],total:0,recommendedId:'',reason:''});
test('same-page rich result refresh retains the exact focused filter, caret, composition and handlers while repainting current records',()=>{
 const current={id:'linear:saved',title:'Saved item',source:'linear',status:'todo'},f=stableResultsFixture([current]);f.show({type:'display-items',title:'Selected',items:[work({...current})],reason:''});const form=f.form(),input=f.input(),handler=input.oninput,list=f.$('#chat-search-matches');input.value='Sved';input.oninput();input.focus();input.setSelectionRange(1,1);input.compositionActive=true;const writes=input.valueWrites;
 current.title='Saved latest';current.status='blocked';f.c.render();f.c.render();f.flush();assert.equal(f.form(),form);assert.equal(f.input(),input);assert.equal(f.document.activeElement,input);assert.equal(input.value,'Sved');assert.equal(input.selectionStart,1);assert.equal(input.selectionEnd,1);assert.equal(input.compositionActive,true);assert.equal(input.valueWrites,writes);assert.equal(input.oninput,handler);
 // Native typing resumes at the retained caret after the same render used by sidebar toggles.
 input.compositionActive=false;const at=input.selectionStart;input.value=input.value.slice(0,at)+'a'+input.value.slice(input.selectionEnd);input.setSelectionRange(at+1,at+1);input.oninput();assert.equal(input.value,'Saved');assert.equal(list.children[0].textContent,'Saved latest');assert.equal(list.children[0].status,'blocked');const previousRow=list.children[0];list.scrollTop=42;current.title='Saved newest';f.c.render();assert.notEqual(list.children[0],previousRow);assert.equal(list.children[0].textContent,'Saved newest');assert.equal(list.scrollTop,42);assert.equal(f.requests.length,0);
});
test('chat-search unsent text and native selection survive refresh without becoming the stored command query',()=>{
 const f=stableResultsFixture();f.show(emptySearch('submitted'));const input=f.input(),form=f.form(),submit=form.onsubmit;input.value='unsent Ω text';input.focus();input.setSelectionRange(2,7);input.compositionActive=true;const writes=input.valueWrites;f.c.render();f.flush();assert.equal(f.input(),input);assert.equal(f.form(),form);assert.equal(form.onsubmit,submit);assert.equal(input.value,'unsent Ω text');assert.equal(input.valueWrites,writes);assert.equal(input.selectionStart,2);assert.equal(input.selectionEnd,7);assert.equal(input.compositionActive,true);assert.equal(f.document.activeElement,input);assert.equal(JSON.parse(f.stored.get('truffle-chat-results')).command.query,'submitted');
});
test('a pending actual search remains connected and singleflight through repeated unrelated renders then resolves its captured query once',async()=>{
 const f=stableResultsFixture();f.show(emptySearch('old'));const form=f.form(),input=f.input();input.value='new query';input.focus();const pending=f.submit();assert.equal(form.children[1].disabled,true);assert.equal(form.attrs['aria-busy'],'true');f.c.render();f.c.render();const duplicate=f.submit();assert.equal(f.requests.length,1);assert.equal(f.form(),form);assert.equal(form.isConnected,true);assert.equal(f.input(),input);assert.equal(form.children[1].disabled,true);assert.equal(f.requests.length,1);assert.equal(f.requests[0].payload.args.query,'new query');f.requests[0].resolve({chats:[{id:'match',title:'New query match',cwd:'/work'}],total:1});await Promise.all([pending,duplicate]);f.flush();assert.equal(vm.runInContext('chatSearchPage.command.query',f.c),'new query');assert.equal(vm.runInContext('chatSearchPage.command.chats[0].id',f.c),'match');assert.notEqual(f.form(),form);assert.equal(f.requests.length,1);
});
test('same-page search errors and editable drafts survive later background renders without restarting requests',async()=>{
 const f=stableResultsFixture();f.show(emptySearch('alpha'));const input=f.input(),form=f.form(),pending=f.submit();f.requests[0].reject(Error('Search temporarily unavailable'));await pending;input.value='beta draft';input.focus();f.c.render();assert.equal(f.form(),form);assert.equal(f.input(),input);assert.equal(input.value,'beta draft');assert.equal(f.$('#work-list').children[1].textContent,'Search temporarily unavailable');assert.equal(form.children[1].disabled,false);assert.equal(form.attrs['aria-busy'],'false');assert.equal(f.requests.length,1);assert.equal(f.document.activeElement,input);
});
test('source-away ABA, replacement command and replacement host invalidate the old surface and its pending reply',async()=>{
 for(const mode of ['navigate-return','command','host','host-still-connected','owned-node']){
  const f=stableResultsFixture();f.show(emptySearch('alpha'));const old=f.form(),pending=f.submit();
  if(mode==='navigate-return'){f.c.source='agents';f.c.render();assert.equal(old.isConnected,true);f.c.source='chat-results';f.c.render();}
  if(mode==='command')f.show(emptySearch('new command'));
  if(mode==='host'){const host=new f.Node('main');host.id='work-list';f.$('#work-list').remove();f.document.body.append(host);f.nodes.set('#work-list',host);f.c.render();}
  if(mode==='host-still-connected'){const previous=f.$('#work-list'),host=new f.Node('main');previous.id='previous-work-list';host.id='work-list';f.document.body.append(host);f.nodes.set('#work-list',host);f.c.render();}
  if(mode==='owned-node'){old.children[0].remove();f.c.render();}
  assert.notEqual(f.form(),old,mode);f.requests[0].resolve({chats:[{id:'obsolete',title:'Old result',cwd:'/old'}],total:1});await pending;f.flush();assert.notEqual(vm.runInContext('chatSearchPage.command.chats[0]?.id',f.c),'obsolete',mode);assert.equal(f.requests.length,1,mode);
 }
});
test('refreshing rows never focuses the filter over an outside or modal input',()=>{
 for(const modal of [false,true]){const f=stableResultsFixture();f.show(emptySearch('alpha'));const input=f.input(),outside=new f.Node('input');if(modal){const dialog=new f.Node('dialog');dialog.open=true;dialog.append(outside);f.document.body.append(dialog);}else f.document.body.append(outside);outside.focus();f.c.render();assert.equal(f.input(),input);assert.equal(f.document.activeElement,outside);}
});
