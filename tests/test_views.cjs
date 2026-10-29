const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const defaults=JSON.parse(fs.readFileSync('static/config/views.json','utf8'));
function fixture({prefs={},responses=[]}={}){
 const calls=[],storage=new Map(),elements=new Map();let renders=0;
 class Element{
  constructor(tag,cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.children=[];this.attrs={};this.dataset={};this.value='';this.listeners={};this._disabled=false;this.inert=false;this.classList={toggle(){},contains:name=>this.className.split(' ').includes(name)};}
  append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}
  replaceChildren(...nodes){if(this.children.some(node=>node.contains(document.activeElement)))document.activeElement=document.body;for(const node of this.children)node.parentElement=null;this.children=[];this.append(...nodes);}
  setAttribute(name,value){this.attrs[name]=value;}
  add(option){this.children.push(option);}
  addEventListener(type,callback){(this.listeners[type]??=[]).push(callback);}
  dispatchEvent(event){for(const callback of this.listeners[event.type]||[])callback(event);return !event.defaultPrevented;}
  get disabled(){return this._disabled;}
  set disabled(value){this._disabled=value;if(value&&document.activeElement===this)document.activeElement=document.body;}
  matches(selector){return selector.split(',').some(part=>part==='dialog[open]'?this.tagName==='dialog'&&this.open:part.startsWith('#')?this.id===part.slice(1):part.startsWith('.')?this.className.split(' ').includes(part.slice(1)):this.tagName===part);}
  closest(selector){for(let node=this;node;node=node.parentElement)if(node.matches(selector))return node;return null;}
  get isConnected(){return this===document.body||!!this.parentElement?.isConnected;}
  getClientRects(){let node=this;while(node){if(node.hidden)return [];node=node.parentElement;}return this.isConnected?[{}]:[];}
  focus(options){if(this.disabled)return;this.focused=true;this.focusOptions=options;document.activeElement=this;}
  contains(node){return Boolean(node)&&descendants(this).includes(node);}
  getAttribute(name){return this.attrs[name]??null;}
  querySelectorAll(selector){const tags=selector.split(',');return descendants(this).slice(1).filter(node=>tags.includes(node.tagName));}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  showModal(){this.open=true;}
  close(){this.open=false;for(const callback of this.listeners.close||[])callback();}
  remove(){if(this.contains(document.activeElement))document.activeElement=document.body;if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(node=>node!==this);this.parentElement=null;}
  insertAdjacentElement(_,node){this.after=node;}
 }
 const document={querySelector(selector){return elements.get(selector)||null;},querySelectorAll:selector=>descendants(document.body).filter(node=>node.matches(selector)),body:new Element('body')};document.activeElement=document.body;
 const c=vm.createContext({Event,TextEncoder,console,crypto:{randomUUID:()=>String(calls.length)},localStorage:{getItem:key=>storage.get(key)||null,setItem:(key,value)=>storage.set(key,value)},prefs,groupingPrefs:{github:'stack',linear:'status',all:'status'},source:'github',sourceNames:{github:'GitHub'},document,getComputedStyle:node=>({visibility:node.visibility||'visible'}),el:(tag,cls,text)=>new Element(tag,cls,text),Option:class extends Element{constructor(text,value){super('option','',text);this.text=text;this.value=value;}},api:async(path,payload)=>{calls.push({path,payload});const next=responses.shift();if(next instanceof Error)throw next;return next;},render:()=>renders++,persist(){},allItems:()=>[]});
 const app=fs.readFileSync('static/app.js','utf8'),features=fs.readFileSync('static/features.js','utf8');vm.runInContext(app.slice(app.indexOf('function holdFormFocus('),app.indexOf('function projectCorrectionTarget(')),c);vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog')")),c);vm.runInContext(features.slice(features.indexOf('function freezeFeatureForm('),features.indexOf('function presentFeatureFormResult(')),c);c.workReference=item=>item.reference||'';vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function workViewAllows(')),c);vm.runInContext(fs.readFileSync('static/views.js','utf8'),c);
 return {c,calls,elements,Element,document,get renders(){return renders;},setDoc(doc={...defaults,contentHash:'a'.repeat(64)}){c.doc=doc;vm.runInContext('savedViewsDocument=validateSavedViewsDocument(doc);savedViewsError="";',c);},state:()=>JSON.parse(vm.runInContext('JSON.stringify(savedViewPageStates)',c))};
}
const predicate=(op,field,values)=>({op,field,values});
test('typed nested filters apply AND/OR and reject missing negative fields',()=>{
 const {c}=fixture();
 const filter={op:'all',conditions:[predicate('in','source',['github']),{op:'any',conditions:[{op:'equals',field:'pinned',value:true},predicate('containsAny','relations',['reviews'])]}]};
 c.validateSavedViewFilter(filter);assert.equal(c.matchesSavedViewFilter(filter,{source:'github',relations:['reviews']}),true);assert.equal(c.matchesSavedViewFilter(filter,{source:'linear',pinned:true}),false);
 for(const filter of [predicate('notIn','status',['done']),predicate('containsNone','relations',['reviews'])])assert.equal(c.matchesSavedViewFilter(filter,{}),false);
 assert.equal(c.matchesSavedViewFilter(predicate('containsNone','relations',['reviews']),{relations:[]}),true);assert.equal(c.matchesSavedViewFilter(predicate('containsNone','relations',['reviews']),{relations:[null]}),false);
 assert.equal(c.matchesSavedViewFilter(predicate('in','scope',['owner/repo']),{source:'github',url:'https://github.com/owner/repo/pull/2',project:'repo'}),true);assert.equal(c.matchesSavedViewFilter(predicate('notIn','scope',['Latency']),{}),false);
 assert.equal(c.matchesSavedViewFilter({op:'all',conditions:[]},{}),true);assert.equal(c.matchesSavedViewFilter({op:'any',conditions:[]},{}),false);
});
test('lifecycle includes unknown Active states, Completed is explicit, source parents constrain views',()=>{
 const {c}=fixture(),base={parent:'all',filter:{op:'all',conditions:[]}};
 for(const status of ['todo','unknown',undefined])assert.equal(c.savedViewMatches({...base,lifecycle:'active'},{status}),true);
 assert.equal(c.savedViewMatches({...base,lifecycle:'active'},{status:'done'}),false);assert.equal(c.savedViewMatches({...base,lifecycle:'completed'},{}),false);assert.equal(c.savedViewMatches({...base,lifecycle:'completed'},{status:'done'}),true);assert.equal(c.savedViewMatches({...base,lifecycle:'any'},{status:'done'}),true);
 assert.equal(c.savedViewMatches({...base,lifecycle:'any',parent:'github'},{source:'linear'}),false);
 const mine=defaults.views[0],review=defaults.views[1],item={source:'github',kind:'Pull request',status:'draft',relations:['authored']};assert.equal(c.savedViewMatches(mine,item),true);assert.equal(c.savedViewMatches(review,item),false);assert.equal(c.savedViewMatches(mine,{...item,kind:'Issue'}),false);
});
test('validation rejects executable or malformed predicates and bounded documents',()=>{
 const {c}=fixture();
 for(const filter of [{op:'eval',value:'true'},predicate('containsAny','status',['done']),predicate('in','status',[]),predicate('in','status',['done','done']),{op:'equals',field:'source',value:true},{op:'all',conditions:[],script:'true'}])assert.throws(()=>c.validateSavedViewFilter(filter));
 let filter={op:'all',conditions:[]};for(let i=0;i<5;i++)filter={op:'all',conditions:[filter]};assert.throws(()=>c.validateSavedViewFilter(filter));
 const doc={...defaults,contentHash:'a'.repeat(64)};c.validateSavedViewsDocument(doc);assert.throws(()=>c.validateSavedViewsDocument({...doc,version:2}));assert.throws(()=>c.validateSavedViewsDocument({...doc,views:[doc.views[0],doc.views[0]]}));assert.throws(()=>c.validateSavedViewsDocument({...doc,views:Array.from({length:65},(_,i)=>({...doc.views[0],id:'view-'+i}))}));assert.throws(()=>c.validateSavedViewsDocument({...doc,views:[{...doc.views[0],query:'x'.repeat(65537)}]}));
});
test('legacy preferences migrate per page and selecting saved views restores query/grouping',async()=>{
 const doc={...defaults,contentHash:'a'.repeat(64)},f=fixture({prefs:{status:'all',githubView:'review'},responses:[doc]});f.c.currentSavedViewState('github');await f.c.loadSavedViews();assert.equal(f.state().github.id,'github-review');assert.equal(f.state().github.draft.lifecycle,'active');assert.equal(f.state().linear.draft.lifecycle,'any');
 const query=new f.Element('input');f.elements.set('#search',query);f.c.activateSavedView('github-mine');assert.equal(f.state().github.id,'github-mine');assert.equal(query.value,'');assert.equal(f.c.groupingPrefs.github,'stack');
 query.value='API';f.c.savedViewCapturePresentation();assert.equal(f.state().github.draft.query,'API');f.c.savedViewUsePage('linear');assert.equal(query.value,'');f.c.savedViewUsePage('github');assert.equal(query.value,'API');
 const second=fixture({prefs:{status:'blocked'}});assert.equal(second.c.currentSavedViewState('linear').draft.lifecycle,'any');assert.equal(second.c.savedViewAllows({source:'linear',status:'todo'},'linear'),false);assert.equal(second.c.savedViewAllows({source:'linear',status:'blocked'},'linear'),true);
});
test('save sends revision and content hash, a conflict preserves the current draft',async()=>{
 const initial={...defaults,contentHash:'a'.repeat(64)},saved={...defaults,revision:1,contentHash:'b'.repeat(64)};saved.views=structuredClone(defaults.views);saved.views[0].name='Authored';
 const f=fixture({responses:[saved,new Error('Saved views changed. Reload the views before saving again.')]});f.setDoc(initial);const before=f.state(),changed={...defaults.views[0],name:'Authored'};await f.c.saveSavedViewDefinition(changed,{replaceId:'github-mine'});assert.equal(f.calls[0].payload.revision,0);assert.equal(f.calls[0].payload.contentHash,initial.contentHash);assert.equal(f.calls[0].payload.views.length,2);assert.deepEqual(f.state(),before);assert.equal(vm.runInContext('savedViewsDocument.views[0].name',f.c),'Authored');
 await assert.rejects(f.c.saveSavedViewDefinition({...changed,name:'Again'},{replaceId:'github-mine'}),/changed/);assert.equal(f.calls[1].payload.revision,1);assert.equal(f.calls[1].payload.contentHash,saved.contentHash);assert.deepEqual(f.state(),before);assert.equal(vm.runInContext('savedViewsDocument.views[0].name',f.c),'Authored');
});
test('compact controls change lifecycle and the editor builds nested typed conditions',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div');f.elements.set('#saved-view-controls',host);f.c.renderSavedViewControls();assert.equal(host.children[0].attrs['aria-label'],'Work lifecycle');host.children[0].children[1].onclick();assert.equal(f.c.currentSavedViewState().draft.lifecycle,'completed');
 const filter={op:'all',conditions:[]};let changes=0;const builder=f.c.savedViewFilterBuilder(filter,{canEdit:()=>true,onchange:()=>changes++});const buttons=builder.children.find(child=>child.tagName==='details').children[1].children;buttons[1].onclick();assert.equal(filter.conditions[0].op,'all');buttons[0].onclick();assert.equal(filter.conditions[1].field,'status');assert.equal(changes,2);cValidate(f.c,filter);
});
function cValidate(c,filter){assert.doesNotThrow(()=>c.validateSavedViewFilter(filter));}
test('view counts apply eligibility and page visibility; loading errors never broaden a filter',async()=>{
 const f=fixture({responses:[new Error('Invalid views file')]});f.setDoc();const item={source:'github',kind:'Pull request',status:'todo',relations:['authored']};assert.equal(f.c.savedViewCount({...defaults.views[0],query:'absent'},[item]),0);assert.equal(f.c.savedViewCount(defaults.views[0],[item,{...item,hidden:true},{...item,snoozed:true},{...item,archivedAt:1},{...item,status:'done'}]),1);f.c.pageSettingsAllows=()=>false;assert.equal(f.c.savedViewCount(defaults.views[0],[item]),0);
 await f.c.loadSavedViews();assert.equal(f.c.savedViewAllows(item,'github'),false);
});

test('integrated selector counts the same lifecycle, predicates, scope and visibility as rows',()=>{
 const f=fixture();f.setDoc();f.c.scopePrefs={github:'owner/repo'};f.c.matchesSentry=()=>true;f.c.scopeKey=item=>item.project;f.c.source='github';
 const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function workViewAllows('),app.indexOf('function render() {')),f.c);
 f.c.activateSavedView('github-mine');f.c.setSavedViewScope('owner/repo');f.c.scopePrefs.github='owner/repo';
 const item={id:'good',source:'github',kind:'Pull request',status:'unknown',relations:['authored'],project:'owner/repo'};
 const items=[item,{...item,id:'done',status:'done'},{...item,id:'hidden',hidden:true},{...item,id:'snoozed',snoozed:true},{...item,id:'archive',archivedAt:1},{...item,id:'other',project:'other/repo'},{...item,id:'role',relations:['assigned']}];
 assert.equal(f.c.workViewCount(items,'github'),1);assert.deepEqual(items.filter(item=>f.c.workViewAllows(item,'github')).map(item=>item.id),['good']);
 f.c.pageSettingsAllows=()=>false;assert.equal(f.c.workViewCount(items,'github'),0);
});
test('page transitions restore their own query/group and metric filters stay editable data',()=>{
 const f=fixture();f.setDoc();const query=new f.Element('input');f.elements.set('#search',query);f.c.source='github';f.c.prepareSavedViewPage();query.value='API';f.c.savedViewCapturePresentation();f.c.source='linear';f.c.prepareSavedViewPage();assert.equal(query.value,'');query.value='Latency';f.c.savedViewCapturePresentation();f.c.source='github';f.c.prepareSavedViewPage();assert.equal(query.value,'API');
 f.c.setSavedViewStatus('blocked');assert.equal(f.c.savedViewAllows({source:'github',status:'blocked'}),true);assert.equal(f.c.savedViewAllows({source:'github',status:'todo'}),false);assert.equal(f.c.currentSavedViewState().draft.filter.conditions[0].field,'status');f.c.setSavedViewStatus('active');assert.equal(f.c.currentSavedViewState().draft.filter.conditions.length,0);assert.equal(f.c.currentSavedViewState().draft.lifecycle,'active');
 f.c.activateSavedView('github-review');f.c.resetSavedViewState();assert.equal(f.c.currentSavedViewState().id,'');assert.equal(f.c.savedViewAllows({source:'github',kind:'Issue',status:'todo',relations:[]}),true);
});

test('quick repository scopes become saved predicates and activation clears unrelated scopes',()=>{
 const f=fixture();f.setDoc();f.c.scopePrefs={github:'unrelated/repo'};f.c.activateSavedView('github-mine');assert.equal(f.c.scopePrefs.github,'');f.c.setSavedViewScope('owner/repo');const draft=f.c.currentSavedViewState().draft;assert.equal(draft.filter.conditions.at(-1).field,'scope');assert.equal(draft.filter.conditions.at(-1).values[0],'owner/repo');
 f.c.applySavedViewPresentation(draft);assert.equal(f.c.scopePrefs.github,'owner/repo');f.c.setSavedViewScope('');assert.equal(f.c.currentSavedViewState().draft.filter.conditions.some(condition=>condition.field==='scope'),false);
 const migrated=fixture({prefs:{scopes:{linear:'Missing project'}}});assert.equal(migrated.c.currentSavedViewState('linear').draft.filter.conditions[0].values[0],'Missing project');
});

test('page settings exposes filter editing beside provider visibility',async()=>{
 const f=fixture();f.setDoc();let filters=0;f.c.openSavedViewEditor=()=>filters++;
 const settings=fs.readFileSync('static/page-settings.js','utf8').replace('void loadPageSettings();','');vm.runInContext(settings,f.c);await f.c.openPageSettings('github');
 const dialog=f.document.body.children.at(-1),edit=descendants(dialog).find(node=>node.textContent==='Edit this view’s filters'),visibility=descendants(dialog).find(node=>node.textContent==='Choose visible repositories');assert.ok(edit);assert.ok(visibility);edit.onclick();assert.equal(filters,1);
});

test('Sentry saved badges and activated rows use the same legacy filter regardless of current page',()=>{
 const f=fixture();f.setDoc();f.c.sentryFilters={level:'error'};f.c.environmentMembers=new Set();f.c.scopePrefs={sentry:''};f.c.scopeKey=item=>item.project||'';
 const features=fs.readFileSync('static/features.js','utf8');vm.runInContext(features.slice(features.indexOf('function matchesSentry('),features.indexOf('let environmentRequest=')),f.c);const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function workViewAllows('),app.indexOf('const renderHooks=')),f.c);
 const view={id:'sentry-errors',name:'Errors',parent:'sentry',lifecycle:'active',filter:{op:'all',conditions:[]},query:'',groupBy:'status'},items=[{id:'error',source:'sentry',status:'todo',level:'error'},{id:'warning',source:'sentry',status:'todo',level:'warning'}];f.setDoc({version:1,revision:0,contentHash:'a'.repeat(64),views:[view]});f.c.activateSavedView(view.id);assert.equal(f.c.savedViewCount(view,items),1);assert.equal(f.c.workViewCount(items,'sentry'),1);f.c.source='github';assert.equal(f.c.savedViewCount(view,items),1);assert.equal(f.c.workViewCount(items,'sentry'),1);
});
test('Reload adopts changed saved definitions for clean snapshots and preserves visibly stale drafts',async()=>{
 const initial={...defaults,contentHash:'a'.repeat(64)},changed=structuredClone(initial);changed.revision=1;changed.contentHash='b'.repeat(64);changed.views[0].filter={op:'in',field:'status',values:['blocked']};
 for(const dirty of [false,true]){
  const f=fixture({responses:[changed]});f.setDoc(initial);f.c.activateSavedView('github-mine');if(dirty)f.c.currentSavedViewState().draft.query='local query';await f.c.loadSavedViews();const state=f.c.currentSavedViewState();
  if(dirty){assert.equal(state.draft.query,'local query');assert.equal(state.draft.filter.op,'all');assert.equal(state.stale,'changed');const host=new f.Element('div');f.elements.set('#saved-view-controls',host);f.c.renderSavedViewControls();assert.ok(host.children.some(node=>node.attrs.role==='status'&&node.textContent.includes('Saved view changed')));f.c.activateSavedView('github-mine');assert.equal(f.c.currentSavedViewState().stale,undefined);}
  assert.equal(f.c.currentSavedViewState().draft.filter.op,'in');assert.equal(f.c.currentSavedViewState().draft.filter.values[0],'blocked');
 }
});
test('shared UI search covers provider status, relationships and effective lifecycle without fuzzy descriptions',()=>{
 const f=fixture(),item={source:'github',title:'API',reference:'#1',project:'repo',sourceStatus:'Awaiting approval',relations:['reviews'],detail:''};assert.equal(f.c.savedViewSearchAllows('approval',item),true);assert.equal(f.c.savedViewSearchAllows('reviews',item),true);assert.equal(f.c.savedViewSearchAllows('reviews',{...item,sourceStatus:'',relations:[],detail:'red elephants visit idle elephants west slowly'}),false);assert.equal(f.c.savedViewMatches({parent:'github',lifecycle:'active',filter:{op:'all',conditions:[]}},{...item,status:'done'}),false);
});

function descendants(node){return [node,...(node.children||[]).flatMap(descendants)];}
function control(node,label){const found=descendants(node).find(child=>child.attrs?.['aria-label']===label);assert.ok(found,'Missing control: '+label);return found;}
test('compact rule rows keep exact values in removable chips and pending text is applied without Enter',()=>{
 const {c}=fixture(),filter={op:'in',field:'scope',values:['org/repo','Project, with comma']};let changes=0;
 const row=c.savedViewFilterBuilder(filter,{canEdit:()=>true,onchange:()=>changes++});assert.equal(descendants(row).some(node=>node.tagName==='textarea'),false);
 const entry=control(row,'Add Project / repository value');entry.value='new/repo';entry.oninput();assert.deepEqual([...filter.values],['org/repo','Project, with comma','new/repo']);
 entry.onkeydown({key:'Enter',isComposing:true,preventDefault(){throw Error('IME must not commit');}});assert.equal(entry.value,'new/repo');
 entry.onkeydown({key:'Enter',preventDefault(){}});assert.equal(entry.value,'');assert.ok(control(row,'Remove new/repo'));
 control(row,'Remove org/repo').onclick();assert.deepEqual([...filter.values],['Project, with comma','new/repo']);assert.ok(changes>=3);
});
test('known-value chips deduplicate choices and field changes reset only the edited rule',()=>{
 const {c}=fixture(),filter={op:'in',field:'status',values:['todo']};const row=c.savedViewFilterBuilder(filter,{canEdit:()=>true});const select=control(row,'Choose Status value');select.value='blocked';select.onchange();select.value='blocked';select.onchange();assert.deepEqual([...filter.values],['todo','blocked']);
 const field=control(row,'Filter field');field.value='pinned';field.onchange();assert.deepEqual(filter,{op:'equals',field:'pinned',value:true});assert.equal(control(row,'Pinned value').value,'true');
});
test('group logic updates visible connectors and nested removal preserves sibling rules',()=>{
 const {c}=fixture(),filter={op:'all',conditions:[{op:'in',field:'status',values:['todo']},{op:'any',conditions:[{op:'equals',field:'pinned',value:true}]}]};
 const builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true}),logic=control(builder,'Group logic');assert.ok(descendants(builder).some(node=>node.className==='saved-view-join'&&node.textContent==='AND'));
 logic.value='any';logic.onchange();assert.equal(filter.op,'any');assert.ok(descendants(builder).some(node=>node.className==='saved-view-join'&&node.textContent==='OR'));
 control(builder,'Remove condition group').onclick();assert.equal(filter.conditions.length,1);assert.equal(filter.conditions[0].field,'status');
});
test('date filters use strict calendars and timestamps with deterministic rolling and UTC boundaries',()=>{
 const {c}=fixture(),now=Date.parse('2026-10-27T12:00:00Z')/1000,cutoff=now-7*86400;
 for(const field of ['createdAt','updatedAt']){
  const within={op:'withinLast',field,days:7},older={op:'olderThan',field,days:7};c.validateSavedViewFilter(within);
  for(const [timestamp,expected]of [[cutoff,true],[cutoff-0.001,false],[now,true],[now+1,false]])assert.equal(c.matchesSavedViewFilter(within,{[field]:timestamp},now),expected);
  assert.equal(c.matchesSavedViewFilter(older,{[field]:cutoff},now),false);assert.equal(c.matchesSavedViewFilter(older,{[field]:cutoff-0.001},now),true);
  const before={op:'beforeDate',field,date:'2026-10-27'},after={op:'onOrAfterDate',field,date:'2026-10-27'};c.validateSavedViewFilter(before);const midnight=Date.parse('2026-10-27T00:00:00Z')/1000;
  assert.equal(c.matchesSavedViewFilter(before,{[field]:midnight},now),false);assert.equal(c.matchesSavedViewFilter(after,{[field]:midnight},now),true);assert.equal(c.matchesSavedViewFilter(after,{[field]:'2026-10-26T20:00:00-04:00'},now),true);assert.equal(c.matchesSavedViewFilter(after,{[field]:'2026-10-27 00:00:00Z'},now),true);
  for(const timestamp of [undefined,null,true,NaN,Infinity,'2026-10-27','2026-10-27T00:00:00','2026-02-30T00:00:00Z','2026-10-27T24:00:00Z','2026-10-27T00:00:60Z'])assert.equal(c.matchesSavedViewFilter(after,{[field]:timestamp},now),false,String(timestamp));
 }
 for(const date of ['2026-02-29','2026-02-30','2026-13-01','2026-1-01','2026-10-27T00:00:00Z'])assert.throws(()=>c.validateSavedViewFilter({op:'beforeDate',field:'updatedAt',date}),/valid UTC/);
 for(const date of ['2024-02-29','0000-02-29'])assert.doesNotThrow(()=>c.validateSavedViewFilter({op:'beforeDate',field:'updatedAt',date}));
 for(const days of [0,3651,1.5,'7',null])assert.throws(()=>c.validateSavedViewFilter({op:'withinLast',field:'updatedAt',days}),/valid UTC/);
 assert.throws(()=>c.validateSavedViewFilter({op:'withinLast',field:'status',days:7}),/valid UTC/);
 const nested={op:'all',conditions:[{op:'withinLast',field:'updatedAt',days:7},{op:'any',conditions:[{op:'olderThan',field:'createdAt',days:7}]}]};assert.equal(c.matchesSavedViewFilter(nested,{updatedAt:cutoff,createdAt:cutoff-1},now),true);
});
test('inline date editor switches between rolling days and explicitly UTC dates without stale payload fields',()=>{
 const {c}=fixture(),filter={op:'withinLast',field:'updatedAt',days:7},row=c.savedViewFilterBuilder(filter,{canEdit:()=>true});const days=control(row,'Number of rolling days');days.value='14';days.oninput();assert.equal(filter.days,14);
 const op=control(row,'Date operator');op.value='beforeDate';op.onchange();assert.equal(filter.days,undefined);const date=control(row,'Date in UTC');assert.equal(date.type,'date');date.value='2026-02-30';date.oninput();assert.throws(()=>c.validateSavedViewFilter(filter));date.value='2026-10-27';date.oninput();c.validateSavedViewFilter(filter);assert.match(c.savedViewSummary(filter),/2026-10-27 \(UTC\)/);
 const again=control(row,'Date operator');again.value='olderThan';again.onchange();assert.equal(filter.date,undefined);assert.equal(filter.days,7);assert.match(c.savedViewSummary(filter),/7 days ago/);
});

test('filter edits stay local until Apply and save naming unfolds only when needed',async()=>{
 const f=fixture();f.setDoc();const original=JSON.stringify(f.c.currentSavedViewState().draft);f.c.openSavedViewEditor();const dialog=f.document.body.children.at(-1),options=descendants(dialog).find(node=>node.className==='saved-view-options');assert.equal(options.open,false);
 const menu=descendants(dialog).find(node=>node.className==='saved-view-add-options');menu.children[0].onclick();assert.equal(JSON.stringify(f.c.currentSavedViewState().draft),original);
 await descendants(dialog).find(node=>node.textContent==='Save view').onclick();assert.equal(options.open,true);assert.equal(control(dialog,'View name').focused,true);assert.equal(f.calls.length,0);
 descendants(dialog).find(node=>node.textContent==='Apply filters').onclick();assert.equal(f.c.currentSavedViewState().draft.filter.conditions[0].field,'status');assert.equal(dialog.open,false);assert.equal(f.calls.length,0);
});

function editorButton(dialog,label){const button=descendants(dialog).find(node=>node.tagName==='button'&&node.textContent===label);assert.ok(button,'Missing '+label);return button;}
function editorFixture(responses=[]){const f=fixture({responses});f.setDoc();f.c.activateSavedView('github-mine');f.c.currentSavedViewState().draft.query='local draft query';f.c.openSavedViewEditor();return {...f,dialog:f.document.body.children.at(-1)};}
test('delete rejects missing saved IDs and is absent for unsaved filters',async()=>{
 const f=fixture();f.setDoc();await assert.rejects(f.c.deleteSavedViewDefinition('missing',{...defaults,contentHash:'a'.repeat(64)}),/unavailable/);assert.equal(f.calls.length,0);
 f.c.openSavedViewEditor();assert.equal(descendants(f.document.body.children.at(-1)).some(node=>node.textContent==='Delete view'),false);
});
test('delete confirmation names the view and cancellation keeps the editor draft',()=>{
 const f=editorFixture(),before=JSON.stringify(f.c.currentSavedViewState().draft),confirmation=descendants(f.dialog).find(node=>node.className==='saved-view-delete-confirmation');
 assert.equal(confirmation.hidden,true);editorButton(f.dialog,'Delete view').onclick();assert.equal(confirmation.hidden,false);assert.match(confirmation.children[0].textContent,/Delete “Mine”/);
 editorButton(f.dialog,'Keep view').onclick();assert.equal(confirmation.hidden,true);assert.equal(f.dialog.open,true);assert.equal(JSON.stringify(f.c.currentSavedViewState().draft),before);assert.equal(f.calls.length,0);
});
test('deletion uses editor-open CAS baseline and stale failure retains editable draft',async()=>{
 const f=editorFixture([new Error('Saved views changed. Reload the views before saving again.')]);
 const changed={...structuredClone(defaults),revision:9,contentHash:'b'.repeat(64)};f.setDoc(changed);
 const field=control(f.dialog,'Filter field');field.value='status';field.onchange();control(f.dialog,'View name').value='Unsaved name';
 editorButton(f.dialog,'Delete view').onclick();await editorButton(f.dialog,'Confirm delete').onclick();
 assert.equal(f.calls[0].payload.revision,0);assert.equal(f.calls[0].payload.contentHash,'a'.repeat(64));assert.equal(f.dialog.open,true);assert.equal(f.dialog.inert,false);assert.equal(f.c.currentSavedViewState().id,'github-mine');assert.match(descendants(f.dialog).find(node=>node.attrs?.role==='alert').textContent,/changed/);
 editorButton(f.dialog,'Keep view').onclick();assert.equal(control(f.dialog,'View name').value,'Unsaved name');assert.equal(control(f.dialog,'Filter field').value,'status');
 assert.equal(f.c.currentSavedViewState().draft.query,'local draft query');
});
test('confirmed deletion keeps current filters unsaved and leaves other page selections unchanged',async()=>{
 const next={...structuredClone(defaults),revision:1,contentHash:'b'.repeat(64)};next.views=next.views.filter(view=>view.id!=='github-mine');
 const f=editorFixture([next]);const linear=f.c.currentSavedViewState('linear');linear.draft.query='other page';const before=JSON.stringify(linear),draft=JSON.stringify(f.c.currentSavedViewState().draft);
 editorButton(f.dialog,'Delete view').onclick();await editorButton(f.dialog,'Confirm delete').onclick();
 const current=f.c.currentSavedViewState();assert.equal(current.id,'');assert.equal(JSON.stringify(current.draft),draft);assert.equal(current.base,undefined);assert.equal(JSON.stringify(f.c.currentSavedViewState('linear')),before);assert.equal(f.dialog.open,false);assert.equal(f.calls.length,1);assert.equal(f.calls[0].payload.views.some(view=>view.id==='github-mine'),false);
});
test('pending deletion ignores duplicate confirmation',async()=>{
 const f=editorFixture();let finish;f.c.api=async()=>{f.calls.push('delete');return new Promise(resolve=>finish=resolve);};
 const button=editorButton(f.dialog,'Confirm delete');const first=button.onclick();await button.onclick();assert.equal(f.calls.length,1);assert.equal(f.dialog.inert,false);assert.equal(f.dialog.attrs['aria-busy'],'true');assert.ok(f.dialog.querySelectorAll('input,select,button').every(control=>control.disabled));
 const next={...structuredClone(defaults),revision:1,contentHash:'b'.repeat(64),views:defaults.views.filter(view=>view.id!=='github-mine')};finish(next);await first;assert.equal(f.dialog.open,false);
});

test('Yesterday uses named-zone calendar boundaries across spring and fall DST changes',()=>{
 const {c}=fixture(),seconds=text=>Date.parse(text)/1000;
 for(const field of ['updatedAt','createdAt'])for(const [now,start,end]of [
  ['2026-03-30T00:30:00Z','2026-03-28T23:00:00Z','2026-03-29T22:00:00Z'],
  ['2026-10-26T00:30:00Z','2026-10-24T22:00:00Z','2026-10-25T23:00:00Z']
 ]){
  const filter={op:'calendarPeriod',field,period:'yesterday',timezone:'Europe/Berlin'};c.validateSavedViewFilter(filter);
  for(const [at,expected]of [[seconds(start)-.001,false],[seconds(start),true],[seconds(end)-.001,true],[seconds(end),false]])assert.equal(c.matchesSavedViewFilter(filter,{[field]:at},seconds(now)),expected,field+' '+now+' '+at);
 }
 const filter={op:'calendarPeriod',field:'updatedAt',period:'yesterday',timezone:'Europe/Berlin'},now=seconds('2026-03-30T00:30:00Z'),item={updatedAt:seconds('2026-03-28T23:00:00Z')};assert.equal(c.matchesSavedViewFilter(filter,item,now),true);assert.equal(c.matchesSavedViewFilter({op:'withinLast',field:'updatedAt',days:1},item,now),false);
 for(const at of ['2026-10-25T00:30:00Z','2026-10-25T01:30:00Z'])assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:at},seconds('2026-10-26T00:30:00Z')),true);
});
test('calendar weeks start Monday in the selected zone and Today does not assume UTC',()=>{
 const {c}=fixture(),now=Date.parse('2026-10-25T12:00:00Z')/1000,filter={op:'calendarPeriod',field:'updatedAt',period:'thisWeek',timezone:'Europe/Berlin'};
 assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:'2026-10-18T22:00:00Z'},now),true);assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:'2026-10-18T21:59:59Z'},now),false);assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:'2026-10-25T23:00:00Z'},now),false);
 filter.period='lastWeek';assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:'2026-10-11T22:00:00Z'},now),true);assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:'2026-10-18T22:00:00Z'},now),false);
 filter.period='today';filter.timezone='America/Los_Angeles';const dayNow=Date.parse('2026-10-27T00:30:00Z')/1000,item={updatedAt:'2026-10-26T08:00:00Z'};assert.equal(c.matchesSavedViewFilter(filter,item,dayNow),true);assert.equal(c.matchesSavedViewFilter({...filter,timezone:'UTC'},item,dayNow),false);
 for(const timestamp of [undefined,null,true,NaN,Infinity,'2026-02-30T00:00:00Z'])assert.equal(c.matchesSavedViewFilter(filter,{updatedAt:timestamp},dayNow),false);
});
test('calendar schema rejects invalid zones, periods and stale fields and preserves nested evaluation now',()=>{
 const {c}=fixture(),base={op:'calendarPeriod',field:'updatedAt',period:'today',timezone:'UTC'};
 for(const patch of [{period:'rolling24h'},{timezone:'Mars/Olympus'},{timezone:'Europe/Berlin\0'},{timezone:'x'.repeat(101)},{timezone:'+05:00'},{timezone:null},{field:'status'},{days:1},{date:'2026-10-27'}])assert.throws(()=>c.validateSavedViewFilter({...base,...patch}),/calendar period/);
 const now=Date.parse('2026-10-27T12:00:00Z')/1000;assert.equal(c.matchesSavedViewFilter({op:'all',conditions:[{op:'any',conditions:[base]}]},{updatedAt:'2026-10-27T00:00:00Z'},now),true);
 assert.match(c.savedViewSummary({...base,period:'yesterday',timezone:'Europe/Berlin'}),/Yesterday.*Europe\/Berlin/);
});
test('During exposes calendar periods and an editable retained timezone; switching operators removes stale fields',()=>{
 const {c}=fixture(),filter={op:'withinLast',field:'updatedAt',days:7},row=c.savedViewFilterBuilder(filter,{canEdit:()=>true}),op=control(row,'Date operator');op.value='calendarPeriod';op.onchange();assert.equal(filter.days,undefined);assert.equal(filter.period,'yesterday');assert.ok(c.savedViewCalendarFormatter(filter.timezone));
 const period=control(row,'Calendar period');period.value='lastWeek';period.onchange();const zone=control(row,'Calendar time zone (IANA)');zone.value='America/New_York';zone.oninput();assert.equal(filter.timezone,'America/New_York');c.validateSavedViewFilter(filter);
 const rebuilt=c.savedViewFilterBuilder(filter,{canEdit:()=>true});assert.equal(control(rebuilt,'Calendar time zone (IANA)').value,'America/New_York');assert.equal(control(rebuilt,'Calendar period').value,'lastWeek');
 const again=control(row,'Date operator');again.value='beforeDate';again.onchange();assert.equal(filter.period,undefined);assert.equal(filter.timezone,undefined);assert.ok(filter.date);c.validateSavedViewFilter(filter);
});
test('Since yesterday includes both calendar days while Yesterday excludes today',()=>{
 const {c}=fixture(),filter={op:'calendarPeriod',field:'createdAt',period:'sinceYesterday',timezone:'Europe/Berlin'},now=Date.parse('2026-03-30T12:00:00Z')/1000;c.validateSavedViewFilter(filter);
 for(const [at,expected]of [['2026-03-28T22:59:59Z',false],['2026-03-28T23:00:00Z',true],['2026-03-29T22:00:00Z',true],['2026-03-30T21:59:59Z',true],['2026-03-30T22:00:00Z',false]])assert.equal(c.matchesSavedViewFilter(filter,{createdAt:at},now),expected,at);
 assert.equal(c.matchesSavedViewFilter({...filter,period:'yesterday'},{createdAt:'2026-03-30T10:00:00Z'},now),false);assert.match(c.savedViewSummary(filter),/Since yesterday/);
});

test('editor Save and Save as new keep editor-open CAS even after background document refresh',async()=>{
 for(const action of ['Save changes','Save as new view']){
  const f=editorFixture([new Error('Saved views changed. Reload the views before saving again.')]),fresh={...structuredClone(defaults),revision:9,contentHash:'b'.repeat(64)};fresh.views[0].query='remote query';fresh.views.push({...fresh.views[1],id:'remote-extra',name:'Remote extra'});f.setDoc(fresh);
  await editorButton(f.dialog,action).onclick();assert.equal(f.calls.length,1);assert.equal(f.calls[0].payload.revision,0);assert.equal(f.calls[0].payload.contentHash,'a'.repeat(64));assert.equal(f.calls[0].payload.views.some(view=>view.id==='remote-extra'),false);assert.equal(f.dialog.open,true);assert.match(descendants(f.dialog).find(node=>node.attrs?.role==='alert').textContent,/Your draft is kept/);
 }
});
test('explicit Reload adopts a fresh CAS baseline only when the edited saved definition is unchanged',async()=>{
 const fresh={...structuredClone(defaults),revision:4,contentHash:'b'.repeat(64)};fresh.views.push({...fresh.views[1],id:'remote-extra',name:'Remote extra'});const saved=structuredClone(fresh);saved.revision=5;saved.contentHash='c'.repeat(64);saved.views[0].query='local draft query';
 const f=editorFixture([fresh,saved]);await editorButton(f.dialog,'Reload views').onclick();await editorButton(f.dialog,'Save changes').onclick();assert.equal(f.calls[1].payload.revision,4);assert.equal(f.calls[1].payload.contentHash,'b'.repeat(64));assert.equal(f.calls[1].payload.views.some(view=>view.id==='remote-extra'),true);assert.equal(f.calls[1].payload.views.find(view=>view.id==='github-mine').query,'local draft query');assert.equal(f.dialog.open,false);
});
test('Reload preserves conflicting or removed editor drafts and permits only explicit fresh-baseline copy',async()=>{
 for(const change of ['definition','name','removed']){
  const fresh={...structuredClone(defaults),revision:7,contentHash:'b'.repeat(64)};if(change==='removed')fresh.views=fresh.views.filter(view=>view.id!=='github-mine');else if(change==='name')fresh.views[0].name='Renamed remotely';else fresh.views[0].query='remote query';
  const f=editorFixture([fresh]);editorButton(f.dialog,'Delete view').onclick();await editorButton(f.dialog,'Reload views').onclick();const confirmation=descendants(f.dialog).find(node=>node.className==='saved-view-delete-confirmation');assert.equal(confirmation.hidden,true);
  await editorButton(f.dialog,'Save changes').onclick();assert.equal(f.calls.length,1);assert.equal(f.dialog.open,true);editorButton(f.dialog,'Delete view').onclick();assert.equal(confirmation.hidden,true);assert.match(descendants(f.dialog).find(node=>node.attrs?.role==='alert').textContent,/Close and reopen/);
  const capturedFresh=structuredClone(fresh);const background=structuredClone(fresh);background.revision=10;background.contentHash='d'.repeat(64);f.setDoc(background);
  let request;f.c.api=async(path,payload)=>{request=payload;return {...capturedFresh,revision:8,contentHash:'c'.repeat(64),views:payload.views};};await editorButton(f.dialog,'Save as new view').onclick();assert.equal(request.revision,7);assert.equal(request.contentHash,'b'.repeat(64));const copy=request.views.find(view=>view.id.startsWith('view-'));assert.equal(copy.query,'local draft query');assert.equal(copy.name,'Mine');assert.deepEqual(JSON.parse(JSON.stringify(request.views.filter(view=>!view.id.startsWith('view-')))),capturedFresh.views);assert.equal(f.dialog.open,false);
 }
});

test('filter badge counts direct agent predicates and nested rules, including a match-none group',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div');f.elements.set('#saved-view-controls',host);
 const date={op:'calendarPeriod',field:'updatedAt',period:'sinceYesterday',timezone:'Europe/Berlin'};
 const cases=[[date,1],[{op:'all',conditions:[]},0],[{op:'any',conditions:[]},1],[{op:'all',conditions:[date,{op:'any',conditions:[predicate('in','source',['github']),predicate('in','source',['linear'])]}]},3]];
 for(const [filter,count]of cases){f.c.currentSavedViewState().draft.filter=filter;f.c.renderSavedViewControls();assert.equal(host.children[1].textContent,'Filters'+(count?' · '+count:''));assert.equal(host.children[1].title,f.c.savedViewSummary(filter));}
});

test('background view refresh preserves draft and document when a dialog opens during the read',async()=>{
 const f=fixture();f.setDoc();f.c.activateSavedView('github-mine');f.c.currentSavedViewState().draft.query='Unsaved query';const before=f.state();let release,allow=true;const gate=new Promise(resolve=>release=resolve);
 const changed=structuredClone(defaults);changed.revision=2;changed.contentHash='b'.repeat(64);changed.views[0].name='External name';
 f.c.api=async()=>{await gate;return changed;};const pending=f.c.loadSavedViews({canApply:()=>allow});allow=false;release();assert.equal(await pending,null);assert.deepEqual(f.state(),before);assert.equal(vm.runInContext('savedViewsDocument.revision',f.c),0);
});

test('focused filter fields and date operators retain focus across their own redraws',()=>{
 const {c,document}=fixture(),row=c.savedViewFilterBuilder({op:'in',field:'status',values:['todo']},{canEdit:()=>true});
 const field=control(row,'Filter field');field.focus();field.value='updatedAt';field.onchange();
 assert.notEqual(control(row,'Filter field'),field);assert.equal(document.activeElement,control(row,'Filter field'));
 const operator=control(row,'Date operator');operator.focus();operator.value='calendarPeriod';operator.onchange();
 assert.notEqual(control(row,'Date operator'),operator);assert.equal(document.activeElement,control(row,'Date operator'));
 const calendarOperator=control(row,'Date operator');calendarOperator.value='beforeDate';calendarOperator.onchange();assert.equal(document.activeElement,control(row,'Date operator'));
});
test('initial filter rendering and non-focused programmatic field changes never steal focus',()=>{
 const {c,document,Element}=fixture(),outside=new Element('input');outside.focus();
 const row=c.savedViewFilterBuilder({op:'withinLast',field:'updatedAt',days:7},{canEdit:()=>true});assert.equal(document.activeElement,outside);
 const operator=control(row,'Date operator');operator.value='calendarPeriod';operator.onchange();assert.equal(document.activeElement,outside);
 const field=control(row,'Filter field');field.value='status';field.onchange();assert.equal(document.activeElement,outside);
});
test('removing focused rules chooses the next rule, previous rule, then Add rule',()=>{
 const {c,document}=fixture(),filter={op:'all',conditions:[predicate('in','status',['todo']),predicate('in','source',['github']),predicate('in','kind',['Issue'])]},builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true});
 control(builder,'Remove Source rule').focus();control(builder,'Remove Source rule').onclick();
 assert.equal(document.activeElement,descendants(builder).filter(node=>node.attrs?.['aria-label']==='Filter field')[1]);assert.equal(document.activeElement.value,'kind');
 control(builder,'Remove Kind rule').focus();control(builder,'Remove Kind rule').onclick();assert.equal(document.activeElement,control(builder,'Filter field'));assert.equal(document.activeElement.value,'status');
 control(builder,'Remove Status rule').focus();control(builder,'Remove Status rule').onclick();assert.equal(document.activeElement,builder.querySelector('summary'));assert.equal(filter.conditions.length,0);
});
test('nested group removal restores adjacent focus and unrelated programmatic removal preserves its current control',()=>{
 const {c,document,Element}=fixture(),filter={op:'all',conditions:[{op:'all',conditions:[predicate('in','status',['todo'])]},predicate('in','source',['github'])]},builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true});
 const removeGroup=control(builder,'Remove condition group'),sourceField=descendants(builder).filter(node=>node.attrs?.['aria-label']==='Filter field')[1];sourceField.focus();removeGroup.onclick();assert.equal(document.activeElement,control(builder,'Filter field'));assert.equal(document.activeElement.value,'source');
 const outside=new Element('button');outside.focus();control(builder,'Remove Source rule').onclick();assert.equal(document.activeElement,outside);
 const nested=c.savedViewFilterBuilder({op:'all',conditions:[{op:'any',conditions:[predicate('in','status',['todo'])]},predicate('in','source',['github'])]},{canEdit:()=>true});control(nested,'Remove condition group').focus();control(nested,'Remove condition group').onclick();assert.equal(document.activeElement,control(nested,'Filter field'));
});

test('changing an Updated field to Source refreshes its parent remove label without losing field focus',()=>{
 const {c,document}=fixture(),filter={op:'all',conditions:[{op:'withinLast',field:'updatedAt',days:7}]};let changes=0;
 const builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true,onchange:()=>changes++}),remove=control(builder,'Remove Updated rule'),field=control(builder,'Filter field');
 field.focus();field.value='source';field.onchange();
 assert.equal(control(builder,'Remove Source rule'),remove);assert.equal(descendants(builder).some(node=>node.attrs?.['aria-label']==='Remove Updated rule'),false);
 assert.equal(document.activeElement,control(builder,'Filter field'));assert.equal(document.activeElement.value,'source');assert.equal(changes,1);assert.equal(control(builder,'Group logic').value,'all');
});

test('adding rules and groups from the focused Add menu focuses their new first editor control',()=>{
 for(const group of [false,true]){
  const {c,document}=fixture(),filter={op:'all',conditions:[]},builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true}),menu=builder.querySelector('details'),actions=menu.children[1],add=actions.children[group?1:0];
  menu.open=true;add.focus();add.onclick();assert.equal(menu.open,false);assert.equal(filter.conditions.length,1);
  const added=builder.children[1].children[0];assert.equal(document.activeElement,control(added,group?'Group logic':'Filter field'));assert.equal(document.activeElement.value,group?'all':'status');
 }
});
test('programmatic rule addition preserves outside or existing rule focus without jumping to the added editor',()=>{
 for(const outside of [false,true]){
  const {c,document,Element}=fixture(),filter={op:'all',conditions:[predicate('in','source',['github'])]},builder=c.savedViewFilterBuilder(filter,{canEdit:()=>true}),previous=outside?new Element('input'):control(builder,'Filter field');
  previous.focus();builder.querySelector('details').children[1].children[0].onclick();
  assert.equal(filter.conditions.length,2);assert.equal(document.activeElement,outside?previous:control(builder,'Filter field'));if(!outside)assert.equal(document.activeElement.value,'source');
 }
});

test('lifecycle selection preserves semantic keyboard focus through the actual controls redraw',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div');f.elements.set('#saved-view-controls',host);f.c.render=()=>f.c.renderSavedViewControls();f.c.renderSavedViewControls();
 const lifecycle=host.children[0];assert.equal(lifecycle.attrs.role,'group');assert.match(lifecycle.className,/ui-segmented/);
 const completed=lifecycle.children[1];assert.equal(completed.dataset.variant,'secondary');completed.focus();completed.onclick();
 const next=host.children[0].children[1];assert.notEqual(next,completed);assert.equal(f.document.activeElement,next);assert.equal(next.focusOptions?.preventScroll,true);assert.equal(next.attrs['aria-pressed'],'true');assert.equal(f.c.currentSavedViewState().draft.lifecycle,'completed');
 next.onclick();assert.equal(f.document.activeElement,host.children[0].children[1]);
});
test('controls redraw leaves external modal focus untouched and restores removed actions to the selected lifecycle',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div'),modalField=new f.Element('input');f.elements.set('#saved-view-controls',host);f.c.render=()=>f.c.renderSavedViewControls();modalField.focus();f.c.renderSavedViewControls();assert.equal(f.document.activeElement,modalField);
 host.children[0].children[2].onclick();assert.equal(f.document.activeElement,modalField);
 f.c.activateSavedView('github-mine');f.c.currentSavedViewState().draft.query='Unsaved query';f.c.renderSavedViewControls();const reset=host.children.find(button=>button.dataset.savedViewControl==='reset');reset.focus();f.c.currentSavedViewState().id='';f.c.renderSavedViewControls();assert.equal(f.document.activeElement,host.children[0].children[0]);
 const filters=host.children.find(button=>button.dataset.savedViewControl==='filters');filters.focus();f.c.renderSavedViewControls();assert.equal(f.document.activeElement,host.children.find(button=>button.dataset.savedViewControl==='filters'));
});
test('saved view action variants follow explicit intent when all displayed button labels change',()=>{
 const f=fixture();f.setDoc();f.c.activateSavedView('github-mine');const button=f.c.savedViewButton;f.c.savedViewButton=(label,...args)=>button('Translated '+label,...args);f.c.openSavedViewEditor();const dialog=f.document.body.children.at(-1);
 for(const [label,variant]of [['Delete view','destructive'],['Confirm delete','destructive'],['Save changes','primary'],['Keep view','secondary'],['Reload views','secondary'],['Apply filters','secondary']]){
  const action=editorButton(dialog,'Translated '+label);assert.match(action.className,/ui-control/);assert.equal(action.dataset.variant,variant,label);
 }
 editorButton(dialog,'Translated Delete view').onclick();assert.equal(descendants(dialog).find(node=>node.className==='saved-view-delete-confirmation').hidden,false);
});
function sidebarFixture(){const f=fixture();f.setDoc();const parent=new f.Element('button'),host=new f.Element('div');f.document.body.append(parent,host);f.elements.set('[data-source="github"]',parent);f.elements.set('#github-subnav',host);f.c.render=()=>f.c.renderSavedViewSidebar();f.c.renderSavedViewSidebar();return {...f,host};}
test('saved-view sidebar keyboard activation and later redraw retain the exact visible view without scrolling',()=>{
 const f=sidebarFixture(),mine=f.host.children.find(button=>button.dataset.savedViewId==='github-mine');mine.focus();mine.onclick();let replacement=f.host.children.find(button=>button.dataset.savedViewId==='github-mine');assert.notEqual(replacement,mine);assert.equal(f.document.activeElement,replacement);assert.equal(replacement.focusOptions.preventScroll,true);assert.equal(replacement.attrs['aria-pressed'],'true');f.c.renderSavedViewSidebar();assert.notEqual(f.document.activeElement,replacement);assert.equal(f.document.activeElement.dataset.savedViewId,'github-mine');
});
test('sidebar redraw preserves external search/modal focus and never falls back after deletion or source switch',()=>{
 const f=sidebarFixture(),outside=new f.Element('input');f.document.body.append(outside);outside.focus();f.c.renderSavedViewSidebar();assert.equal(f.document.activeElement,outside);const modal=new f.Element('dialog');modal.open=true;f.document.body.append(modal);f.elements.set('dialog[open]',modal);const field=new f.Element('input');modal.append(field);field.focus();f.c.renderSavedViewSidebar();assert.equal(f.document.activeElement,field);f.elements.delete('dialog[open]');
 f.host.children.find(button=>button.dataset.savedViewId==='github-mine').focus();f.setDoc({...defaults,views:defaults.views.filter(view=>view.id!=='github-mine'),contentHash:'a'.repeat(64)});f.c.renderSavedViewSidebar();assert.equal(f.document.activeElement,f.document.body);
 f.host.children[0].focus();f.c.source='linear';f.c.renderSavedViewSidebar();assert.equal(f.host.hidden,true);assert.equal(f.document.activeElement,f.document.body);
});
test('sidebar focus does not move into hidden, disabled, detached, or obscured replacement groups',()=>{
 for(const mode of ['hidden','disabled','detached','modal']){const f=sidebarFixture();f.host.children[0].focus();if(mode==='hidden')f.host.visibility='hidden';if(mode==='disabled'){const button=f.c.savedViewButton;f.c.savedViewButton=(...args)=>Object.assign(button(...args),{disabled:true});}if(mode==='detached'){f.host.parentElement=null;}if(mode==='modal')f.elements.set('dialog[open]',new f.Element('dialog'));f.c.renderSavedViewSidebar();assert.equal(f.document.activeElement,f.document.body);}
});


test('pristine saved-view toolbar offers Edit view without claiming changes or offering Reset',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div');f.elements.set('#saved-view-controls',host);f.c.render=()=>f.c.renderSavedViewControls();f.c.activateSavedView('github-mine');
 const state=f.c.currentSavedViewState();assert.equal(f.c.sameSavedViewDefinition(state.draft,state.base),true);
 const edit=host.children.find(button=>button.dataset.savedViewControl==='save');assert.equal(edit.textContent,'Edit view');assert.equal(host.children.some(button=>button.dataset.savedViewControl==='reset'),false);assert.equal(host.children.some(node=>node.textContent==='Modified · unsaved filters'),false);
 edit.onclick();const dialog=f.document.body.children.at(-1);assert.equal(dialog.open,true);assert.equal(editorButton(dialog,'Save changes').textContent,'Save changes');assert.equal(f.calls.length,0);
});

test('modified saved-view Reset restores exact saved definition and removes itself while stale views retain recovery',()=>{
 const f=fixture();f.setDoc();const host=new f.Element('div');f.elements.set('#saved-view-controls',host);f.c.render=()=>f.c.renderSavedViewControls();f.c.activateSavedView('github-mine');const baseline=JSON.parse(JSON.stringify(f.c.currentSavedViewState().base));
 f.c.currentSavedViewState().draft.query='Different query';f.c.currentSavedViewState().draft.groupBy='none';f.c.renderSavedViewControls();const reset=host.children.find(button=>button.dataset.savedViewControl==='reset');assert.ok(reset);assert.equal(host.children.some(node=>node.textContent==='Modified · unsaved filters'),true);reset.focus();reset.onclick();
 assert.deepEqual(JSON.parse(JSON.stringify(f.c.currentSavedViewState().draft)),baseline);assert.equal(host.children.some(button=>button.dataset.savedViewControl==='reset'),false);assert.equal(host.children.some(node=>node.textContent==='Modified · unsaved filters'),false);assert.equal(f.document.activeElement,host.children[0].children.find(button=>button.attrs['aria-pressed']==='true'));
 f.c.currentSavedViewState().stale='changed';f.c.renderSavedViewControls();assert.ok(host.children.find(button=>button.dataset.savedViewControl==='reset'));assert.equal(host.children.find(node=>node.attrs.role==='status').textContent,'Saved view changed. Your filters are kept. Reset to use the saved version.');
 f.c.resetSavedViewState();f.c.renderSavedViewControls();assert.equal(host.children.find(button=>button.dataset.savedViewControl==='save').textContent,'Save view');assert.equal(host.children.some(button=>button.dataset.savedViewControl==='reset'),false);assert.equal(f.calls.length,0);
});

const viewDeferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const editorStatus=dialog=>descendants(dialog).find(node=>node.attrs?.role==='status');
function submittedViews(payload){return {version:1,revision:1,contentHash:'b'.repeat(64),views:JSON.parse(JSON.stringify(payload.views))};}
test('pending Save and Save as new freeze every editor field before native focus loss and block parallel actions and dismissal',async()=>{
 for(const action of ['Save changes','Save as new view']){
  const f=editorFixture(),gate=viewDeferred(),name=control(f.dialog,'View name'),group=control(f.dialog,'Group work'),field=control(f.dialog,'Filter field'),save=editorButton(f.dialog,action),close=control(f.dialog,'Close filter editor'),originalId=f.c.currentSavedViewState().id;let sent;
  name.value='Captured name';close.focus();f.c.api=async(path,payload)=>{f.calls.push({path,payload:JSON.parse(JSON.stringify(payload))});sent=payload;return gate.promise;};const pending=save.onclick();
  assert.equal(f.document.activeElement,editorStatus(f.dialog));assert.equal(f.dialog.attrs['aria-busy'],'true');assert.match(editorStatus(f.dialog).textContent,/Saving saved view/);assert.ok(f.dialog.querySelectorAll('input,textarea,select,button').every(node=>node.disabled));assert.equal(name.disabled,true);assert.equal(field.disabled,true);assert.equal(group.disabled,true);
  await editorButton(f.dialog,'Save changes').onclick();await editorButton(f.dialog,'Save as new view').onclick();editorButton(f.dialog,'Apply filters').onclick();editorButton(f.dialog,'Reset').onclick();editorButton(f.dialog,'Delete view').onclick();await editorButton(f.dialog,'Reload views').onclick();close.onclick();f.c.requestDialogClose(f.dialog);assert.equal(f.dialog.open,true);assert.equal(f.calls.length,1);assert.equal(f.c.currentSavedViewState().id,originalId);assert.equal(descendants(f.dialog).find(node=>node.className==='saved-view-delete-confirmation').hidden,true);
  gate.resolve(submittedViews(sent));await pending;assert.equal(f.dialog.open,false);assert.equal(f.calls.length,1);assert.equal(f.c.currentSavedViewState().draft.name,'Captured name');assert.equal(f.calls[0].payload.revision,0);assert.equal(f.calls[0].payload.contentHash,'a'.repeat(64));
 }
});
test('submitted saved definition and nested conditions are immutable across the await',async()=>{
 const f=editorFixture(),gate=viewDeferred(),name=control(f.dialog,'View name'),entry=descendants(f.dialog).find(node=>node.className==='saved-view-value-entry');let sent;name.value='Captured';f.c.api=async(path,payload)=>{sent=payload;return gate.promise;};const pending=editorButton(f.dialog,'Save changes').onclick();const before=JSON.stringify(sent);name.value='Programmatic later value';if(entry){entry.value='later/programmatic';entry.oninput();}assert.equal(JSON.stringify(sent),before);gate.resolve(submittedViews(sent));await pending;assert.equal(f.c.currentSavedViewState().draft.name,'Captured');assert.ok(!JSON.stringify(f.c.currentSavedViewState().draft.filter).includes('later/programmatic'));
});
test('rejected Save preserves the exact draft nodes and captured CAS baseline then restores owned correction focus and disabled baselines',async()=>{
 for(const status of [400,409]){
  const f=editorFixture(),gate=viewDeferred(),name=control(f.dialog,'View name'),group=control(f.dialog,'Group work'),save=editorButton(f.dialog,'Save changes');name.value='Local draft';group.value='none';group.onchange();const field=control(f.dialog,'Filter field');field.disabled=true;save.focus();let request;f.c.api=async(path,payload)=>{request=payload;return gate.promise;};const pending=save.onclick();const error=Error(status===409?'Saved views changed.':'Invalid view.');error.status=status;gate.reject(error);await pending;
  assert.equal(f.dialog.open,true);assert.equal(control(f.dialog,'View name'),name);assert.equal(control(f.dialog,'Group work'),group);assert.equal(name.value,'Local draft');assert.equal(group.value,'none');assert.equal(name.disabled,false);assert.equal(group.disabled,false);assert.equal(field.disabled,true);assert.equal(save.disabled,false);assert.equal(f.dialog.attrs['aria-busy'],'false');assert.equal(f.document.activeElement,save);assert.match(descendants(f.dialog).find(node=>node.attrs?.role==='alert').textContent,/Your draft is kept/);assert.equal(request.revision,0);assert.equal(request.contentHash,'a'.repeat(64));
 }
});
test('Save rejection never steals focus from App chat or a new modal',async()=>{
 for(const modal of [false,true]){const f=editorFixture(),gate=viewDeferred(),save=editorButton(f.dialog,'Save changes');save.focus();f.c.api=()=>gate.promise;const pending=save.onclick();const outside=new f.Element('input'),host=new f.Element(modal?'dialog':'div',modal?'':'app-chat-panel');if(modal)host.open=true;host.append(outside);f.document.body.append(host);outside.focus();gate.reject(Error('No save.'));await pending;assert.equal(f.document.activeElement,outside);}
});
test('accepted Save and Delete paint failures remain locked and recover through reads only without replaying their writes',async()=>{
 for(const action of ['Save changes','Save as new view','Confirm delete']){
  const f=editorFixture(),deleting=action==='Confirm delete';if(deleting)editorButton(f.dialog,'Delete view').onclick();const button=editorButton(f.dialog,action);button.focus();let writes=0,reads=0,saved;
  f.c.api=async(path,payload)=>{if(payload){writes++;saved=submittedViews(payload);return saved;}reads++;return saved;};f.c.render=()=>{throw Error('Paint unavailable');};await button.onclick();
  assert.equal(writes,1);assert.equal(f.dialog.open,true);assert.match(editorStatus(f.dialog).textContent,/saved|deleted/);assert.match(editorStatus(f.dialog).textContent,/Refresh view.*do not repeat/);const retry=editorButton(f.dialog,'Refresh view');assert.equal(retry.hidden,false);assert.equal(retry.disabled,false);assert.equal(f.document.activeElement,retry);assert.equal(control(f.dialog,'Close filter editor').disabled,false);assert.equal(editorButton(f.dialog,'Save changes').disabled,true);assert.equal(control(f.dialog,'View name').disabled,true);const acceptedId=f.c.currentSavedViewState().id;
  await editorButton(f.dialog,'Save changes').onclick();await editorButton(f.dialog,'Save as new view').onclick();await editorButton(f.dialog,'Confirm delete').onclick();assert.equal(writes,1);await retry.onclick();assert.equal(writes,1);assert.equal(reads,1);assert.equal(f.dialog.open,true);assert.equal(f.c.currentSavedViewState().id,acceptedId);
  f.c.render=()=>{};await retry.onclick();assert.equal(writes,1);assert.equal(reads,2);assert.equal(f.dialog.open,false);assert.equal(f.c.currentSavedViewState().id,acceptedId);
 }
});
test('late accepted saves and deletes keep canonical persistence without activating or closing replacement editors',async()=>{
 for(const action of ['Save changes','Confirm delete'])for(const replacement of [false,true]){
  const f=editorFixture(),old=f.dialog,gate=viewDeferred(),before=JSON.stringify(f.c.currentSavedViewState());let paints=0;f.c.render=()=>paints++;if(action==='Confirm delete')editorButton(old,'Delete view').onclick();let sent;f.c.api=async(path,payload)=>{sent=payload;return gate.promise;};const pending=editorButton(old,action).onclick();old.close();let newer;if(replacement){f.c.openSavedViewEditor();newer=f.document.body.children.at(-1);control(newer,'View name').value='Replacement draft';control(newer,'View name').focus();}const active=f.document.activeElement;
  gate.resolve(submittedViews(sent));await pending;assert.equal(vm.runInContext('savedViewsDocument.revision',f.c),1);assert.equal(JSON.stringify(f.c.currentSavedViewState()),before);assert.equal(paints,0);assert.equal(f.document.activeElement,active);if(newer){assert.equal(newer.open,true);assert.equal(control(newer,'View name').value,'Replacement draft');assert.equal(control(newer,'View name').disabled,false);}
 }
});
test('Reload shares pending freeze and owned focus while preserving the independent filter draft',async()=>{
 const f=editorFixture(),gate=viewDeferred(),name=control(f.dialog,'View name'),reload=editorButton(f.dialog,'Reload views');name.value='Draft name';reload.focus();f.c.api=()=>gate.promise;const pending=reload.onclick();assert.equal(f.document.activeElement,editorStatus(f.dialog));assert.equal(name.disabled,true);await editorButton(f.dialog,'Save changes').onclick();control(f.dialog,'Close filter editor').onclick();assert.equal(f.dialog.open,true);gate.resolve({...structuredClone(defaults),revision:2,contentHash:'c'.repeat(64)});await pending;assert.equal(name.value,'Draft name');assert.equal(name.disabled,false);assert.equal(f.document.activeElement,reload);assert.equal(f.dialog.attrs['aria-busy'],'false');
});
test('pending builder handlers cannot change the editor model even when invoked directly instead of native disabled interaction',async()=>{
 const f=editorFixture(),gate=viewDeferred(),save=editorButton(f.dialog,'Save changes'),beforeSummary=descendants(f.dialog).find(node=>node.className==='form-hint'&&!node.attrs.role).textContent;let first,second;
 f.c.api=async(path,payload)=>{first=JSON.parse(JSON.stringify(payload));return gate.promise;};const pending=save.onclick();
 const logic=control(f.dialog,'Group logic');logic.value='any';logic.onchange();const field=control(f.dialog,'Filter field');field.value='status';field.onchange();const operator=descendants(f.dialog).find(node=>node.attrs?.['aria-label']==='Filter operator');if(operator){operator.value='notIn';operator.onchange();}const entry=descendants(f.dialog).find(node=>node.className==='saved-view-value-entry');if(entry){entry.value='not/submitted';entry.oninput();entry.onblur();}const remove=descendants(f.dialog).find(node=>(node.attrs?.['aria-label']||'').startsWith('Remove ')&&node.className==='saved-view-remove-rule');remove?.onclick();editorButton(f.dialog,'Condition').onclick();editorButton(f.dialog,'Condition group').onclick();assert.equal(descendants(f.dialog).find(node=>node.className==='form-hint'&&!node.attrs.role).textContent,beforeSummary);
 gate.reject(Error('No save.'));await pending;f.c.api=async(path,payload)=>{second=JSON.parse(JSON.stringify(payload));return submittedViews(payload);};await save.onclick();assert.deepEqual(second.views,first.views);
});
test('each pending action captures current disabled baselines instead of restoring those from an earlier rejection',async()=>{
 const f=editorFixture(),field=control(f.dialog,'Filter field');for(const disabled of [true,false]){field.disabled=disabled;const gate=viewDeferred();f.c.api=()=>gate.promise;const pending=editorButton(f.dialog,'Save changes').onclick();assert.equal(field.disabled,true);assert.equal(descendants(f.dialog).find(node=>node.attrs?.role==='alert').textContent,'');gate.reject(Error('No save.'));await pending;assert.equal(field.disabled,disabled);}
});
test('accepted save behind a new modal preserves a read-only recovery action without navigating or stealing focus',async()=>{
 const f=editorFixture(),gate=viewDeferred(),save=editorButton(f.dialog,'Save as new view'),before=f.c.currentSavedViewState().id;let sent,writes=0;f.c.api=async(path,payload)=>{if(payload){writes++;sent=payload;return gate.promise;}return submittedViews(sent);};save.focus();const pending=save.onclick(),other=new f.Element('dialog'),outside=new f.Element('input');other.append(outside);f.document.body.append(other);other.showModal();outside.focus();gate.resolve(submittedViews(sent));await pending;
 assert.equal(f.c.currentSavedViewState().id,before);assert.equal(f.document.activeElement,outside);assert.equal(f.dialog.open,true);assert.match(editorStatus(f.dialog).textContent,/View saved.*Refresh view/);const retry=editorButton(f.dialog,'Refresh view');assert.equal(retry.hidden,false);assert.equal(retry.disabled,false);assert.equal(save.disabled,true);other.close();await retry.onclick();assert.equal(writes,1);assert.equal(f.dialog.open,false);assert.ok(f.c.currentSavedViewState().id.startsWith('view-'));
});
