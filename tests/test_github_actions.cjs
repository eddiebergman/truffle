const {test}=require('node:test'),assert=require('node:assert/strict');
const {githubUnreadReviews,githubReviewerLabel}=require('../static/github-actions.js');
test('unread attention excludes drafts, closed PRs, authorship-only and seen cycles',()=>{
  const base={source:'github',kind:'Pull request',status:'needs-review',isDraft:false,relations:['reviews']};
  const items=[{...base,id:'ready'},{...base,id:'draft',isDraft:true},{...base,id:'done',status:'done'},{...base,id:'mine',relations:['authored']},{...base,id:'seen'}];
  const data={githubReviewInbox:Object.fromEntries(items.map(i=>[i.id,{active:true,version:i.id,seenAt:i.id==='seen'?1:null}]))};
  assert.deepEqual(githubUnreadReviews(data,items),[{id:'ready',version:'ready'}]);
});
test('reviewer chip distinguishes unavailable data from no pending requests and truncates multiple people',()=>{
  assert.equal(githubReviewerLabel({}),'Reviewers…');assert.equal(githubReviewerLabel({requestedReviewers:[]}),'None');
  assert.equal(githubReviewerLabel({requestedReviewers:[{login:'reviewer'}],reviewerCount:3}),'@reviewer +2');
  assert.equal(githubReviewerLabel({requestedReviewers:[],latestReviews:[{author:{login:'reviewer'},state:'APPROVED'}]}),'✓ @reviewer');
});
const {githubStackGroups}=require('../static/github-actions.js');
const pr=(id,head,base,repo='org/repo')=>({id,source:'github',kind:'Pull request',project:repo,headRepository:repo,headRefName:head,baseRefName:base,reference:'#'+id,status:'needs-review',githubState:'OPEN'});
test('stacked PRs sort base first even when filtered; unstacked stay separate',()=>{
  const base=pr('1','base','main'),middle=pr('2','middle','base'),tip=pr('3','tip','middle'),solo=pr('4','solo','main');
  const all=[tip,middle,solo,base];const groups=githubStackGroups(all,all);
  assert.deepEqual(groups[0].rows.map(r=>[r.item.id,r.depth]),[['1',0],['2',1],['3',2]]);
  assert.equal(groups[1].root,null);
  const filtered=githubStackGroups([tip],all)[0];assert.equal(filtered.root.id,'1');assert.equal(filtered.rows[0].parent.id,'2');
});
test('branch matching ignores other repos, closed PRs, ambiguous fork heads and old caches',()=>{
  const base=pr('1','base','main'),tip=pr('2','tip','base');
  for(const candidates of [[{...base,project:'other/repo',headRepository:'other/repo'},tip],[{...base,githubState:'MERGED'},tip],[base,{...base,id:'duplicate'},tip],[{...base,headRepository:'fork/repo'},tip],[{...base,headRefName:null},tip]])assert.equal(githubStackGroups([tip],candidates)[0].root,null);
});
test('cycles fail open and branching children retain shared root',()=>{
  const a=pr('1','a','b'),b=pr('2','b','a');assert.equal(githubStackGroups([a,b],[a,b]).every(g=>!g.root),true);
  const root=pr('1','a','main'),left=pr('2','b','a'),right=pr('3','c','a');const group=githubStackGroups([right,left,root],[root,left,right])[0];assert.deepEqual(group.rows.map(r=>r.depth),[0,1,1]);
});

test('normal GitHub ordering keeps entire stacks together ahead of standalone PRs',()=>{
  const {orderedGithubItems}=require('../static/github-actions.js');
  const base=pr('1','base','main'),tip=pr('2','tip','base'),solo=pr('3','solo','main');
  assert.deepEqual(orderedGithubItems([solo,tip,base],[base,tip,solo]).map(i=>i.id),['1','2','3']);
  assert.deepEqual(orderedGithubItems([tip,solo],[base,tip,solo]).map(i=>i.id),['2','3']);
});

const fs=require('node:fs'),vm=require('node:vm');
function reviewPanelFixture(){
 const calls=[],messages=[],requests=[],document={activeElement:null};let renders=0;
 class Element{
  constructor(tag='div',cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.children=[];this.attrs={};this.dataset={};this.style={};}
  append(...nodes){for(const node of nodes){node.parent=this;this.children.push(node);}}
  replaceChildren(...nodes){if(this.contains(document.activeElement))document.activeElement=body;for(const node of this.children)node.parent=null;this.children=[];this.append(...nodes);}
  remove(){if(this.contains(document.activeElement))document.activeElement=body;if(this.parent)this.parent.children=this.parent.children.filter(node=>node!==this);this.parent=null;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  get isConnected(){return this===body||body.contains(this);}
  setAttribute(key,value){this.attrs[key]=value;}
  removeAttribute(key){delete this.attrs[key];}
  focus(){if(this.isConnected&&!this.disabled)document.activeElement=this;}
 }
 const body=new Element('body'),anchor=new Element('button','github-reviewer-chip','Reviewers'),fallback=new Element('button','','Activity');anchor.dataset.focusKey='github:PR_A:reviewers';anchor.getBoundingClientRect=()=>({bottom:30,right:300});body.append(anchor,fallback);document.body=body;document.activeElement=anchor;document.querySelectorAll=()=>all().filter(node=>node.className.includes('github-reviewer-chip'));
 const item=(id='github:PR_A',extra={})=>({id,source:'github',kind:'Pull request',reference:'#'+id.slice(-1),requestedReviewers:[],reviewerUpdatedAt:Date.now()/1000,canUpdate:true,isDraft:false,status:'needs-review',githubState:'OPEN',...extra});
 const initial=item(),c=vm.createContext({document,$:()=>fallback,window:{innerWidth:1000,innerHeight:700},Date,state:{revision:'one',items:[initial],sources:{github:{lastSuccess:Date.now()/1000}}},el:(...args)=>new Element(...args),featureButton:(label,onclick,cls)=>Object.assign(new Element('button',cls,label),{onclick}),Option:class extends Element{constructor(text,value){super('option','',text);this.value=value;}},workReference:row=>row.reference,api:(path,args)=>{calls.push({path,args});return new Promise((resolve,reject)=>requests.push({path,args,resolve,reject}));},toast:message=>messages.push(message),render:()=>renders++});
 const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);
 const github=fs.readFileSync('static/github-actions.js','utf8');vm.runInContext('let githubReviewPanel=null;\n'+github.slice(github.indexOf('function openGithubReviewPanel('),github.indexOf("if(typeof document!==")),c);
 const all=(node=body)=>node.children.flatMap(child=>[child,...all(child)]),panel=()=>body.children.find(node=>node.className==='github-review-panel'),button=text=>all().find(node=>node.tagName==='BUTTON'&&node.textContent===text);
 return {c,body,anchor,fallback,document,Element,item,initial,calls,messages,requests,panel,button,all,open:(row=initial)=>c.openGithubReviewPanel(row,anchor),get renders(){return renders;}};
}
const settle=async()=>{for(let i=0;i<8;i++)await Promise.resolve();};
const githubReceipt=(id,action,extra={})=>({id,action,status:'completed',remote:'confirmed',localSaved:true,...extra});
test('PR buttons invoke exact named commands once, normalize reviewer, and use guarded state refresh before showing confirmed result',async()=>{
 const f=reviewPanelFixture();f.open();const form=f.all().find(node=>node.tagName==='FORM'),input=f.all().find(node=>node.tagName==='INPUT');input.value=' @reviewer ';input.oninput();form.onsubmit({preventDefault(){}});form.onsubmit({preventDefault(){}});
 assert.equal(f.requests.length,1);assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0])),{path:'/api/commands/invoke',args:{name:'github.pr.request-review',args:{id:f.initial.id,login:'reviewer'}}});
 f.requests[0].resolve(githubReceipt(f.initial.id,'request-review'));await settle();assert.equal(f.requests[1].path,'/api/state');assert.equal(f.messages.length,0);
 const updated=f.item(f.initial.id,{requestedReviewers:[{login:'reviewer',kind:'user'}]});f.requests[1].resolve({...f.c.state,revision:'two',items:[updated]});await settle();assert.equal(f.c.state.items[0],updated);assert.deepEqual(f.messages,['Review requested on GitHub.']);assert.equal(f.renders,1);assert.ok(f.all().some(node=>node.textContent==='@reviewer'));
});
test('ready preview stays explicit and refresh uses its named command without claiming a remote change',async()=>{
 const f=reviewPanelFixture();f.open(f.item(f.initial.id,{isDraft:true,status:'draft'}));f.button('Mark ready for review').onclick();assert.equal(f.requests[0].args.name,'github.pr.ready');f.requests[0].resolve(githubReceipt(f.initial.id,'ready',{status:'preview',remote:'not-contacted'}));await settle();f.requests[1].resolve({...f.c.state,revision:'two',items:[f.item()]});await settle();assert.deepEqual(f.messages,['Preview updated; GitHub was unchanged.']);
 f.button('Refresh PR').onclick();assert.equal(f.requests[2].args.name,'github.pr.refresh');assert.deepEqual(JSON.parse(JSON.stringify(f.requests[2].args.args)),{id:f.initial.id});f.requests[2].resolve(githubReceipt(f.initial.id,'refresh'));await settle();f.requests[3].resolve({...f.c.state,revision:'three'});await settle();assert.equal(f.messages.length,1);
});
test('completed mutation with failed snapshot keeps success clear, blocks replay, and offers only explicit Refresh recovery',async()=>{
 const f=reviewPanelFixture();f.open(f.item(f.initial.id,{isDraft:true,status:'draft'}));f.button('Mark ready for review').onclick();f.requests[0].resolve(githubReceipt(f.initial.id,'ready'));await settle();f.requests[1].reject(new Error('Snapshot unavailable'));await settle();
 assert.ok(f.all().some(node=>node.attrs.role==='alert'&&node.textContent.includes('Marked ready on GitHub.')&&node.textContent.includes('do not repeat')));const ready=f.button('Mark ready for review');assert.equal(ready.disabled,true);ready.onclick();assert.equal(f.requests.length,2);f.button('Refresh PR').onclick();assert.equal(f.requests.length,3);assert.equal(f.requests[2].args.name,'github.pr.refresh');
});
test('stale snapshot responses preserve newer state and two conflicts never replay the remote mutation',async()=>{
 const f=reviewPanelFixture();f.open();f.button('Refresh PR').onclick();f.requests[0].resolve(githubReceipt(f.initial.id,'refresh'));await settle();
 const newer={...f.c.state,revision:'newer',marker:'keep'};f.c.state=newer;f.requests[1].resolve({...newer,revision:'stale',marker:'lost'});await settle();assert.equal(f.c.state,newer);assert.equal(f.requests[2].path,'/api/state');
 const newest={...newer,revision:'newest'};f.c.state=newest;f.requests[2].resolve({...newer,revision:'still-stale'});await settle();assert.equal(f.c.state,newest);assert.equal(f.calls.filter(call=>call.path==='/api/commands/invoke').length,1);assert.ok(f.all().some(node=>node.attrs.role==='alert'&&node.textContent.includes('newer changes were kept')));
});
test('responses for a closed or replaced PR panel never repaint or clear inputs on its replacement',async()=>{
 for(const delayedStage of ['command','snapshot']){
  const f=reviewPanelFixture();f.open();f.button('Refresh PR').onclick();if(delayedStage==='snapshot'){f.requests[0].resolve(githubReceipt(f.initial.id,'refresh'));await settle();}
  const other=f.item('github:PR_B');f.open(other);const replacement=f.panel(),input=f.all().find(node=>node.tagName==='INPUT');input.value='keep-b';input.oninput();const children=[...replacement.children];
  f.requests[delayedStage==='command'?0:1].resolve(delayedStage==='command'?githubReceipt(f.initial.id,'refresh'):{...f.c.state,revision:'two'});await settle();assert.equal(f.panel(),replacement);assert.deepEqual(replacement.children,children);assert.equal(input.value,'keep-b');assert.equal(f.messages.length,0);assert.equal(f.renders,0);if(delayedStage==='command')assert.equal(f.requests.length,1);
 }
});
test('remote-confirmed local-save failure keeps uncertainty text and requires Refresh rather than a mutation replay',async()=>{
 const f=reviewPanelFixture();f.open(f.item(f.initial.id,{isDraft:true,status:'draft'}));f.button('Mark ready for review').onclick();const error=new Error('GitHub returned the PR, but local state could not be saved. Refresh the PR before retrying; any remote change may already be applied.');error.data={githubReceipt:{remote:'confirmed',localSaved:false,status:'remote-confirmed-local-failed'}};f.requests[0].reject(error);await settle();
 assert.equal(f.messages.length,0);assert.equal(f.button('Mark ready for review').disabled,true);assert.ok(f.all().some(node=>node.attrs.role==='alert'&&node.textContent===error.message));assert.equal(f.requests.length,1);
});

test('post-dispatch uncertain rejection preserves uncertainty and blocks mutation replay until an explicit successful Refresh',async()=>{
 const f=reviewPanelFixture();f.open();const submit=()=>f.all().find(node=>node.tagName==='FORM').onsubmit({preventDefault(){}}),input=f.all().find(node=>node.tagName==='INPUT');input.value='reviewer';input.oninput();submit();
 const error=new Error('GitHub did not confirm this change. Refresh the PR before retrying; check repository permissions and token write access.');error.data={githubReceipt:{id:f.initial.id,action:'request-review',status:'confirmation-unknown',remote:'unconfirmed',localSaved:false}};f.requests[0].reject(error);await settle();
 assert.equal(f.messages.length,0);assert.ok(f.all().some(node=>node.attrs.role==='alert'&&node.textContent===error.message));const disabled=f.all().find(node=>node.tagName==='BUTTON'&&node.textContent==='Request review');assert.equal(disabled.disabled,true);submit();assert.equal(f.requests.length,1);
 f.button('Refresh PR').onclick();assert.equal(f.requests[1].args.name,'github.pr.refresh');f.requests[1].resolve(githubReceipt(f.initial.id,'refresh'));await settle();f.requests[2].resolve({...f.c.state,revision:'two'});await settle();assert.equal(f.all().find(node=>node.tagName==='BUTTON'&&node.textContent==='Request review').disabled,false);assert.equal(f.calls.filter(call=>call.args?.name==='github.pr.request-review').length,1);
});
test('preflight errors without an uncertainty receipt remain editable and never claim remote completion',async()=>{
 const f=reviewPanelFixture();f.open();const form=f.all().find(node=>node.tagName==='FORM'),input=f.all().find(node=>node.tagName==='INPUT');input.value='valid-user';input.oninput();form.onsubmit({preventDefault(){}});f.requests[0].reject(new Error('GitHub username not found.'));await settle();assert.equal(f.all().find(node=>node.tagName==='BUTTON'&&node.textContent==='Request review').disabled,false);assert.equal(f.messages.length,0);assert.ok(f.all().some(node=>node.attrs.role==='alert'&&node.textContent==='GitHub username not found.'));
});
test('review acknowledgement ignores its old response snapshot and commits only a guarded current read',async()=>{
 const requests=[],calls=[],hook={},item={id:'github:PR_A',source:'github',kind:'Pull request',status:'needs-review',relations:['reviews'],isDraft:false};let renders=0;
 const c=vm.createContext({state:{revision:'one',items:[item],githubReviewInbox:{[item.id]:{active:true,version:'cycle',seenAt:null}}},source:'github',githubUnreadReviews,allItems:()=>[item],$:()=>({setAttribute(){}}),document:{querySelectorAll:()=>[{dataset:{workId:item.id}}],addEventListener(){}},registerRenderHook:(name,run)=>hook[name]=run,api:(path,args)=>{calls.push({path,args});return new Promise((resolve,reject)=>requests.push({path,resolve,reject}));},render:()=>renders++});
 const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);const github=fs.readFileSync('static/github-actions.js','utf8');vm.runInContext(github.slice(github.indexOf("if(typeof document!=="),github.indexOf('// Dependencies are branch targets')),c);hook['github:review-attention']();assert.equal(requests[0].path,'/api/github/reviews/seen');
 const newer={...c.state,revision:'two',localChange:'keep'};c.state=newer;requests[0].resolve({...newer,revision:'old-response',localChange:'lost'});await settle();assert.equal(c.state,newer);assert.equal(requests[1].path,'/api/state');
 const newest={...newer,revision:'three',localChange:'newest'};c.state=newest;requests[1].resolve({...newer,revision:'stale-get'});await settle();assert.equal(c.state,newest);assert.equal(requests[2].path,'/api/state');requests[2].resolve({...newest,githubReviewInbox:{[item.id]:{active:true,version:'cycle',seenAt:123}}});await settle();assert.equal(c.state.localChange,'newest');assert.equal(c.state.githubReviewInbox[item.id].seenAt,123);assert.equal(renders,1);assert.equal(calls.filter(call=>call.path==='/api/github/reviews/seen').length,1);
});

test('opening PR actions focuses an available action without running it and Escape returns to its trigger',()=>{
 for(const draft of [false,true]){
  const f=reviewPanelFixture();f.open(f.item(f.initial.id,{isDraft:draft,status:draft?'draft':'needs-review'}));assert.equal(f.requests.length,0);assert.equal(f.document.activeElement.dataset.githubFocus,draft?'ready':'reviewer');assert.ok(f.panel().contains(f.document.activeElement));const close=f.all().find(node=>node.dataset.githubFocus==='close');assert.equal(close.attrs['aria-label'],'Close PR actions');
  f.panel().onkeydown({key:'Escape',preventDefault(){},stopPropagation(){}});assert.equal(f.panel(),undefined);assert.equal(f.document.activeElement,f.anchor);
 }
});
test('pending and completed redraws keep focus on enabled controls so Escape works after Mark ready',async()=>{
 const f=reviewPanelFixture();f.open(f.item(f.initial.id,{isDraft:true,status:'draft'}));assert.equal(f.document.activeElement,f.button('Mark ready for review'));f.button('Mark ready for review').onclick();assert.ok(f.panel().contains(f.document.activeElement));assert.equal(f.document.activeElement.disabled,undefined);assert.equal(f.document.activeElement.dataset.githubFocus,'close');
 f.requests[0].resolve(githubReceipt(f.initial.id,'ready'));await settle();f.requests[1].resolve({...f.c.state,revision:'two',items:[f.item()]});await settle();assert.ok(f.panel().contains(f.document.activeElement));assert.equal(f.document.activeElement.dataset.githubFocus,'close');assert.equal(f.button('Mark ready for review'),undefined);
 f.panel().onkeydown({key:'Escape',preventDefault(){},stopPropagation(){}});assert.equal(f.panel(),undefined);assert.equal(f.document.activeElement,f.anchor);
});
test('closing returns to the matching replacement chip, then stable fallback, only when the panel owned focus',()=>{
 for(const replacement of [true,false]){
  const f=reviewPanelFixture();f.open();f.anchor.remove();let chip;
  if(replacement){const unrelated=new f.Element('button','github-reviewer-chip','Other PR');unrelated.dataset.focusKey='github:PR_B:reviewers';chip=new f.Element('button','github-reviewer-chip','New reviewer chip');chip.dataset.focusKey=f.initial.id+':reviewers';f.body.append(unrelated,chip);}
  f.all().find(node=>node.dataset.githubFocus==='close').onclick();assert.equal(f.document.activeElement,replacement?chip:f.fallback);
 }
 const f=reviewPanelFixture();f.open();const outside=new f.Element('input');f.body.append(outside);outside.focus();f.all().find(node=>node.dataset.githubFocus==='close').onclick();assert.equal(f.document.activeElement,outside);
});
test('async completion never steals focus from outside or a replacement PR panel',async()=>{
 for(const move of ['outside','replacement','closed']){
  const f=reviewPanelFixture();f.open();f.button('Refresh PR').onclick();let target;
  if(move==='outside'){target=new f.Element('input');f.body.append(target);target.focus();}else if(move==='closed'){f.all().find(node=>node.dataset.githubFocus==='close').onclick();target=f.anchor;}else{f.open(f.item('github:PR_B'));target=f.document.activeElement;}
  f.requests[0].resolve(githubReceipt(f.initial.id,'refresh'));await settle();if(move==='outside'){f.requests[1].resolve({...f.c.state,revision:'two'});await settle();}
  assert.equal(f.document.activeElement,target);assert.equal(target.isConnected,true);
 }
});


test('malformed reviewer stays editable with exact draft, associated inline error and owned correction focus before any API',()=>{
 for(const raw of ['', '   ', '@', '-user', 'user-', 'user_name', 'two words', 'a'.repeat(40), '@@user', 'é']){
  const f=reviewPanelFixture();f.open();const input=f.all().find(node=>node.tagName==='INPUT'),form=f.all().find(node=>node.tagName==='FORM'),children=[...f.panel().children];input.value=raw;input.oninput();f.button('Request review').focus();form.onsubmit({preventDefault(){}});
  assert.equal(f.calls.length,0,raw);assert.equal(input.value,raw);assert.equal(input.disabled,false);assert.equal(f.document.activeElement,input);assert.equal(input.attrs['aria-invalid'],'true');const error=f.all().find(node=>node.id===input.attrs['aria-describedby']);assert.equal(error.hidden,false);assert.equal(error.attrs.role,'alert');assert.ok(error.textContent.includes('GitHub username'));assert.deepEqual(f.panel().children,children);assert.equal(form.noValidate,true);
  input.selectionStart=2;input.value=' @valid-user ';input.oninput();assert.equal(input.attrs['aria-invalid'],undefined);assert.equal(input.attrs['aria-describedby'],undefined);assert.equal(error.hidden,true);assert.equal(error.textContent,'');assert.equal(input.selectionStart,2);assert.equal(f.document.activeElement,input);assert.deepEqual(f.panel().children,children);
  form.onsubmit({preventDefault(){}});assert.equal(f.calls.length,1);assert.equal(f.calls[0].args.args.login,'valid-user');
 }
});
test('local reviewer validation never takes outside focus or acts through a closed or replaced panel',()=>{
 for(const move of ['outside','closed','replacement']){
  const f=reviewPanelFixture();f.open();const input=f.all().find(node=>node.tagName==='INPUT'),form=f.all().find(node=>node.tagName==='FORM');input.value='bad_name';input.oninput();let target;
  if(move==='outside'){target=new f.Element('input');f.body.append(target);target.focus();}else if(move==='closed'){f.all().find(node=>node.dataset.githubFocus==='close').onclick();target=f.document.activeElement;}else{f.open(f.item('github:PR_B'));target=f.document.activeElement;}
  form.onsubmit({preventDefault(){}});assert.equal(f.calls.length,0);assert.equal(f.document.activeElement,target);if(move!=='outside')assert.equal(input.attrs['aria-invalid'],undefined);
 }
});
test('valid username syntax matches owner bounds and normalized requests while remote failures stay general',async()=>{
 for(const raw of ['a','1','a-b','a--b','a'.repeat(39),'@'+'a'.repeat(39)]){
  const f=reviewPanelFixture();f.open();const input=f.all().find(node=>node.tagName==='INPUT');input.value=raw;input.oninput();f.all().find(node=>node.tagName==='FORM').onsubmit({preventDefault(){}});assert.equal(f.calls.length,1);assert.equal(f.calls[0].args.args.login,raw.replace(/^@/,''));
  f.requests[0].reject(new Error('GitHub username not found.'));await settle();const current=f.all().find(node=>node.tagName==='INPUT');assert.equal(current.value,raw);assert.equal(current.attrs['aria-invalid'],undefined);assert.equal(current.attrs['aria-describedby'],undefined);assert.equal(current.disabled,false);
 }
});
