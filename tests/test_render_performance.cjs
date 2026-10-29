const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const read=name=>fs.readFileSync(path.join(__dirname,'..','static',name),'utf8');
const app=read('app.js');

function fixture(){
  const nodes=new Map(),drawn=[];
  const node=()=>({children:[],dataset:{},value:'',classList:{toggle(){}},append(...items){this.children.push(...items);},replaceChildren(...items){this.children=items;}});
  const $=id=>{if(!nodes.has(id))nodes.set(id,node());return nodes.get(id);};
  $('#group-by').value='project';
  const context=vm.createContext({$,document:{activeElement:null,querySelectorAll:()=>[],createTextNode:text=>text},
    el:(tag,cls,text)=>Object.assign(node(),{tag,cls,text}),
    renderWorkspacePage:()=>context.renderWorkPage(),allItems:()=>context.state.items,buildBundleIndex(){},updateGroupingControl(){},renderScopeFilters(){},renderFolderSelector(){},renderBoardSessions(){},renderHiddenItems(){},renderSnoozed(){},renderSentryControls(){},renderFeatureView:()=>false,renderSync(){},
    matchesScope:()=>true,matchesSentry:()=>true,matchesSource:()=>true,isSource:(item,source)=>item.source===source,
    orderedGithubItems:require('../static/github-actions.js').orderedGithubItems,
    appendGithubStacks:(group,items,all,limit)=>{for(const item of items.slice(0,limit))context.renderRow(item);},
    workReference:item=>item.id,scopeKey:item=>item.project,scopeLabel:key=>key,
    renderRow:item=>{drawn.push(item.id);return node();},
    appendLinearTree:(group,items)=>{for(const item of items)context.renderRow(item);},
    sessionWorkState:()=> 'active',source:'sentry',githubView:'all',scopePrefs:{},sentryFilters:{},sentryView:'',
    sourceNames:{sentry:'Sentry',linear:'Linear'},labels:{todo:'To do'},order:['todo'],statusFilter:{value:'all'},
    state:{items:Array.from({length:250},(_,i)=>({id:'id'+i,title:'Issue '+String(i).padStart(3,'0'),project:i<150?'A':'B',source:'sentry',status:'todo',eventCount:250-i})),sessions:[]}});
  vm.runInContext('const workPageSize=100;let workPageLimit=workPageSize,workPageKey="";'+app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function renderRow(item)')),context);
  return {context,$,drawn,render(){drawn.length=0;context.render();}};
}
test('work paging spans groups, expands on demand and searches all cached records',()=>{
  const f=fixture();f.render();assert.equal(f.drawn.length,100);
  assert.deepEqual(f.drawn.slice(0,2),['id0','id1']);
  const footer=f.$('#work-list').children.at(-1);
  assert.match(footer.children[0].text,/100 of 250/);
  f.drawn.length=0;footer.children[1].onclick();assert.equal(f.drawn.length,200);
  assert.equal(f.$('#work-list').children.length,3); // two repository groups and the paging control
  assert.equal(f.$('#work-list').children[0].children[0].children[2].text,150); // full group count
  f.drawn.length=0;f.$('#search').value='Issue 249';f.render();assert.deepEqual(f.drawn,['id249']);
  f.$('#search').value='';f.render();assert.equal(f.drawn.length,100);
});
test('Linear pagination retains the tree renderer and resets when changing view',()=>{
  const f=fixture();f.context.source='linear';f.context.state.items=f.context.state.items.map(item=>({...item,source:'linear'}));f.render();assert.equal(f.drawn.length,100);
  f.$('#work-list').children.at(-1).children[1].onclick();
  f.context.source='sentry';f.context.state.items=f.context.state.items.map(item=>({...item,source:'sentry'}));f.render();assert.equal(f.drawn.length,100);
});
test('snapshot revision detects restart and mutations, and legacy services compare contents',()=>{
  const context=vm.createContext({});vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),context);
  assert.equal(context.sameSnapshot(null,{revision:'p1:1'}),false);
  assert.equal(context.sameSnapshot({revision:'p1:1'},{revision:'p1:1'}),true);
  assert.equal(context.sameSnapshot({revision:'p1:1'},{revision:'p2:1'}),false);
  assert.equal(context.sameSnapshot({revision:'p1:1'},{revision:'p1:2'}),false);
  assert.equal(context.sameSnapshot({items:[]},{items:[]}),true);
  assert.equal(context.sameSnapshot({items:[]},{items:[1]}),false);
});
test('session link index uses direct attachments and excludes transitive bundle neighbours',()=>{
  const features=read('features.js'),sessions=read('sessions.js');
  const context=vm.createContext({state:{relatedItems:[],sessions:[{id:'one',items:[{id:'b'},{id:'c'}]},{id:'archived',archivedAt:1,items:[{id:'a'}]},{id:'two',items:[{id:'a'}]}],bundles:[{itemIds:['a','b']}]},allItems:()=>[]});
  vm.runInContext('let bundleIndex,bundleCatalog,linkedSessionIndex;'+features.slice(features.indexOf('function buildBundleIndex('),features.indexOf('function relatedRecords('))+sessions.slice(sessions.indexOf('function linkedSessions('),sessions.indexOf('let quickStarting')),context);
  context.buildBundleIndex();
  assert.deepEqual(Array.from(context.linkedSessions({id:'a'}),x=>x.id),['two']);
  assert.deepEqual(Array.from(context.linkedSessions({id:'c'}),x=>x.id),['one']);
  assert.equal(context.linkedSessions({id:'missing'}).length,0);
  context.state.sessions=[];context.buildBundleIndex();assert.equal(context.linkedSessions({id:'a'}).length,0);
});

test('unchanged refresh updates sync text without rebuilding DOM and changes do render',async()=>{
  let renders=0,syncs=0,hashes=0,configurations=0;
  const context=vm.createContext({window:{refreshConfiguration:()=>configurations++},state:{revision:'process:1'},refreshing:false,
    api:async()=>context.next,render(){renders++;},renderSync(){syncs++;},handleSessionHash(){hashes++;}});
  vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function mutate(')),context);
  context.next={revision:'process:1'};await context.refresh();
  assert.equal(renders,0);assert.equal(syncs,1);assert.equal(hashes,1);
  context.next={revision:'process:2'};await context.refresh();assert.equal(renders,1);assert.equal(configurations,2);
});
test('hide and restore move focus without scrolling the page',async()=>{
  for(const hidden of [false,true]){
    const focusCalls=[],mutations=[];
    const target={dataset:{focusKey:'item:visibility'},focus:options=>focusCalls.push(options)};
    const node=()=>({dataset:{},setAttribute(){},append(){}});
    const context=vm.createContext({el:node,
      source:'github',currentItem:null,document:{createElementNS:node,querySelectorAll:()=>[target]},
      $:id=>id==='#item-dialog'?{open:false}:target,
      mutate:async payload=>{mutations.push(payload);return {kind:'accepted',presentation:'ready'};},toast(){}});
    vm.runInContext(app.slice(app.indexOf('function visibilityButton('),app.indexOf('function renderHiddenItems(')),context);
    vm.runInContext(app.slice(app.indexOf('function localWriteMessage('),app.indexOf('async function mutateChatMetadata(')),context);
    const button=context.visibilityButton({id:'item',title:'An issue',hidden});context.document.activeElement=button;await button.onclick();
    assert.equal(mutations[0].value.hidden,!hidden);
    assert.equal(focusCalls.length,1);assert.equal(focusCalls[0].preventScroll,true);
    assert.equal(button.disabled,false);
  }
});
test('item projections are reused across view renders and invalidated by a new snapshot',()=>{
  const context=vm.createContext({state:{items:[{id:'one',source:'github',status:'todo',title:'Original'}],notes:[],overrides:{one:{hidden:true}}}});
  vm.runInContext(app.slice(app.indexOf('let itemCacheState='),app.indexOf('function relative(')),context);
  const first=context.allItems();assert.equal(first[0].hidden,true);assert.equal(context.allItems(),first);
  context.state={...context.state,overrides:{one:{status:'done'}}};
  const second=context.allItems();assert.notEqual(second,first);assert.equal(second[0].status,'done');assert.equal(second[0].hidden,false);
  assert.equal(context.state.items[0].status,'todo');
});
test('closed Hidden section keeps its count without creating work rows',()=>{
  let rows=0;
  const section={dataset:{},open:false},list={replaceChildren(){},append(){}};
  const nodes={'#hidden-items':section,'#hidden-list':list,'#hidden-summary':{},'#search':{value:''}};
  const context=vm.createContext({$:id=>nodes[id],source:'github',hiddenSectionsOpen:new Set(),state:{sessions:[]},matchesSource:()=>true,matchesScope:()=>true,workReference:()=>'',el:()=>({}),renderRow(){rows++;}});
  vm.runInContext(app.slice(app.indexOf('function renderHiddenItems('),app.indexOf("$('#hidden-items').ontoggle=")),context);
  const items=[{id:'one',hidden:true,title:'Hidden issue'}];context.renderHiddenItems(items);
  assert.equal(nodes['#hidden-summary'].textContent,'Hidden · 1');assert.equal(rows,0);
  context.hiddenSectionsOpen.add('github');context.renderHiddenItems(items);assert.equal(rows,1);
});
test('closed Snoozed section defers its rows until expanded',()=>{
  let rows=0;const section={open:false},list={replaceChildren(){},append(){rows++;}},nodes={'#snoozed-items':section,'#snoozed-list':list,'#snoozed-summary':{},'#search':{value:''}};
  const context=vm.createContext({$:id=>nodes[id],source:'github',state:{snoozes:{one:{until:123}}},matchesSource:()=>true,matchesScope:()=>true,workReference:()=>'',featureButton(){},dated:()=>'',el:()=>({append(){}})});
  const features=read('features.js');vm.runInContext(features.slice(features.indexOf('function renderSnoozed('),features.indexOf("$('#snoozed-items').ontoggle=")),context);
  const items=[{id:'one',snoozed:true,title:'Snoozed issue'}];context.renderSnoozed(items);assert.equal(rows,0);assert.equal(nodes['#snoozed-summary'].textContent,'Snoozed · 1');
  section.open=true;context.renderSnoozed(items);assert.equal(rows,1);
});

test('search ranks the best repository first and does not fuzzy-match long descriptions',()=>{
  const f=fixture();f.context.source='github';f.context.state.items=[
    {id:'unrelated',title:'Dependency update',project:'A',detail:'gaps between cycles and stages',source:'github',status:'todo'},
    {id:'match',title:'perf(gcs): upload in one request',project:'Z',source:'github',status:'todo'}];
  f.$('#search').value='gcs';f.render();assert.deepEqual(f.drawn,['match']);
});

test('an empty filtered view clears search and scope without offering a project creation',()=>{
  const f=fixture();f.context.persist=()=>{};f.$('#search').value='no-such-result';f.context.scopePrefs.sentry='A';f.render();
  const empty=f.$('#work-list').children[0];assert.equal(empty.children[0].text,'No items match these filters.');assert.equal(empty.children[1].text,'Clear filters');
  empty.children[1].onclick();assert.equal(f.$('#search').value,'');assert.equal(f.context.scopePrefs.sentry,'');
});
test('stack grouping receives the full catalog but respects filtered paging and load-more',()=>{
  const f=fixture();f.context.source='github';f.context.state.items=f.context.state.items.map(item=>({...item,source:'github',kind:'Pull request'}));f.$('#group-by').value='stack';
  let catalogSize=0,matched=0;
  f.context.appendGithubStacks=(list,visible,all,limit)=>{catalogSize=all.length;matched=visible.length;for(const item of visible.slice(0,limit))f.context.renderRow(item);};
  f.render();assert.equal(catalogSize,250);assert.equal(matched,250);assert.equal(f.drawn.length,100);
  f.drawn.length=0;f.$('#work-list').children.at(-1).children[1].onclick();assert.equal(f.drawn.length,200);
  f.$('#search').value='Issue 249';f.render();assert.equal(catalogSize,250);assert.equal(matched,1);assert.deepEqual(f.drawn,['id249']);
});

test('default GitHub repository grouping orders stack layers before paginating',()=>{
  const f=fixture();f.context.source='github';
  const pr=(id,title,head,base)=>({id,title,source:'github',kind:'Pull request',project:'org/repo',headRepository:'org/repo',headRefName:head,baseRefName:base,reference:'#'+id,status:'todo'});
  f.context.state.items=[...Array.from({length:101},(_,i)=>pr('solo'+i,'A standalone '+i,'solo'+i,'main')),pr('tip','B tip','tip','middle'),pr('middle','Y middle','middle','base'),pr('base','Z base','base','main')];
  f.render();assert.equal(f.drawn.length,100);assert.deepEqual(f.drawn.slice(0,3),['base','middle','tip']);
});

test('status grouping retains unknown and missing statuses under Unknown',()=>{
 const f=fixture();f.$('#group-by').value='status';f.context.state.items=[{id:'known',title:'Known',source:'sentry',status:'todo'},{id:'unknown',title:'Unknown',source:'sentry',status:'pending-provider-state'},{id:'missing',title:'Missing',source:'sentry'}];
 f.render();assert.deepEqual(f.drawn,['known','missing','unknown']);assert.equal(f.$('#work-list').children[1].children[0].children[1],'Unknown');
});


test('disabled connections show a finite honest sync state',()=>{
  const nodes=new Map();const $=id=>{if(!nodes.has(id))nodes.set(id,{classList:{toggle(){}},setAttribute(){},replaceChildren(){},append(){}});return nodes.get(id);};
  $('#connection-notice').parentElement=$('#sidebar-notices');
  const context=vm.createContext({$,state:{sources:{github:{disabled:true},linear:{disabled:true},sentry:{disabled:true}},syncing:[]},sourceNames:{},Date,localStorage:{removeItem(){},getItem(){return null;}},document:{createTextNode:value=>value},el:()=>({setAttribute(){}}),openSettings(){}});
  vm.runInContext(app.slice(app.indexOf('function renderSync()'),app.indexOf('const projectEditableFields=')),context);
  context.renderSync();assert.equal($('#sync').disabled,true);assert.equal($('#force-refresh').disabled,true);assert.equal($('#sync span').textContent,'No connections enabled');
  context.state.demo=true;context.renderSync();assert.equal($('#sync span').textContent,'Sample workspace');
});

test('an old periodic snapshot cannot overwrite a successful metadata command',async()=>{
 let resolveRead;const original={revision:'fixture:1',sessions:[{id:'chat',title:'Before'}]},old=structuredClone(original);let renders=0,syncs=0;
 const context=vm.createContext({state:original,refreshing:false,window:{},api:async(path)=>path==='/api/state'?new Promise(resolve=>resolveRead=resolve):{chat:{id:'chat',title:'After',updatedAt:2}},render(){renders++;},renderSync(){syncs++;},handleSessionHash(){},toast(){},$:()=>({textContent:'',classList:{add(){}}})});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('function workStatusLabel(')),context);
 const pending=context.refresh();await context.mutateChatMetadata('chats.rename',{id:'chat',title:'After'});const committed=context.state;
 resolveRead(old);await pending;
 assert.equal(context.state,committed);assert.equal(context.state.sessions[0].title,'After');assert.equal(renders,1);assert.equal(syncs,0);assert.equal(context.refreshing,false);
});
test('shared snapshot loader commits before returning acceptance metadata',async()=>{
 let resolveRead;const context=vm.createContext({state:{revision:'fixture:1'},api:()=>new Promise(resolve=>resolveRead=resolve)});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),context);
 const pending=context.loadStateSnapshot();resolveRead({revision:'fixture:2',sessions:[]});
 const result=await pending.then(result=>{assert.equal(context.state.revision,'fixture:2');context.state={...context.state,localChange:true};return result;});
 assert.deepEqual(JSON.parse(JSON.stringify(result)),{accepted:true,changed:true});assert.equal(context.state.localChange,true);
});
