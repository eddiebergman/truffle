const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
test('provider visibility keeps all, none and selected project meanings distinct',()=>{
 const source=fs.readFileSync('static/page-settings.js','utf8').replace('void loadPageSettings();','');
 const context=vm.createContext({scopeKey:item=>item.project});vm.runInContext(source,context);
 const check=(doc,item)=>{context.doc=doc;context.item=item;return vm.runInContext('pageSettingsDocument=doc;pageSettingsAllows(item,item.source)',context);};
 assert.equal(check(null,{source:'linear',project:'Latency'}),true);
 assert.equal(check({pages:{linear:{scopes:null}}},{source:'linear',project:'New project'}),true);
 assert.equal(check({pages:{linear:{scopes:[]}}},{source:'linear',project:'Latency'}),false);
 const doc={pages:{linear:{scopes:['Latency']}}};
 assert.equal(check(doc,{source:'linear',project:'Latency'}),true);
 assert.equal(check(doc,{source:'linear',project:'Other'}),false);
 assert.equal(check(doc,{source:'github',project:'Other'}),true);
 context.doc=doc;context.item={source:'linear',project:'Other'};assert.equal(vm.runInContext("pageSettingsDocument=doc;pageSettingsAllows(item,'all')",context),true);
});

function dialogFixture({fail=false}={}){
 const calls=[],nodes=[];let document,nativeFocusDrops=0,nativeHiddenDrops=0;
 class Node{
  constructor(tag,cls='',text=''){this.tag=tag;this.disabled=false;this.className=cls;this.textContent=text;this.dataset={};this.attributes={};this.children=[];this.hidden=false;this.classList={contains:name=>this.className.split(' ').includes(name)};nodes.push(this);}
  append(...items){for(const item of items){item.parentElement=this;this.children.push(item);}}
  insertBefore(item,before){item.parentElement=this;this.children.splice(this.children.indexOf(before),0,item);}
  get lastElementChild(){return this.children.at(-1);}
  get isConnected(){return this===body||!this.removed&&!!this.parentElement?.isConnected;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  closest(selector){for(let node=this;node;node=node.parentElement)if(selector.split(',').some(value=>value===node.tag||value[0]==='#'&&node.id===value.slice(1)||value[0]==='.'&&node.classList.contains(value.slice(1))))return node;return null;}
  focus(options){if(this.disabled||this.hidden||!this.isConnected)return;document.activeElement=this;this.focusOptions=options;this.focusCount=(this.focusCount||0)+1;}
  set disabled(value){this.disabledValue=value;if(value&&document?.activeElement===this){document.activeElement=body;nativeFocusDrops++;}}
  get disabled(){return Boolean(this.disabledValue);}
  set hidden(value){this.hiddenValue=value;if(value&&document&&this.contains(document.activeElement)){document.activeElement=body;nativeHiddenDrops++;}}
  get hidden(){return Boolean(this.hiddenValue);}
  setAttribute(key,value){this.attributes[key]=value;}
  addEventListener(name,handler){this['on'+name]=handler;}
  dispatchEvent(event){this['on'+event.type]?.(event);return !event.defaultPrevented;}
  querySelectorAll(selector){const tags=selector.split(',');const visit=node=>node.children.flatMap(child=>[...(tags.includes(child.tag)?[child]:[]),...visit(child)]);return visit(this);}
  showModal(){this.open=true;}
  close(){this.open=false;if(this.contains(document.activeElement))document.activeElement=body;this.onclose?.();}
  remove(){if(this.contains(document.activeElement))document.activeElement=body;this.removed=true;if(this.parentElement)this.parentElement.children=this.parentElement.children.filter(child=>child!==this);}
 }
 const body=new Node('body');document={body,activeElement:body,querySelectorAll:selector=>selector==='dialog[open]'?nodes.filter(node=>node.tag==='dialog'&&node.open&&node.isConnected):[],createTextNode:text=>new Node('text','',text)};const c=vm.createContext({Event,toast:message=>calls.push(message),el:(...args)=>new Node(...args),document,currentSavedViewState:()=>({}),openSavedViewEditor:()=>calls.push('filters'),api:async(path,payload)=>{calls.push({path,payload});if(fail)throw new Error('Fixture load failed');return {revision:7,contentHash:'a'.repeat(64),pages:{sentry:{scopes:payload?.scopes??null}}};},allItems:()=>[{source:'sentry',project:'Frontend'},{source:'sentry',project:'Backend'}],scopeKey:item=>item.project,scopePrefs:{sentry:'Frontend'},persist:()=>calls.push('persist'),render:()=>calls.push('render')});
 vm.runInContext(fs.readFileSync('static/page-settings.js','utf8').replace('void loadPageSettings();',''),c);
 const app=fs.readFileSync('static/app.js','utf8'),features=fs.readFileSync('static/features.js','utf8');vm.runInContext(app.slice(app.indexOf('function holdFormFocus('),app.indexOf('function projectCorrectionTarget(')),c);vm.runInContext(features.slice(features.indexOf('function freezeFeatureForm('),features.indexOf('function presentFeatureFormResult(')),c);vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog')")),c);vm.runInContext(app.slice(app.indexOf('function dismissOpenDialog('),app.indexOf("document.addEventListener('keydown',dismissOpenDialog")),c);
 return {c,body,nodes,calls,document,Node,get nativeFocusDrops(){return nativeFocusDrops;},get nativeHiddenDrops(){return nativeHiddenDrops;}};
}
test('page chooser groups parallel actions with shared control variants and a secondary Close',async()=>{
 const {c,body,nodes,calls}=dialogFixture();await c.openPageSettings('sentry');const dialog=body.children[0],group=dialog.children.find(node=>node.className.includes('ui-actiongroup'));
 assert.equal(group.attributes.role,'group');assert.equal(group.attributes['aria-label'],'Sentry settings actions');assert.deepEqual(group.children.map(node=>node.textContent),['Edit this view’s filters','Choose visible projects','Close']);assert.deepEqual(group.children.map(node=>node.dataset.variant),['primary','secondary','secondary']);assert.ok(group.children.every(node=>node.type==='button'&&node.className==='ui-control'));assert.ok(nodes.every(node=>!node.autofocus&&!Object.hasOwn(node.attributes,'autofocus')));
 group.children[0].onclick();assert.equal(dialog.open,false);assert.equal(dialog.removed,true);assert.deepEqual(calls,['filters']);
});
test('visibility footer groups actions and preserves clear-selection CAS save semantics',async()=>{
 const {c,body,calls}=dialogFixture();await c.openPageVisibilitySettings('sentry');const dialog=body.children[0],group=dialog.children.find(node=>node.className.includes('ui-actiongroup'));
 assert.equal(group.attributes.role,'group');assert.equal(group.attributes['aria-label'],'Sentry visibility actions');assert.deepEqual(group.children.map(node=>node.textContent),['Clear selection','Open Connections settings','Close','Save visibility']);assert.deepEqual(group.children.map(node=>node.dataset.variant),['secondary','secondary','secondary','primary']);
 group.children[0].onclick();await group.children.at(-1).onclick();const save=calls.find(call=>call.payload);assert.deepEqual(JSON.parse(JSON.stringify(save.payload)),{revision:7,contentHash:'a'.repeat(64),page:'sentry',scopes:[]});assert.equal(c.scopePrefs.sentry,'');assert.equal(dialog.open,false);
});
test('visibility load failure retains grouped secondary Close without a broken Save action',async()=>{
 const {c,body}=dialogFixture({fail:true});await c.openPageVisibilitySettings('sentry');const dialog=body.children[0],group=dialog.children.find(node=>node.className.includes('ui-actiongroup'));
 assert.deepEqual(group.children.map(node=>node.textContent),['Open Connections settings','Close']);assert.ok(group.children.every(node=>node.dataset.variant==='secondary'));assert.equal(dialog.children.find(node=>node.className.includes('page-settings-error')).textContent,'Fixture load failed');group.children.at(-1).onclick();assert.equal(dialog.removed,true);
});
test('the shared action-group primitive owns spacing, wrapping and control heights for dialog reuse',()=>{
 const css=fs.readFileSync('static/ui-controls.css','utf8'),pageCSS=fs.readFileSync('static/page-settings.css','utf8');
 assert.match(css,/\.ui-actiongroup,\.dialog-actions\{[^}]*display:flex[^}]*flex-wrap:wrap[^}]*gap:10px[^}]*padding:4px/);assert.match(css,/\.ui-actiongroup>[^}]*min-height:var\(--control-height\)/);assert.ok(!pageCSS.includes('.page-settings-dialog>.text-button'));
});

test('visibility conflicts preserve checkbox choices and offer explicit discard/reload',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const dialog=f.body.children[0],group=dialog.children.find(node=>node.className.includes('ui-actiongroup')),save=group.children.at(-1);
 group.children[0].onclick();const choices=f.nodes.filter(node=>node.type==='checkbox');assert.ok(choices.every(node=>!node.checked));let sent;
 f.c.api=async(path,payload)=>{sent=payload;const error=new Error('Conflict');error.status=409;throw error;};
 f.c.changed={revision:99,contentHash:'b'.repeat(64),pages:{sentry:{scopes:['Backend']}}};vm.runInContext('pageSettingsDocument=changed',f.c);
 await save.onclick();assert.equal(sent.revision,7);assert.equal(sent.contentHash,'a'.repeat(64));assert.equal(dialog.open,true);assert.ok(choices.every(node=>!node.checked));assert.equal(save.disabled,true);assert.ok(!f.calls.includes('render'));
 assert.match(dialog.children.find(node=>node.className.includes('page-settings-error')).textContent,/Your selection is kept/);
 const reload=group.children.find(node=>node.textContent==='Discard selection and reload');assert.ok(reload);f.c.api=async()=>f.c.changed;await reload.onclick();assert.equal(dialog.removed,true);assert.equal(f.body.children.at(-1).open,true);const current=f.nodes.filter(node=>node.type==='checkbox').slice(choices.length);assert.equal(current.find(node=>node.value==='Backend').checked,true);assert.equal(current.find(node=>node.value==='Frontend').checked,false);
});

function visibilityNodes(f){const dialog=f.body.children.at(-1),group=dialog.children.find(node=>node.className.includes('ui-actiongroup'));return {dialog,group,save:group.children.find(node=>node.textContent==='Save visibility'),clear:group.children.find(node=>node.textContent==='Clear selection'),done:group.children.find(node=>node.textContent==='Close'),category:group.children.find(node=>node.textContent==='Open Connections settings'),error:dialog.children.find(node=>node.className.includes('page-settings-error')),checks:f.nodes.filter(node=>node.type==='checkbox')};}
test('pending visibility save freezes edits and all dismissal routes honor the actual cancel guard',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save,clear,done,category,error,checks}=visibilityNodes(f);clear.onclick();checks.find(node=>node.value==='Frontend').checked=true;
 let resolve,writes=0,payload;f.c.api=async(_path,sent)=>{writes++;payload=structuredClone(sent);return new Promise(yes=>resolve=yes);};const pending=save.onclick();assert.equal(dialog.attributes['aria-busy'],'true');assert.ok(dialog.children.some(node=>node.attributes.role==='status'&&!node.hidden&&node.textContent==='Saving visibility…'));assert.ok(dialog.querySelectorAll('input,button').every(node=>node.disabled));
 clear.onclick();assert.equal(checks.find(node=>node.value==='Frontend').checked,true);await save.onclick();assert.equal(writes,1);
 const headerClose=dialog.children[0].children.at(-1);headerClose.onclick();done.onclick();category.onclick();f.c.dismissOpenDialog({key:'Escape',target:{closest:()=>dialog},preventDefault(){},stopImmediatePropagation(){}});assert.equal(dialog.open,true);assert.ok(!dialog.removed);
 resolve({revision:8,contentHash:'b'.repeat(64),pages:{sentry:{scopes:['Frontend']}}});await pending;assert.deepEqual(payload.scopes,['Frontend']);assert.equal(dialog.open,false);assert.equal(dialog.attributes['aria-busy'],'false');
});
test('a rejected pending save keeps its exact checkbox draft and restores correction and dismissal',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save,clear,done,error,checks}=visibilityNodes(f);clear.onclick();checks.find(node=>node.value==='Backend').checked=true;
 let reject;f.c.api=async()=>new Promise((_yes,no)=>reject=no);const pending=save.onclick();reject(Error('Disk unavailable'));await pending;
 assert.equal(dialog.open,true);assert.equal(save.disabled,false);assert.equal(clear.disabled,false);assert.equal(checks.find(node=>node.value==='Backend').checked,true);assert.equal(checks.find(node=>node.value==='Frontend').checked,false);assert.equal(checks.find(node=>node.value==='Backend').disabled,false);assert.equal(error.textContent,'Disk unavailable');done.onclick();assert.equal(dialog.removed,true);
});
test('accepted visibility save with a presentation failure offers read-only recovery instead of resaving',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,group,save,clear,error,checks,done}=visibilityNodes(f);clear.onclick();checks.find(node=>node.value==='Backend').checked=true;
 let writes=0;f.c.api=async()=>{writes++;return {revision:8,contentHash:'b'.repeat(64),pages:{sentry:{scopes:['Backend']}}};};f.c.render=()=>{throw Error('Presentation failed');};await save.onclick();assert.equal(writes,1);assert.equal(dialog.open,true);assert.equal(save.disabled,true);assert.equal(clear.disabled,true);assert.ok(checks.every(node=>node.disabled));assert.equal(done.disabled,false);assert.match(error.textContent,/Visibility saved.*Refresh view.*do not save again/);
 await save.onclick();assert.equal(writes,1);const retry=group.children.find(node=>node.textContent==='Refresh view');assert.ok(retry);retry.onclick();assert.equal(writes,1);assert.equal(dialog.open,true);
 f.c.render=()=>{};retry.onclick();assert.equal(dialog.open,false);assert.equal(writes,1);
});
test('a superseded pending visibility dialog never repaints after its accepted save',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save}=visibilityNodes(f);let resolve;f.c.api=async()=>new Promise(yes=>resolve=yes);const pending=save.onclick();dialog.close();resolve({revision:8,contentHash:'b'.repeat(64),pages:{sentry:{scopes:null}}});await pending;assert.ok(!f.calls.includes('render'));assert.match(f.calls.at(-1),/Visibility saved.*Refresh the dashboard/);
});


function pendingVisibility(f){let resolve,reject,writes=0;f.c.api=async()=>{writes++;return new Promise((yes,no)=>{resolve=yes;reject=no;});};return {resolve:value=>resolve(value),reject:error=>reject(error),get writes(){return writes;}};}
function visibilityStatus(dialog){return dialog.children.find(node=>node.tabIndex===-1);}
test('visibility Save holds native focus before disabling and restores exact rejected correction controls',async()=>{
 for(const kind of ['save','search','checkbox']){const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save,clear,checks}=visibilityNodes(f);clear.onclick();checks.find(node=>node.value==='Backend').checked=true;const origin=kind==='save'?save:kind==='search'?f.nodes.find(node=>node.type==='search'):checks.find(node=>node.value==='Backend');origin.focus();const outcome=pendingVisibility(f),saving=save.onclick(),status=visibilityStatus(dialog);assert.equal(f.document.activeElement,status);assert.equal(status.hidden,false);assert.equal(status.attributes.role,'status');assert.equal(f.nativeFocusDrops,0);outcome.reject(Error('Disk unavailable'));await saving;assert.equal(f.document.activeElement,origin);assert.equal(origin.focusOptions.preventScroll,true);assert.equal(status.hidden,true);assert.equal(f.nativeHiddenDrops,0);assert.equal(checks.find(node=>node.value==='Backend').checked,true);assert.equal(checks.find(node=>node.value==='Frontend').checked,false);assert.equal(outcome.writes,1);}
});

test('visibility conflict focuses its exact Reload action while preserving selections and baseline disabled states',async()=>{
 for(const selected of [true,false]){const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save,clear,checks,group}=visibilityNodes(f);if(!selected)clear.onclick();save.focus();const outcome=pendingVisibility(f),saving=save.onclick(),status=visibilityStatus(dialog),error=Error('Conflict');error.status=409;outcome.reject(error);await saving;const reload=group.children.find(node=>node.textContent==='Discard selection and reload');assert.equal(f.document.activeElement,reload);assert.equal(reload.focusOptions.preventScroll,true);assert.equal(status.hidden,true);assert.equal(save.disabled,true);for(const check of checks.filter(node=>node.value))assert.equal(check.disabled,selected);assert.equal(outcome.writes,1);assert.equal(f.nativeFocusDrops,0);assert.equal(f.nativeHiddenDrops,0);}
});

test('accepted visibility paint failure focuses read-only Refresh and repeated failure never saves or loses focus',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save,group}=visibilityNodes(f);save.focus();let writes=0;const paintFocus=[];f.c.api=async()=>{writes++;return {revision:8,contentHash:'b'.repeat(64),pages:{sentry:{scopes:null}}};};f.c.render=()=>{paintFocus.push(f.document.activeElement);throw Error('Paint failed');};await save.onclick();const retry=group.children.find(node=>node.textContent==='Refresh view'),status=visibilityStatus(dialog);assert.equal(f.document.activeElement,retry);assert.equal(status.hidden,true);assert.equal(save.disabled,true);assert.equal(paintFocus[0],status);retry.onclick();assert.equal(paintFocus[1],status);assert.equal(f.document.activeElement,retry);assert.equal(status.hidden,true);assert.equal(writes,1);assert.equal(f.nativeHiddenDrops,0);const outside=new f.Node('input');f.body.append(outside);outside.focus();retry.onclick();assert.equal(paintFocus[2],outside);assert.equal(f.document.activeElement,outside);assert.equal(writes,1);f.c.render=()=>{};const count=retry.focusCount;retry.focus();retry.onclick();assert.equal(dialog.open,false);assert.equal(retry.focusCount,count+1,'successful close does not restore old recovery focus');assert.equal(writes,1);
});

test('visibility settlement never takes outside, child-modal or replacement focus',async()=>{
 for(const mode of ['outside','child-modal','closed','replacement']){const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save}=visibilityNodes(f);save.focus();const outcome=pendingVisibility(f),saving=save.onclick();let outside;if(mode==='outside'){outside=new f.Node('input');f.body.append(outside);}else{if(mode==='closed')dialog.close();const next=new f.Node('dialog');f.body.append(next);next.showModal();outside=new f.Node('input');next.append(outside);}outside.focus();outcome.reject(Error('Save rejected'));await saving;assert.equal(f.document.activeElement,outside,mode);assert.equal(outcome.writes,1);assert.equal(save.focusCount,1);}
});

test('a blocked focus restoration keeps its neutral holder visible instead of hiding focused content',async()=>{
 const f=dialogFixture();await f.c.openPageVisibilitySettings('sentry');const {dialog,save}=visibilityNodes(f);save.focus();const outcome=pendingVisibility(f),saving=save.onclick(),status=visibilityStatus(dialog),child=new f.Node('dialog');f.body.append(child);child.showModal();outcome.reject(Error('Fix the selection'));await saving;assert.equal(f.document.activeElement,status);assert.equal(status.hidden,false);assert.equal(status.textContent,'Fix the selection');assert.equal(f.nativeHiddenDrops,0);assert.equal(save.focusCount,1);
});
