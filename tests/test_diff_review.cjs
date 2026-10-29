const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/diff-review.js','utf8');
test('continuous loading keeps other files available after an unsupported file and never mounts a closed review',async()=>{
  const mounted=[],unobserved=[],root={getBoundingClientRect:()=>({top:0,bottom:600})};
  const entry=()=>({status:'queued',section:{getBoundingClientRect:()=>({top:100,bottom:200})},content:{replaceChildren(){},classList:{add(){}}}});
  const current={record:{id:'agent'},files:new Map([['binary',entry()],['text',entry()]]),queue:[{path:'binary',forced:false},{path:'text',forced:false}],loading:false};
  const dialog={open:true};
  const c=vm.createContext({reviewState:current,$:id=>id==='#diff-review-dialog'?dialog:root,cachedReviewFile:async(record,file)=>{if(file==='binary')throw new Error('Binary file');return {path:file};},reviewRenderer:{mount:(node,data)=>mounted.push(data.path)},reviewAnnotations:()=>[],reviewObserver:{unobserve:node=>unobserved.push(node)},el:()=>({})});
  vm.runInContext(source.slice(source.indexOf('async function drainReviewFiles('),source.indexOf('function paintReviewComments(')),c);
  await c.drainReviewFiles(current);assert.deepEqual(mounted,['text']);assert.equal(current.files.get('binary').status,'error');assert.equal(current.loading,false);assert.equal(unobserved.length,2);
  current.queue.push({path:'text',forced:true});c.cachedReviewFile=async()=>{dialog.open=false;return {path:'text'};};await c.drainReviewFiles(current);assert.equal(mounted.length,1);
});
test('changing diff layout keeps mounted files and line annotations and passes fresh options to the renderer',()=>{
  const adapter=fs.readFileSync('desktop/frontend/diff-viewer.js','utf8'),views=[];
  class Virtualizer{setup(){}cleanUp(){}}
  class VirtualizedFileDiff{
    constructor(options){this.options=options;this.annotations=[];views.push(this);}
    render(){} setOptions(options){assert.notEqual(options,this.options);this.options=options;}
    rerender(){}setLineAnnotations(items){this.annotations=items;}cleanUp(){}
  }
  const c=vm.createContext({themeName:()=> 'mydash-rose-pine',getComputedStyle:()=>({getPropertyValue:()=>''}),document:{documentElement:{dataset:{theme:'rose-pine'}}},Virtualizer,VirtualizedFileDiff,registerCustomTheme(){},rosePine:{}});
  vm.runInContext(adapter.slice(adapter.indexOf('export function createDiffReview(')).replace('export function','function'),c);
  const controller=c.createDiffReview({},'unified');controller.mount({}, {path:'a',oldContent:'',newContent:'x'},[],()=>{});controller.mount({}, {path:'b',oldContent:'x',newContent:''},[],()=>{});
  controller.annotate('a',[{lineNumber:1}]);controller.setStyle('split');assert.deepEqual(views.map(v=>v.options.diffStyle),['split','split']);assert.equal(views[0].annotations.length,1);controller.setStyle('unified');assert.deepEqual(views.map(v=>v.options.diffStyle),['unified','unified']);
});

test('active chat cache deduplicates requests and discards responses after switching chats',async()=>{
 const pending=[],requests=[];const c=vm.createContext({api:(route,payload)=>{requests.push(payload);return new Promise(resolve=>pending.push(resolve));},Date});
 const begin=source.indexOf('const activeDiffCache='),end=source.indexOf('let reviewRefreshAt=');
 vm.runInContext(source.slice(begin,end),c);c.activateDiffCache('a\0/repo');
 const first=c.cachedReviewFile({id:'a'},'file','/repo'),second=c.cachedReviewFile({id:'a'},'file','/repo');assert.equal(requests.length,1);
 c.activateDiffCache('b\0/repo');pending.shift()({oldContent:'old',newContent:'new'});await Promise.all([first,second]);
 c.activateDiffCache('a\0/repo');const third=c.cachedReviewFile({id:'a'},'file','/repo');assert.equal(requests.length,2);pending.shift()({oldContent:'',newContent:'new'});await third;
});


test('comments from different reviewers on one line stay visible together and stale versions stay separate',()=>{
 const file={path:'src/api.rs',root:'/repo',fingerprint:'current'},base={...file,side:'additions',end:2};
 const c=vm.createContext({reviewState:{draft:{comments:[{...base,author:'You',text:'First'},{...base,author:'Codex · Performance',text:'Second'},{...base,fingerprint:'old',text:'Outdated'}]}}});
 vm.runInContext(source.slice(source.indexOf('function reviewAnnotations('),source.indexOf('function updateReviewAnnotations(')),c);
 const annotations=c.reviewAnnotations(file);assert.equal(annotations.length,1);assert.equal(annotations[0].metadata.length,2);assert.equal(annotations[0].metadata[0].author,'You');assert.equal(annotations[0].metadata[1].author,'Codex · Performance');assert.equal(annotations[0].metadata[1].text,'Second');assert.ok(!annotations[0].metadata.some(c=>c.text==='Outdated'));
});


test('an observed file finishes loading even when earlier mounts move it outside the viewport',async()=>{
 const mounted=[],entry={status:'queued',section:{getBoundingClientRect:()=>({top:2000,bottom:2200})},content:{replaceChildren(){},classList:{add(){}}}};
 const current={record:{id:'agent'},files:new Map([['next',entry]]),queue:[{path:'next',forced:false}],loading:false};
 const c=vm.createContext({reviewState:current,$:id=>id==='#diff-review-dialog'?{open:true}:{getBoundingClientRect:()=>({top:0,bottom:600})},cachedReviewFile:async()=>({path:'next'}),reviewRenderer:{mount:(node,data)=>mounted.push(data.path)},reviewAnnotations:()=>[],reviewObserver:{unobserve(){}},el:()=>({})});
 vm.runInContext(source.slice(source.indexOf('async function drainReviewFiles('),source.indexOf('function paintReviewComments(')),c);
 await c.drainReviewFiles(current);assert.deepEqual(mounted,['next']);assert.equal(entry.status,'loaded');assert.equal(current.loading,false);
});

test('review UI keeps unsaved text, range, drawer and scroll context across chat switches',()=>{
 const data=new Map(),record={id:'a'},section={dataset:{path:'src/api.rs'},getBoundingClientRect:()=>({top:40,bottom:200})};
 const nodes={'#diff-review-body':{getBoundingClientRect:()=>({top:100})},'#diff-review-drawer':{open:true},'#diff-review-assessments':{querySelectorAll:()=>[{dataset:{assessmentId:'assessment'}}]},'#diff-review-files':{querySelectorAll:()=>[]},'#diff-review-comment':{value:'Unsaved finding'},'#diff-review-note':{value:'Unsaved summary'}};
 const c=vm.createContext({localStorage:{getItem:k=>data.get(k)||null,setItem:(k,v)=>data.set(k,v)},reviewState:{record,directory:'/repo',mode:'split',files:new Map([['src/api.rs',{section}]]),file:{path:'src/api.rs',fingerprint:'v1'},range:{start:2,end:3,side:'additions'},noteDirty:true,noteBase:'Old summary',draft:{note:'Remote summary'}},$:id=>nodes[id]});
 vm.runInContext(source.slice(source.indexOf('function savedReviewUI('),source.indexOf('let reviewUISaveTimer')),c);c.rememberReviewUI();
 const saved=c.savedReviewUI(record,'/repo');assert.equal(saved.comment,'Unsaved finding');assert.equal(saved.note,'Unsaved summary');assert.equal(saved.noteBase,'Old summary');assert.equal(saved.noteDirty,true);assert.equal(saved.path,'src/api.rs');assert.equal(saved.offset,60);assert.equal(saved.range.end,3);assert.equal(saved.drawer,true);assert.deepEqual(Object.keys(c.savedReviewUI({id:'other'},'/repo')),[]);
});

test('Enter adds a review comment while Shift-Enter, composition and held keys do not submit',()=>{
 let clicked=0,prevented=0,stopped=0;const button={disabled:false,click(){if(!this.disabled)clicked++;}};
 const c=vm.createContext({$:id=>{assert.equal(id,'#diff-review-add');return button;}});
 vm.runInContext(source.slice(source.indexOf('function reviewCommentKeydown('),source.indexOf("$('#diff-review-comment').addEventListener('keydown'")),c);
 const key=extra=>({key:'Enter',preventDefault(){prevented++;},stopPropagation(){stopped++;},...extra});
 c.reviewCommentKeydown(key({}));assert.equal(clicked,1);assert.equal(prevented,1);assert.equal(stopped,1);
 for(const extra of [{shiftKey:true},{isComposing:true},{keyCode:229},{ctrlKey:true},{metaKey:true},{altKey:true},{key:'a'}])c.reviewCommentKeydown(key(extra));
 assert.equal(clicked,1);assert.equal(prevented,1);
 c.reviewCommentKeydown(key({repeat:true}));assert.equal(clicked,1);assert.equal(prevented,2);
 button.disabled=true;c.reviewCommentKeydown(key({}));assert.equal(clicked,1);
});

test('scrolling the review highlights the file at the reading position and reveals its tree row without moving the diff',()=>{
 const folder={tagName:'DETAILS',open:false},tree={scrollTop:0,getBoundingClientRect:()=>({top:0,bottom:100})};folder.parentElement=tree;
 const button=(path,top)=>({dataset:{path},parentElement:folder,selected:false,attributes:{},classList:{toggle(name,value){this.owner.selected=value;}},setAttribute(key,value){this.attributes[key]=value;},removeAttribute(key){delete this.attributes[key];},getBoundingClientRect:()=>({top,bottom:top+20})});
 const a=button('a',10),b=button('b',140);a.classList.owner=a;b.classList.owner=b;tree.querySelectorAll=()=>[a,b];
 let first={top:0,bottom:90},second={top:90,bottom:800};
 const state={files:new Map([['a',{section:{getBoundingClientRect:()=>first}}],['b',{section:{getBoundingClientRect:()=>second}}]])};
 const c=vm.createContext({reviewState:state,$:id=>id==='#diff-review-files'?tree:id==='#diff-review-dialog'?{open:true}:{getBoundingClientRect:()=>({top:0,bottom:500,height:500})}});
 vm.runInContext(source.slice(source.indexOf('function highlightReviewFile('),source.indexOf('function scheduleReviewFileHighlight(')),c);
 c.syncReviewFileHighlight();assert.equal(a.selected,true);assert.equal(a.attributes['aria-current'],'location');
 first={top:-100,bottom:-10};second={top:-10,bottom:600};c.syncReviewFileHighlight();assert.equal(a.selected,false);assert.equal(b.selected,true);assert.equal(b.attributes['aria-current'],'location');assert.equal(folder.open,true);assert.equal(tree.scrollTop,60);
 c.syncReviewFileHighlight();assert.equal(tree.scrollTop,60);state.quick=true;first={top:0,bottom:500};c.syncReviewFileHighlight();assert.equal(b.selected,true);
});

test('review code highlighting retains literal text and newlines without treating code as HTML',async()=>{
 const adapter=fs.readFileSync('desktop/frontend/diff-viewer.js','utf8'),code={textContent:'if x:\n  print("<script>")',classList:['language-python'],isConnected:true,replaceChildren(fragment){this.children=fragment.children;}};
 const c=vm.createContext({themeName:()=> 'mydash-rose-pine',getFiletypeFromFileName:()=>{throw new Error('Explicit language should win');},getSharedHighlighter:async options=>{assert.equal(options.langs[0],'python');return {codeToTokens(text,options){assert.equal(text,code.textContent);assert.equal(options.lang,'python');return {tokens:[[{content:'if',color:'#c4a7e7'},{content:' x:'}],[{content:'  print("<script>")',color:'#f6c177'}]]};}};},document:{createDocumentFragment:()=>({children:[],append(node){this.children.push(node);}}),createElement:()=>({style:{}}),createTextNode:text=>({textContent:text})}});
 vm.runInContext(adapter.slice(adapter.indexOf('export async function highlightReviewCode('),adapter.indexOf('export function createDiffReview(')).replace('export async function','async function'),c);
 await c.highlightReviewCode({querySelectorAll:()=>[code]},'example.rs');assert.equal(code.children.map(x=>x.textContent).join(''),code.textContent);assert.equal(code.children[0].style.color,'#c4a7e7');assert.equal(code.children[3].textContent,'  print("<script>")');
});

test('full review chooses full presentation before configuring the title and layout',()=>{
 const nodes=new Map(),settings=[];
 const node=id=>{if(!nodes.has(id))nodes.set(id,{classList:{toggle(){}},replaceChildren(){},append(){},remove(){}});return nodes.get(id);};
 const c=vm.createContext({reviewState:null,$:node,Event,source:'agents',document:{activeElement:null,querySelectorAll:()=>[]},captureDialogReturn(){},gitReviewFocusKey:()=> 'review',rememberReviewUI(){},disposeReview(){},setting:(key,fallback)=>{settings.push(key);return fallback;},localStorage:{getItem:()=>null},savedReviewUI:()=>({}),sharedReviewDrafts:new Map(),gitDiffDirectories:()=>[]});
 const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog')")),c);
 const start=source.indexOf('async function openDiffReview('),end=source.indexOf('  hideReviewComposer();',start);
 vm.runInContext(source.slice(start,end)+'return current;}',c);
 return c.openDiffReview({id:'chat',title:'A chat'},'src/file.rs','/repo','full').then(current=>{
   assert.equal(current.quick,false);assert.equal(node('#diff-review-title').textContent,'Review');assert.equal(node('#diff-review-expand').hidden,true);assert.equal(settings[0],'diffLayout');
 });
});

test('inspector geometry stays scoped to preview CSS and uses local coordinates under UI scaling',()=>{
 for(const scale of [.85,1,1.25]){
  const values=new Map(),current={quick:true},dialog={style:{setProperty:(key,value)=>values.set(key,value)}};
  const bounds={left:200*scale,right:1000*scale,top:50*scale,height:700*scale};
  const c=vm.createContext({reviewState:current,$:id=>id==='#diff-review-dialog'?dialog:{getBoundingClientRect:()=>bounds},getComputedStyle:()=>({getPropertyValue:()=>String(scale)}),document:{documentElement:{}},window:{innerWidth:1200*scale,innerHeight:800*scale},setting:()=>60});
  vm.runInContext(source.slice(source.indexOf('function positionQuickReview('),source.indexOf('function savedReviewUI(')),c);
  c.positionQuickReview(current,{getBoundingClientRect:()=>({top:100*scale})});
  assert.equal(values.get('--inspector-height'),'420px');assert.equal(values.get('--inspector-left'),'200px');assert.equal(values.get('--inspector-right'),'200px');assert.equal(values.get('--inspector-top'),'100px');assert.equal(dialog.style.height,undefined);
 }
});


test('new review saves the note before archiving and never resets an active review',async()=>{
 const current={record:{id:'chat'},draft:{revision:4,note:'Previous note'},noteDirty:true},calls=[];
 const nodes={'#diff-review-comment':{value:''},'#diff-review-note':{value:'Previous note'},'#diff-review-error':{textContent:''},'#diff-review-history':{open:false}};
 const c=vm.createContext({reviewState:current,$:id=>nodes[id],paintReviewComments(){},saveReviewNote:async()=>calls.push('note'),reviewAction:async(id,action,args)=>{calls.push([id,action,args.revision]);current.draft={revision:5,note:''};},hideReviewComposer(){},updateReviewAnnotations(){},rememberReviewUI(){},toast(){}});
 vm.runInContext(source.slice(source.indexOf('async function startNewReview('),source.indexOf("$('#diff-review-new').onclick=")),c);
 await c.startNewReview();assert.equal(calls[0],'note');assert.deepEqual(calls[1],['chat','new',4]);assert.equal(nodes['#diff-review-note'].value,'');assert.equal(current.pending,false);
 calls.length=0;current.draft.run={state:'reviewing'};await c.startNewReview();assert.equal(calls.length,0);
 delete current.draft.run;nodes['#diff-review-comment'].value='Unsubmitted finding';await c.startNewReview();assert.equal(calls.length,0);assert.match(nodes['#diff-review-error'].textContent,/Add or cancel/);
});


test('loading previous reviews does not replace or save the current draft',async()=>{
 const draft={revision:8,note:'Current unsaved note',comments:[{id:'current'}]},current={record:{id:'chat'},draft,noteDirty:true},history=[{id:'older',archivedAt:1,review:{note:'Old note'}}],calls=[];
 const nodes={'#diff-review-history':{open:true},'#diff-review-drawer':{open:true},'#diff-review-history-content':{replaceChildren(){}}};
 const c=vm.createContext({reviewState:current,$:id=>nodes[id],el:()=>({}),api:async(route,payload)=>{calls.push([route,payload.action]);return {reviews:history};},paintReviewHistoryList:reviews=>assert.equal(reviews,history)});
 vm.runInContext(source.slice(source.indexOf('async function loadReviewHistory('),source.indexOf("$('#diff-review-history').addEventListener")),c);
 await c.loadReviewHistory();assert.equal(current.draft,draft);assert.equal(current.noteDirty,true);assert.equal(nodes['#diff-review-drawer'].open,false);assert.deepEqual(calls,[['/api/session/review-draft','history']]);
});

test('review comments render safe Markdown while source excerpts keep literal whitespace and markup',()=>{
 const nodes=[];
 const el=(tag,cls,text)=>{const node={tag,className:cls||'',textContent:text||'',children:[],append(...items){this.children.push(...items);},querySelector:()=>null};nodes.push(node);return node;};
 const c=vm.createContext({URL,el});
 for(const file of ['static/vendor/markdown-it.min.js','static/markdown.js'])vm.runInContext(fs.readFileSync(file,'utf8'),c);
 vm.runInContext(source.slice(source.indexOf('function highlightReviewMarkup('),source.indexOf('function renderReviewAnnotations(')),c);
 const body=c.reviewCommentBody('Keep `x_test_bytes` unchanged.\n\n```python\nif ready:\n    print("<script>")\n```\n\n<img src="https://tracker.example/pixel">','test_bytes.py');
 assert.match(body.innerHTML,/<code>x_test_bytes<\/code>/);assert.match(body.innerHTML,/<pre><code class="language-python">if ready:\n    print\(&quot;&lt;script&gt;&quot;\)/);assert.ok(!body.innerHTML.includes('<img'));
 const text='x_test_bytes = "<img onerror=alert(1)>"\n\tif ready:\n    assert x_test_bytes == "```"\n';
 const excerpt=c.reviewSourceExcerpt(text,'test_bytes.py');assert.equal(excerpt.tag,'figure');assert.equal(excerpt.children[0].tag,'figcaption');assert.equal(excerpt.children[0].textContent,'Earlier file version');assert.equal(excerpt.children[1].tag,'pre');const code=excerpt.children[1].children[0];assert.equal(code.tag,'code');assert.equal(code.textContent,text);assert.equal(code.innerHTML,undefined);
 const bounded=c.reviewSourceExcerpt('x'.repeat(16001),'test_bytes.py');assert.equal(bounded.children[1].children[0].textContent.length,16000);assert.match(bounded.children[2].textContent,/truncated/);
});

test('stale comments and archived comments retain source blocks and individually labelled removal actions',async()=>{
 const el=(tag,cls,text)=>({tag,className:cls||'',textContent:text||'',children:[],attrs:{},dataset:{},setAttribute(key,value){this.attrs[key]=value;},append(...items){this.children.push(...items);},prepend(...items){this.children.unshift(...items);},replaceChildren(...items){this.children=items;},querySelector:()=>null,querySelectorAll:()=>[]});
 const actions=[],list=el('div'),assessments=el('div'),history=el('div'),comment={id:'one',author:'You',path:'test.py',root:'/repo',start:1,end:2,text:'Check `x`.',excerpt:'if x:\n    return x\n',fingerprint:'old',freshness:'outdated'},nodes={'#diff-review-comments':list,'#diff-review-assessments':assessments,'#diff-review-history-content':history,'#diff-review-error':el('p')};
 const c=vm.createContext({document:{activeElement:null},el,$:id=>nodes[id],reviewState:{record:{id:'agent'},directory:'/repo',draft:{comments:[comment,{...comment,id:'two'}],assessments:[]},files:new Map([['test.py',{data:{fingerprint:'current'}}]])},renderBriefingMarkdown:text=>text,reviewAuthorPill:author=>el('span','',author),reviewAction:async(...args)=>actions.push(args),pendingReviewAssessments:()=>[],pendingReviewComments:()=>[],pendingReviewNote:()=>''});
 vm.runInContext(source.slice(source.indexOf('function highlightReviewMarkup('),source.indexOf('function renderReviewAnnotations(')),c);
 const features=fs.readFileSync('static/features.js','utf8');vm.runInContext(features.slice(features.indexOf('function featureButton('),features.indexOf('// Fixed feature forms')),c);
 const start=source.indexOf('function paintReviewComments('),end=source.indexOf("  const active=['requested'",start);vm.runInContext(source.slice(start,end)+'}',c);c.paintReviewComments();
 const remove=list.children.map(row=>row.children[0].children.at(-1));assert.equal(remove[0].attrs['aria-label'],'Remove review comment 1 at test.py:1–2');assert.equal(remove[1].attrs['aria-label'],'Remove review comment 2 at test.py:1–2');assert.equal(remove[0].textContent,'×');assert.equal(remove[0].type,'button');await remove[1].onclick();assert.deepEqual(JSON.parse(JSON.stringify(actions)),[['agent','remove',{commentIds:['two']}]]);c.reviewAction=async()=>{throw Error('Cannot remove');};await remove[0].onclick();assert.equal(nodes['#diff-review-error'].textContent,'Cannot remove');
 const excerpt=list.children[0].children.find(node=>node.className==='review-source-excerpt');assert.equal(excerpt.children[1].children[0].textContent,comment.excerpt);
 vm.runInContext(source.slice(source.indexOf('function showReviewHistoryEntry('),source.indexOf('function paintReviewHistoryList(')),c);c.showReviewHistoryEntry({archivedAt:1,review:{comments:[comment]}});
 const archived=history.children.find(node=>node.className==='review-comment').children.find(node=>node.className==='review-source-excerpt');assert.equal(archived.children[0].textContent,'Commented file version');assert.equal(archived.children[1].children[0].textContent,comment.excerpt);
});

test('review highlighting keeps original CRLF and trailing newline when tokenization normalizes them',async()=>{
 const adapter=fs.readFileSync('desktop/frontend/diff-viewer.js','utf8');let replacements=0;
 const c=vm.createContext({themeName:()=> 'mydash-rose-pine',getSharedHighlighter:async()=>({codeToTokens:()=>({tokens:[[{content:'first'}],[{content:'second'}]]})}),document:{createDocumentFragment:()=>({})}});
 vm.runInContext(adapter.slice(adapter.indexOf('export async function highlightReviewCode('),adapter.indexOf('export function createDiffReview(')).replace('export async function','async function'),c);
 for(const text of ['first\r\nsecond','first\nsecond\n']){const code={textContent:text,classList:['language-python'],isConnected:true,replaceChildren(){replacements++;}};await c.highlightReviewCode({querySelectorAll:()=>[code]},'test.py');assert.equal(code.textContent,text);}assert.equal(replacements,0);
});

function reviewFocusFixture(){
 let document;const nodes=new Map(),requests=[];
 const matches=(node,selector)=>selector.split(',').some(part=>{part=part.trim();if(part==='a[href]')return node.tag==='a'&&node.attrs.href!==undefined;if(part==='[hidden]')return node.hidden;if(part==='dialog[open]')return node.tag==='dialog'&&node.open;if(part==='details[open]')return node.tag==='details'&&node.open;if(part==='[data-review-entry]')return !!node.dataset.reviewEntry;if(part==='[data-review-control]')return !!node.dataset.reviewControl;return false;});
 class Node{
  constructor(tag,cls='',text=''){this.tag=tag;this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.value='';this.open=false;}
  set innerHTML(value){this.html=value;this.replaceChildren();for(const match of value.matchAll(/<a\s[^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g)){const link=new Node('a','',match[2]);link.setAttribute('href',match[1].replaceAll('&amp;','&').replaceAll('&quot;','"'));this.append(link);}}
  get innerHTML(){return this.html;}
  set open(value){this.expanded=Boolean(value);}get open(){return this.expanded;}
  get tagName(){return this.tag.toUpperCase();}
  get isConnected(){return this===document.body||!!this.parentElement?.isConnected;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  append(...children){for(const child of children){child.parentElement=this;this.children.push(child);}}prepend(...children){for(const child of children.reverse()){child.parentElement=this;this.children.unshift(child);}}
  replaceChildren(...children){if(this.contains(document.activeElement))document.activeElement=document.body;for(const child of this.children)child.parentElement=null;this.children=[];this.append(...children);}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(matches(child,selector)?[child]:[]),...child.querySelectorAll(selector)]);}querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  closest(selector){for(let node=this;node;node=node.parentElement)if(matches(node,selector))return node;return null;}
  getClientRects(){if(!this.isConnected||this.closest('[hidden]'))return [];let child=this;for(let parent=this.parentElement;parent;parent=parent.parentElement){if(parent.tag==='dialog'&&!parent.open||parent.tag==='details'&&!parent.open&&child.tag!=='summary')return [];child=parent;}return [{}];}
  focus(options){if(this.getClientRects().length&&!this.disabled){document.activeElement=this;this.focusOptions=options;}}
  setAttribute(key,value){this.attrs[key]=value;}getAttribute(key){return this.attrs[key]??null;}
 }
 document={body:null,activeElement:null,querySelectorAll:selector=>document.body.querySelectorAll(selector)};document.body=new Node('body');document.activeElement=document.body;
 const dialog=new Node('dialog');dialog.open=true;document.body.append(dialog);
 const $=selector=>{if(selector==='#diff-review-dialog')return dialog;if(!nodes.has(selector)){const node=new Node(selector==='#diff-review-drawer-toggle'?'summary':'div');nodes.set(selector,node);dialog.append(node);}return nodes.get(selector);};
 const draft={revision:1,note:'',comments:['one','two','three'].map(id=>({id,author:'You',path:'test.py',root:'/repo',start:1,end:2,text:'Check',fingerprint:'same'})),assessments:['a','b','c'].map(id=>({id,author:'Reviewer',pass:'correctness',createdAt:1,text:'Review'}))};
 const current={record:{id:'chat'},directory:'/repo',draft,files:new Map(),noteDirty:false};const window={dispatchEvent(){}};
 const c=vm.createContext({document,window,Event,reviewState:current,sharedReviewDrafts:new Map([['chat',draft]]),$: $,el:(...args)=>new Node(...args),reviewAuthorPill:author=>new Node('span','',author),reviewCommentBody:text=>new Node('div','',text),reviewSourceExcerpt:text=>new Node('pre','',text),pendingReviewAssessments:d=>d.assessments||[],pendingReviewComments:d=>d.comments||[],pendingReviewNote:()=>'',reviewRunLabel:()=>'',paintReviewDispatch(){},updateReviewAnnotations(){},api:(path,args)=>new Promise((resolve,reject)=>requests.push({path,args,resolve,reject}))});
 c.URL=URL;for(const file of ['static/vendor/markdown-it.min.js','static/markdown.js'])vm.runInContext(fs.readFileSync(file,'utf8'),c);
 vm.runInContext(source.slice(source.indexOf('function highlightReviewMarkup('),source.indexOf('function renderReviewAnnotations(')),c);
 const features=fs.readFileSync('static/features.js','utf8');vm.runInContext(features.slice(features.indexOf('function featureButton('),features.indexOf('// Fixed feature forms')),c);
 vm.runInContext(source.slice(source.indexOf('function acceptReviewDraft('),source.indexOf('const activeDiffCache=')),c);
 vm.runInContext(source.slice(source.indexOf('function paintReviewComments('),source.indexOf('async function saveReviewNote(')),c);c.paintReviewComments();
 const row=(kind,id)=>$(kind==='comment'?'#diff-review-comments':'#diff-review-assessments').children.find(node=>node.dataset.reviewId===id),control=(kind,id,role)=>row(kind,id)?.querySelectorAll('[data-review-control]').find(node=>node.dataset.reviewControl===role);
 const response=d=>({review:d});return {c,current,draft,document,dialog,Node,$,row,control,requests,response};
}
test('actual review polling preserves stable comment and assessment controls and expanded assessment state',async()=>{
 for(const [kind,id,role]of [['comment','two','anchor'],['comment','two','remove'],['assessment','b','summary'],['assessment','b','remove']]){
  const f=reviewFocusFixture();f.row('assessment','b').open=true;const old=f.control(kind,id,role);old.focus();assert.equal(f.document.activeElement,old);const reading=f.c.reviewAction('chat','check');const draft=structuredClone(f.draft);draft.revision++;draft.comments.reverse();draft.assessments.reverse();draft.assessments[0].freshness='outdated';f.requests[0].resolve(f.response(draft));await reading;
  const next=f.control(kind,id,role);assert.notEqual(next,old);assert.equal(old.isConnected,false);assert.equal(f.document.activeElement,next);assert.equal(next.focusOptions?.preventScroll,true);assert.equal(f.row('assessment','b').open,true);assert.equal(f.requests.length,1);
 }
});
test('actual remove handlers move to next equivalent, previous equivalent, then surviving drawer toggle',async()=>{
 for(const kind of ['comment','assessment']){
  const f=reviewFocusFixture(),ids=kind==='comment'?['two','three','one']:['b','c','a'],remaining=kind==='comment'?['three','one']:['c','a'];for(const id of ['a','b','c'])f.row('assessment',id).open=true;
  for(let index=0;index<ids.length;index++){
   const id=ids[index],button=f.control(kind,id,'remove');button.focus();assert.equal(f.document.activeElement,button);const removing=button.onclick();const request=f.requests.at(-1);assert.deepEqual(JSON.parse(JSON.stringify(request.args)),{id:'chat',action:'remove',[kind==='comment'?'commentIds':'assessmentIds']:[id]});
   const draft=structuredClone(f.current.draft),key=kind==='comment'?'comments':'assessments';draft.revision++;draft[key]=draft[key].filter(row=>row.id!==id);request.resolve(f.response(draft));await removing;
   assert.equal(f.document.activeElement,index<2?f.control(kind,remaining[index],'remove'):f.$('#diff-review-drawer-toggle'));assert.equal(button.isConnected,false);
  }
 }
});
test('review redraw never takes focus from outside, child dialog, a closed dialog or replaced owner',async()=>{
 for(const move of ['outside','child','closed','replaced','paint-outside','paint-child','paint-child-body','paint-closed','paint-replaced']){
  const f=reviewFocusFixture(),old=f.control('comment','one','anchor');old.focus();const reading=f.c.reviewAction('chat','check');let target;
  const change=()=>{if(move==='paint-child-body'){const child=new f.Node('dialog');child.open=true;f.document.body.append(child);target=f.document.body;f.document.activeElement=target;}else if(move.endsWith('outside')){target=new f.Node('input');f.document.body.append(target);target.focus();}else if(move.endsWith('child')){target=new f.Node('dialog');target.open=true;f.document.body.append(target);target.focus();}else if(move.endsWith('closed')){f.dialog.open=false;f.document.activeElement=f.document.body;target=f.document.body;}else{f.c.reviewState={record:{id:'other'}};target=f.document.body;f.document.activeElement=target;}};
  if(move.startsWith('paint-'))f.c.paintReviewDispatch=change;else change();const draft=structuredClone(f.draft);draft.revision++;f.requests[0].resolve(f.response(draft));await reading;assert.equal(f.document.activeElement,target,move);
 }
});
test('review focus falls back to a visible summary when a remove target is collapsed, and to toggle when section is hidden',()=>{
 const f=reviewFocusFixture();f.row('assessment','b').open=true;f.control('assessment','b','remove').focus();f.c.paintReviewDispatch=()=>{f.row('assessment','b').open=false;};f.c.paintReviewComments();assert.equal(f.document.activeElement,f.control('assessment','b','summary'));
 f.c.paintReviewDispatch=()=>{f.$('#diff-review-assessments').hidden=true;};f.c.paintReviewComments();assert.equal(f.document.activeElement,f.$('#diff-review-drawer-toggle'));
});

test('actual safe Markdown body links preserve exact destination and duplicate occurrence through polling',async()=>{
 for(const kind of ['comment','assessment']){
  const f=reviewFocusFixture(),key=kind==='comment'?'comments':'assessments',id=kind==='comment'?'two':'b',text='[First](https://example.test/a?x=1&y=2) [Other](https://example.test/b) [Again](https://example.test/a?x=1&y=2)';f.current.draft[key].find(row=>row.id===id).text=text;f.c.paintReviewComments();if(kind==='assessment')f.row(kind,id).open=true;
  const links=f.row(kind,id).querySelectorAll('a[href]');assert.equal(links.length,3);assert.equal(links[2].getAttribute('href'),'https://example.test/a?x=1&y=2');links[2].focus();const pending=f.c.reviewAction('chat','check'),draft=structuredClone(f.current.draft);draft.revision++;draft[key].reverse();draft[key].find(row=>row.id===id).text='[New unrelated](https://example.test/new) '+text+' https://example.test/linkified';f.requests[0].resolve(f.response(draft));await pending;
  const next=f.row(kind,id).querySelectorAll('a[href]');assert.equal(next.length,5);assert.equal(f.document.activeElement,next[3]);assert.equal(next[3].getAttribute('href'),links[2].getAttribute('href'));assert.equal(next[3].focusOptions.preventScroll,true);assert.equal(links[2].isConnected,false);
 }
});
test('changed or removed Markdown link falls back to known entry control without following another destination',async()=>{
 for(const change of ['destination','removed','duplicate-removed']){
  const f=reviewFocusFixture(),row=f.current.draft.comments.find(row=>row.id==='two');row.text='[First](https://example.test/a) [Again](https://example.test/a)';f.c.paintReviewComments();const old=f.row('comment','two').querySelectorAll('a[href]')[change==='duplicate-removed'?1:0];old.focus();const pending=f.c.reviewAction('chat','check'),draft=structuredClone(f.current.draft);draft.revision++;draft.comments.find(row=>row.id==='two').text=change==='removed'?'No link remains.':change==='destination'?'[Changed](https://example.test/b)':'[First](https://example.test/a)';f.requests[0].resolve(f.response(draft));await pending;assert.equal(f.document.activeElement,f.control('comment','two','anchor'));assert.equal(old.isConnected,false);assert.equal(f.requests.length,1);
 }
});

test('collapsed details controls with nonempty native bounds fall back to the next direct summary',async()=>{
 const f=reviewFocusFixture();f.row('assessment','a').open=true;const button=f.control('assessment','a','remove');button.focus();const removing=button.onclick();
 f.c.paintReviewDispatch=()=>{const next=f.control('assessment','b','remove');next.getClientRects=()=>[{}];const summary=f.control('assessment','b','summary');summary.getClientRects=()=>[{}];};
 const draft=structuredClone(f.current.draft);draft.revision++;draft.assessments.shift();f.requests[0].resolve(f.response(draft));await removing;
 assert.equal(f.row('assessment','b').open,false);assert.equal(f.control('assessment','b','remove').getClientRects().length,1);assert.equal(f.document.activeElement,f.control('assessment','b','summary'));
});
test('closed outer details blocks nested controls despite nonempty bounds, allowing only its first direct summary',()=>{
 const f=reviewFocusFixture(),outer=new f.Node('details'),toggle=f.$('#diff-review-drawer-toggle'),assessments=f.$('#diff-review-assessments');outer.open=true;
 f.dialog.children=f.dialog.children.filter(node=>node!==toggle&&node!==assessments);outer.append(toggle,assessments);f.dialog.append(outer);f.row('assessment','b').open=true;f.control('assessment','b','remove').focus();
 f.c.paintReviewDispatch=()=>{outer.open=false;for(const node of f.row('assessment','b').querySelectorAll('[data-review-control]'))node.getClientRects=()=>[{}];toggle.getClientRects=()=>[{}];};f.c.paintReviewComments();assert.equal(f.document.activeElement,toggle);
 // A later SUMMARY is ordinary collapsed content, not the details toggle.
 const first=new f.Node('summary');outer.prepend(first);outer.open=true;f.row('assessment','b').open=true;f.control('assessment','b','remove').focus();f.c.paintReviewDispatch=()=>{outer.open=false;for(const node of f.row('assessment','b').querySelectorAll('[data-review-control]'))node.getClientRects=()=>[{}];};f.c.paintReviewComments();assert.notEqual(f.document.activeElement,toggle);assert.equal(f.document.activeElement,f.document.body);
});
