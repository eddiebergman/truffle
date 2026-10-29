const {test}=require('node:test');const assert=require('node:assert/strict');
const {normalizeShareHandle,matchingSharePeople}=require('../static/sharing.js');
test('direct names normalize without requiring a directory',()=>{assert.equal(normalizeShareHandle(' /Eddie '),'eddie');for(const value of ['../eddie','/','//eddie','first last','a/b'])assert.equal(normalizeShareHandle(value),'');});
test('autocomplete uses names and handles, never aliases or emails',()=>{const people=[{id:'alex',name:'Alex Example',aliases:['private@example.org']},{id:'sam',name:'Samantha'},{id:'retired',name:'Old',active:false}];assert.deepEqual(matchingSharePeople(people,'saman').map(p=>p.id),['sam']);assert.deepEqual(matchingSharePeople(people,'/alex').map(p=>p.id),['alex']);assert.deepEqual(matchingSharePeople(people,'private@example'),[]);assert.deepEqual(matchingSharePeople(people,'retired'),[]);});

test('inbox refresh retries a failed selected preview without importing a chat or retrying active work',async()=>{
  const fs=require('node:fs'),vm=require('node:vm'),source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8');
  for(const status of ['failed','working','ready']){
    const calls=[];const c=vm.createContext({data:{entries:[{id:'delivery'}]},previewId:'delivery',previewJob:'old-job',previewStarting:false,lastRendered:'',paint(){},currentJob:()=>({status}),call:async(path,payload)=>{calls.push({path,payload});return {jobId:'retried-preview'};},$:()=>({}),message(){}});
    vm.runInContext(source.slice(source.indexOf('  async function previewEntry('),source.indexOf('  function paintPeople(')),c);
    await c.refreshInbox();assert.deepEqual(calls.map(c=>c.path),status==='failed'?['sharing.refresh','sharing.preview']:['sharing.refresh']);
    if(status==='failed'){assert.equal(c.previewJob,'retried-preview');assert.equal(calls[1].payload.deliveryId,'delivery');}
  }
});

const {sharingTransport,sharingProfile,parseSharingProfile,sharingConfigurationPayload,sharingTransportHint}=require('../static/sharing.js');
test('sharing profiles explicitly select folder or GCS without claiming identity or changing access',()=>{
 const folder=sharingProfile('folder','/Volumes/Team Share/',' /Eddie ');assert.deepEqual(folder.transport,{kind:'folder',root:'/Volumes/Team Share'});assert.equal(folder.identity.handle,'eddie');assert.deepEqual(parseSharingProfile(folder),folder);assert.deepEqual(sharingTransport({root:'gs://bucket/prefix'}),{kind:'gcs',root:'gs://bucket/prefix'});assert.equal(sharingProfile('gcs','gs://bucket/prefix/','eddie').transport.root,'gs://bucket/prefix');assert.match(sharingTransportHint('folder'),/same absolute path/);assert.match(sharingTransportHint('folder'),/does not grant access/);
 for(const [kind,root]of [['folder','relative/path'],['folder','/'],['folder','/shared/../outside'],['gcs','file:///shared'],['other','/shared']])assert.throws(()=>sharingProfile(kind,root,'eddie'));
 assert.throws(()=>parseSharingProfile({...folder,version:2}));
});
test('saving a reviewed profile uses the current local configuration CAS, never imported revisions',()=>{
 const profile=sharingProfile('folder','/Volumes/Team','sam');profile.revision=999;profile.contentHash='c'.repeat(64);const imported=parseSharingProfile(profile);const request=sharingConfigurationPayload({revision:7,contentHash:'a'.repeat(64)},imported.transport.kind,imported.transport.root,imported.identity.handle);assert.equal(request.revision,7);assert.equal(request.contentHash,'a'.repeat(64));assert.equal(request.transport.kind,'folder');assert.throws(()=>sharingConfigurationPayload({},'folder','/Volumes/Team','sam'),/Reload/);
});
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
function sharingSettingsFixture(){
 const fs=require('node:fs'),vm=require('node:vm'),calls=[];
 class Node{
  constructor(tag,cls='',text=''){this.tag=tag;this.className=cls;this.textContent=text;this.children=[];this.value='';this.attributes={};this.parentNode=null;this.open=tag==='dialog';}
  append(...items){for(const item of items){item.remove?.();this.children.push(item);if(item&&typeof item==='object')item.parentNode=this;}}
  replaceChildren(...items){for(const child of this.children)if(child&&typeof child==='object')child.parentNode=null;this.children=[];this.append(...items);}
  remove(){if(this.parentNode){this.parentNode.children=this.parentNode.children.filter(child=>child!==this);this.parentNode=null;}}
  setAttribute(key,value){this.attributes[key]=value;}
  add(option){this.children.push(option);}
  closest(selector){for(let node=this;node;node=node.parentNode)if(node.tag===selector)return node;return null;}
  get isConnected(){return this.tag==='document'||!!this.parentNode?.isConnected;}
  click(){}
 }
 let config={revision:2,contentHash:'a'.repeat(64),handle:'eddie',root:'gs://bucket/prefix',transport:{kind:'gcs',root:'gs://bucket/prefix'}},handler=null;
 const document=new Node('document'),dialog=new Node('dialog');document.append(dialog);
 const c=vm.createContext({structuredClone,sharingTransport,sharingProfile,parseSharingProfile,sharingConfigurationPayload,sharingTransportHint,el:(...args)=>new Node(...args),settingsButton:(text,run,cls)=>{const node=new Node('button',cls,text);node.onclick=run;return node;},Option:function(text,value){this.text=text;this.value=value;},sharingCommand:async(name,args)=>{calls.push({name,args});if(handler)return handler(name,args);if(name==='sharing.configure'){config={...config,revision:config.revision+1,contentHash:'b'.repeat(64),handle:args.handle,transport:args.transport,root:args.transport.root};}return {config:structuredClone(config)};}});
 const source=fs.readFileSync(require.resolve('../static/product-tools.js'),'utf8');vm.runInContext(source.slice(source.indexOf('const sharingSettingsDraft='),source.indexOf('let onboardingOffered=')),c);
 let panel;const walk=node=>[node,...node.children.filter(child=>child instanceof Node).flatMap(walk)];
 return {c,calls,dialog,config:()=>config,setConfig(value){config=value;},request(fn){handler=fn;},state:()=>vm.runInContext('sharingSettingsDraft',c),
  async render(){panel?.remove();panel=new Node('panel');dialog.append(panel);await c.renderSharingSettings(panel);return panel;},
  input(label){return walk(panel).find(node=>node.attributes['aria-label']===label);},
  button(label){return walk(panel).find(node=>node.tag==='button'&&node.textContent===label);},
  file(){return walk(panel).find(node=>node.type==='file');},
  status(){return walk(panel).find(node=>node.attributes.role==='status').textContent;},
  text(){return walk(panel).map(node=>node.textContent).join(' ');},
  edit(label,value){const node=this.input(label);node.value=value;node.oninput();},
  detach(){panel.remove();},
 };
}

test('Settings sharing unreadable initial metadata never claims saved and can retry',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();f.request(async()=>({config:{error:'Sharing configuration is unreadable.'}}));await f.render();
 assert.equal(f.state().baseline,null);assert.equal(f.state().draft,null);assert.equal(f.button('Save sharing configuration'),undefined);assert.match(f.text(),/configuration is unreadable/);assert.doesNotMatch(f.text(),/configuration is saved/);
 f.request(null);await f.render();assert.ok(f.button('Save sharing configuration'));assert.equal(f.state().baseline.revision,2);assert.match(f.status(),/configuration is saved/);
});

test('Settings sharing failed reload and clean refresh preserve the current baseline and draft',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();f.edit('Receiving inbox name','reviewed');const baseline=JSON.stringify(f.state().baseline);
 f.request(async()=>({config:{error:'Sharing configuration is unreadable.'}}));await f.button('Discard draft & reload').onclick();
 assert.equal(f.input('Receiving inbox name').value,'reviewed');assert.equal(JSON.stringify(f.state().baseline),baseline);assert.match(f.status(),/configuration is unreadable/);
 f.request(null);await f.button('Discard draft & reload').onclick();
 f.request(async()=>({config:{revision:-1,contentHash:'bad'}}));await f.render();await new Promise(resolve=>setImmediate(resolve));
 assert.equal(f.input('Receiving inbox name').value,'eddie');assert.equal(JSON.stringify(f.state().baseline),baseline);assert.match(f.status(),/Could not load/);
});

test('Settings sharing profile stays a reviewed draft across repaint and close/reopen with its pinned CAS',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();
 const file=f.file();file.files=[{size:100,text:async()=>JSON.stringify(sharingProfile('folder','/Volumes/Team','sam'))}];await file.onchange();
 assert.deepEqual(f.calls.map(call=>call.name),['sharing.state']);assert.match(f.status(),/loaded for review/);
 const old=f.config();f.setConfig({...old,revision:9,contentHash:'c'.repeat(64),handle:'external'});
 await f.render();assert.equal(f.input('Sharing storage root').value,'/Volumes/Team');
 f.dialog.open=false;f.dialog.open=true;await f.render();assert.equal(f.input('Receiving inbox name').value,'sam');
 await f.button('Save sharing configuration').onclick();const request=f.calls.find(call=>call.name==='sharing.configure').args;
 assert.equal(request.revision,2);assert.equal(request.contentHash,'a'.repeat(64));assert.equal(request.transport.root,'/Volumes/Team');assert.equal(request.handle,'sam');
 await f.button('Discard draft & reload').onclick();assert.equal(f.input('Sharing storage root').value,'/Volumes/Team');assert.match(f.status(),/discarded/);
});

test('Settings sharing refreshes a clean reopen but never rebases a dirty draft',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();const original=f.config(),fresh={...original,revision:8,contentHash:'c'.repeat(64),handle:'fresh'};f.setConfig(fresh);await f.render();await new Promise(resolve=>setImmediate(resolve));
 assert.equal(f.input('Receiving inbox name').value,'fresh');assert.equal(f.state().baseline.revision,8);
 f.edit('Receiving inbox name','reviewed');const calls=f.calls.length;f.setConfig({...fresh,revision:9,handle:'external'});await f.render();assert.equal(f.calls.length,calls);assert.equal(f.input('Receiving inbox name').value,'reviewed');assert.equal(f.state().baseline.revision,8);
});

test('Settings sharing clean refresh cannot overwrite an edit while its response is pending',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();const pending=deferred();f.request(()=>pending.promise);await f.render();f.edit('Receiving inbox name','typed');pending.resolve({config:{...f.config(),revision:9,handle:'external'}});await new Promise(resolve=>setImmediate(resolve));assert.equal(f.input('Receiving inbox name').value,'typed');assert.equal(f.state().baseline.revision,2);
});

test('Settings sharing delayed imports cannot replace newer edits, newer imports or a replacement renderer',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();
 const first=deferred(),file=f.file();file.files=[{size:100,text:()=>first.promise}];const pending=file.onchange();
 f.edit('Sharing storage root','/Volumes/Newer edit');first.resolve(JSON.stringify(sharingProfile('folder','/Volumes/Old file','sam')));await pending;
 assert.equal(f.input('Sharing storage root').value,'/Volumes/Newer edit');assert.match(f.status(),/changed while reading/);
 const second=deferred();file.files=[{size:100,text:()=>second.promise}];const olderImport=file.onchange();
 file.files=[{size:100,text:async()=>JSON.stringify(sharingProfile('folder','/Volumes/Newer file','sam'))}];await file.onchange();second.resolve(JSON.stringify(sharingProfile('folder','/Volumes/Old file','sam')));await olderImport;
 assert.equal(f.input('Sharing storage root').value,'/Volumes/Newer file');
 const third=deferred();file.files=[{size:100,text:()=>third.promise}];const detachedImport=file.onchange();await f.render();third.resolve(JSON.stringify(sharingProfile('folder','/Volumes/Detached file','sam')));await detachedImport;
 await f.render();assert.equal(f.input('Sharing storage root').value,'/Volumes/Newer file');assert.equal(file.value,'');
});

test('Settings sharing ignores private file completion after the dialog closes',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();const pending=deferred(),file=f.file();file.files=[{size:100,text:()=>pending.promise}];const importing=file.onchange();f.dialog.open=false;
 pending.resolve(JSON.stringify(sharingProfile('folder','/Volumes/Closed','sam')));await importing;f.dialog.open=true;await f.render();assert.equal(f.input('Sharing storage root').value,'gs://bucket/prefix');assert.equal(file.value,'');
});

test('Settings sharing delayed reload preserves edits made while loading and ignores detached completion',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();const pending=deferred();f.request(()=>pending.promise);
 const reload=f.button('Discard draft & reload').onclick();f.edit('Receiving inbox name','newer');
 pending.resolve({config:{...f.config(),handle:'external',revision:3}});await reload;
 assert.equal(f.input('Receiving inbox name').value,'newer');assert.match(f.status(),/changed while reloading/);assert.equal(f.state().baseline.revision,2);
 const detached=deferred();f.request(()=>detached.promise);const old=f.button('Discard draft & reload').onclick();await f.render();detached.resolve({config:{...f.config(),handle:'detached',revision:4}});await old;
 assert.equal(f.input('Receiving inbox name').value,'newer');assert.equal(f.state().baseline.revision,2);
 f.request(async()=>({config:{...f.config(),handle:'accepted',revision:5}}));await f.button('Discard draft & reload').onclick();assert.equal(f.input('Receiving inbox name').value,'accepted');assert.equal(f.state().baseline.revision,5);
});

test('Settings sharing pending Save freezes conflicting controls across repaint and restores the current renderer',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();f.edit('Receiving inbox name','sam');const pending=deferred();f.request(()=>pending.promise);
 const save=f.button('Save sharing configuration').onclick();await f.render();
 for(const label of ['Sharing storage type','Sharing storage root','Receiving inbox name'])assert.equal(f.input(label).disabled,true);
 for(const label of ['Save sharing configuration','Discard draft & reload','Import sharing profile','Export sharing profile','Check inbox access'])assert.equal(f.button(label).disabled,true);
 const before=f.calls.length;await f.button('Discard draft & reload').onclick();assert.equal(f.calls.length,before);
 pending.resolve({config:{...f.config(),handle:'sam',revision:3,contentHash:'b'.repeat(64)}});await save;
 assert.equal(f.input('Receiving inbox name').disabled,false);assert.equal(f.input('Receiving inbox name').value,'sam');assert.match(f.status(),/configuration saved/);assert.equal(f.state().baseline.revision,3);
});

test('Settings sharing Save only clears the exact submitted draft and retains stale-CAS errors',{timeout:1000},async()=>{
 const f=sharingSettingsFixture();await f.render();f.edit('Receiving inbox name','sam');const pending=deferred();f.request(()=>pending.promise);
 const save=f.button('Save sharing configuration').onclick();
 // Simulate a newer feature draft arriving independently of the disabled DOM.
 f.state().draft={...f.state().draft,handle:'later'};
 pending.resolve({config:{...f.config(),handle:'sam',revision:3,contentHash:'b'.repeat(64)}});await save;
 assert.equal(f.input('Receiving inbox name').value,'later');assert.match(f.status(),/newer draft is still unsaved/);
 f.request(async()=>{throw new Error('Sharing settings changed. Reload them before saving.');});await f.button('Save sharing configuration').onclick();assert.equal(f.input('Receiving inbox name').value,'later');assert.match(f.status(),/settings changed/);assert.equal(f.state().baseline.revision,3);
});

test('a destination-change retry error stays visible without silently starting a new send',async()=>{
 const fs=require('node:fs'),vm=require('node:vm'),calls=[],messages=[];
 const source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8');
 const local={draftId:'draft',sendId:'failed-send',recipient:'eddie',message:'Review',revision:0,sending:false};
 const c=vm.createContext({chatDraft:()=>local,normalizeShareHandle,currentJob:()=>({status:'failed'}),$:()=>({}),call:async(path,payload)=>{calls.push({path,payload});throw new Error('Sharing destination changed. Prepare a new share instead of retrying this send.');},message:(node,text,error)=>messages.push({text,error}),paintSend(){},paint(){}});
 vm.runInContext(source.slice(source.indexOf('  async function sendDraft('),source.indexOf('  async function importEntry(')),c);
 await c.sendDraft();assert.equal(calls.length,1);assert.equal(calls[0].payload.retryId,'failed-send');assert.match(local.error,/destination changed.*Prepare a new share/);assert.equal(local.sending,false);assert.equal(local.sendId,'failed-send');
});

test('sharing buttons invoke named commands and preserve visible state on failure',async()=>{
 const fs=require('node:fs'),vm=require('node:vm'),source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8'),requests=[];
 const initial={config:{handle:'fixture'},jobs:[]},result={config:{handle:'fixture'},jobId:'prepared',jobs:[]};let reject=false,paints=0;
 const c=vm.createContext({data:initial,lastFetch:0,Date,paint(){paints++;},api:async(path,payload)=>{requests.push({path,payload});if(reject)throw new Error('Changed destination');return result;}});
 vm.runInContext(source.slice(source.indexOf('function sharingCommand('),source.indexOf('function normalizeShareHandle(')),c);
 vm.runInContext(source.slice(source.indexOf('  async function call('),source.indexOf('  async function load(')),c);
 const args={recordId:'fixture',recentConversation:false};
 assert.equal(await c.call('sharing.prepare',args),result);assert.equal(c.data,result);assert.equal(paints,1);assert.ok(c.lastFetch>0);
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0])),{path:'/api/commands/invoke',payload:{name:'sharing.prepare',args}});
 reject=true;await assert.rejects(c.call('sharing.send',{draftId:'fixture'}),/Changed destination/);assert.equal(c.data,result);assert.equal(paints,1);
});

test('ready sharing previews load once through the same command transport',async()=>{
 const fs=require('node:fs'),vm=require('node:vm'),source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8'),requests=[],details=new Map(),loading=new Set();
 const basic={id:'prepared',status:'ready',kind:'prepare'},detail={...basic,preview:{title:'Fixture handover'}};let paints=0;
 const c=vm.createContext({data:{jobs:[basic]},jobDetails:details,detailsLoading:loading,lastRendered:'cached',paint(){paints++;},message(){},$:()=>({}),api:async(path,payload)=>{requests.push({path,payload});return detail;}});
 vm.runInContext(source.slice(source.indexOf('function sharingCommand('),source.indexOf('function normalizeShareHandle(')),c);
 vm.runInContext(source.slice(source.indexOf('  function currentJob('),source.indexOf('  function paintSend(')),c);
 c.currentJob('prepared');c.currentJob('prepared');assert.equal(requests.length,1);
 await new Promise(resolve=>setImmediate(resolve));
 assert.deepEqual(JSON.parse(JSON.stringify(requests[0])),{path:'/api/commands/invoke',payload:{name:'sharing.job',args:{id:'prepared'}}});assert.equal(c.currentJob('prepared').preview,detail.preview);assert.equal(loading.size,0);assert.equal(paints,1);assert.equal(requests.length,1);
});

test('initial sharing command waits for the app state bootstrap',async()=>{
 const fs=require('node:fs'),vm=require('node:vm'),source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8'),requests=[];let paints=0;
 const c=vm.createContext({state:null,data:null,loading:false,lastFetch:0,Date,menu:{open:false},$:()=>null,message(){},paint(){paints++;},sharingCommand:async(name,args)=>{requests.push(name);return {entries:[]};}});
 vm.runInContext(source.slice(source.indexOf('  async function load('),source.indexOf('  function field(')),c);
 await c.load();assert.equal(requests.length,0);assert.equal(c.loading,false);
 c.state={};await c.load();assert.deepEqual(requests,['sharing.state']);assert.equal(paints,1);assert.equal(c.loading,false);
});

test('first sharing load builds from its fetched snapshot while later polls preserve the form',async()=>{
 const fs=require('node:fs'),vm=require('node:vm'),source=fs.readFileSync(require.resolve('../static/sharing.js'),'utf8');let builds=0,paints=0;
 const fetched={config:{revision:4},entries:[]};
 const c=vm.createContext({state:{},data:null,loading:false,lastFetch:0,Date,menu:{open:true},$:()=>null,message(){},paint(){paints++;},build(){assert.equal(c.data,fetched);builds++;},sharingCommand:async()=>fetched});
 vm.runInContext(source.slice(source.indexOf('  async function load('),source.indexOf('  function field(')),c);
 await c.load();assert.equal(builds,1);assert.equal(paints,0);
 await c.load();assert.equal(builds,1);assert.equal(paints,1);
});
