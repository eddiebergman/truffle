const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
test('file trees group directories without losing duplicate filenames or Git paths',()=>{
 const source=fs.readFileSync('static/git-diff.js','utf8'),c=vm.createContext({});vm.runInContext(source.slice(source.indexOf('function gitFileTree('),source.indexOf('function appendGitFileTree(')),c);
 const files=[{path:'apps/gapi/config.py'},{path:'apps/worker/config.py'},{path:'README.md'}],tree=c.gitFileTree(files);
 assert.equal(tree.files[0].path,'README.md');assert.equal(tree.folders.get('apps').folders.get('gapi').files[0].path,'apps/gapi/config.py');assert.equal(tree.folders.get('apps').folders.get('worker').files[0].path,'apps/worker/config.py');
});
test('scheduled snoozes become due without restoring tabs or waking agents',()=>{
 const source=fs.readFileSync('static/ux.js','utf8'),c=vm.createContext({});vm.runInContext(source.slice(source.indexOf('function snoozeDue('),source.indexOf('function snoozeScheduleControl(')),c);
 assert.equal(c.snoozeDue({until:Date.now()-1}),true);assert.equal(c.snoozeDue({until:Date.now()+60000}),false);assert.equal(c.snoozeDue({}),false);
 for(const name of ['tomorrow','three','monday']){const next=new Date(c.nextSnoozeTime(name));assert.ok(next.getTime()>Date.now());assert.equal(next.getHours(),9);if(name==='monday')assert.equal(next.getDay(),1);}
 assert.equal(c.nextSnoozeTime('none'),null);
});
test('worktree selection is restricted to linked paths and falls back when a checkout is detached',()=>{
 const source=fs.readFileSync('static/git-diff.js','utf8'),c=vm.createContext({gitDiffSelections:{agent:'/two'}});vm.runInContext(source.slice(source.indexOf('function gitDiffDirectories('),source.indexOf('function gitDirectorySelector(')),c);
 const record={id:'agent',cwd:'/one',worktrees:[{path:'/one',branch:'first'},{path:'/two',branch:'second'}]};assert.equal(c.gitDiffDirectories(record).length,2);assert.equal(c.selectedGitDirectory(record),'/two');record.worktrees.pop();assert.equal(c.selectedGitDirectory(record),'/one');
});

test('file markers keep human and agent counts separate and ignore other roots and files',()=>{
 const source=fs.readFileSync('static/git-diff.js','utf8');let marked=false;
 const markers={children:[],replaceChildren(){this.children=[];},append(node){this.children.push(node);}};
 const row={dataset:{reviewFile:'api.rs',reviewRoot:'/repo'},querySelector:()=>markers,classList:{toggle(name,value){marked=value;}}};
 const c=vm.createContext({el:(tag,className,text)=>({className,text,setAttribute(key,value){this[key]=value;}})});
 vm.runInContext(source.slice(source.indexOf('function paintReviewFileMarkers('),source.indexOf('function paintGitDiff(')),c);
 c.paintReviewFileMarkers(row,{comments:[{path:'api.rs',root:'/repo',author:'You'},{path:'api.rs',root:'/repo',author:'Codex · Correctness'},{path:'api.rs',root:'/repo',author:'Claude · Performance'},{path:'api.rs',root:'/other',author:'You'},{path:'other.rs',root:'/repo',author:'You'}]});
 assert.equal(marked,true);assert.equal(markers.children.length,2);assert.equal(markers.children[0]['aria-label'],'Your comments: 1');assert.equal(markers.children[1]['aria-label'],'Agent comments: 2');assert.match(markers.children[0].innerHTML,/<svg[\s\S]*<\/svg>1$/);assert.match(markers.children[1].innerHTML,/<svg[\s\S]*<\/svg>2$/);assert.ok(!markers.children.some(x=>/Me |AI /.test(x.innerHTML)));
 c.paintReviewFileMarkers(row,{comments:[]});assert.equal(marked,false);assert.equal(markers.children.length,0);
});

function detachFixture({refreshError=false,conflict=false,actualToast=false}={}){
 const calls=[],messages=[];let undo,revision='before',resources=[{resourceKey:'A'}];
 const c=vm.createContext({featureButton:(_label,onclick)=>({onclick,addEventListener(){},setAttribute(){}}),api:async(path,p)=>{
  calls.push(structuredClone(p));
  if(!p.context)return {context:{revision,resources:structuredClone(resources)}};
  if(p.context.revision!==revision){const error=new Error('changed');error.status=409;throw error;}
  resources=structuredClone(p.context.resources);revision=revision==='before'?'removed':'restored';return {id:p.id,revision};
 },loadStateSnapshot:async()=>{if(conflict){resources=[{resourceKey:'B'}];revision='concurrent';}if(refreshError)throw new Error('read failed');return {accepted:true};},render(){},toast:(message,action)=>{messages.push(message);if(action)undo=action;}});
 if(actualToast){const node={hidden:true,children:[],replaceChildren(...children){this.children=children;},append(child){this.children.push(child);}};Object.assign(c,{$:()=>node,document:{createTextNode:message=>({textContent:message})},el:(_tag,_class,label)=>({textContent:label}),clearTimeout(){},setTimeout(){return 1;},toastTimer:null});const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('function toast('),app.indexOf('const serviceClient=')),c);c.toastNode=node;}
 const source=fs.readFileSync('static/ux.js','utf8');vm.runInContext(source.slice(source.indexOf('function detachLinkButton('),source.indexOf('function toggleSidebar(')),c);
 return {button:c.detachLinkButton({id:'record'},'resources','A'),calls,messages,undo:()=>undo(),get resources(){return resources;},toastNode:c.toastNode};
}
test('detach Undo uses mutation receipt, never a later read baseline, and preserves concurrent links',async()=>{
 const f=detachFixture({conflict:true});await f.button.onclick();assert.equal(f.calls.length,2);await f.undo();
 assert.equal(f.calls[2].context.revision,'removed');assert.deepEqual(f.resources,[{resourceKey:'B'}]);assert.match(f.messages.at(-1),/Undo was not applied.*newer links were kept/);
});
test('successful detach Undo restores the removed link once without assigning a returned full snapshot',async()=>{
 const f=detachFixture();await f.button.onclick();await f.undo();await f.undo();assert.equal(f.calls.length,3);assert.deepEqual(f.resources,[{resourceKey:'A'}]);
});
test('detach refresh failure retains exact Undo receipt and accepted Undo is not replayed after another read failure',async()=>{
 const f=detachFixture({refreshError:true});await f.button.onclick();assert.match(f.messages.at(-1),/removed.*refresh failed/);await f.undo();await f.undo();assert.equal(f.calls.length,3);assert.equal(f.calls[2].context.revision,'removed');assert.deepEqual(f.resources,[{resourceKey:'A'}]);assert.match(f.messages.at(-1),/Link restored.*Do not repeat/);
});

test('actual toast Undo wrapper keeps replacement conflict and refresh-failure messages visible',async()=>{
 for(const options of [{conflict:true},{refreshError:true}]){
  const f=detachFixture({...options,actualToast:true});await f.button.onclick();const undo=f.toastNode.children.find(node=>node.textContent==='Undo');assert.ok(undo);await undo.onclick();assert.equal(f.toastNode.hidden,false);assert.match(f.toastNode.children[0].textContent,options.conflict?/Undo was not applied/:/Link restored.*Do not repeat/);
 }
});
test('artifact detach and Undo refresh the actual library used by the result projection',async()=>{
 let revision='before',linked=true,libraryReads=0,undo,rendered=[];const record={id:'record'};
 const c=vm.createContext({Date,source:'agents',selectedSession:null,state:{artifacts:[{id:'artifact',sessionIds:['record']}]},record,featureButton:(_label,onclick)=>({onclick,addEventListener(){},setAttribute(){}}),api:async(path,p)=>{
  if(path==='/api/library'){libraryReads++;return {artifacts:[{id:'artifact',sessionIds:linked?['record']:[]}]};}
  if(!p.context)return {context:{revision,artifactIds:linked?['artifact']:[]}};
  assert.equal(p.context.revision,revision);linked=p.context.artifactIds.includes('artifact');revision=linked?'restored':'removed';return {id:p.id,revision};
 },loadStateSnapshot:async()=>({accepted:true}),render(){rendered=vm.runInContext('(()=>{'+projection+';return outputs.map(row=>row.id)})()',c);},toast:(_message,action)=>{if(action)undo=action;}});
 const workspace=fs.readFileSync('static/workspace.js','utf8'),projection=workspace.split('\n').find(line=>line.includes('const outputs=(artifactLibrary?.artifacts||state.artifacts||[])'));
 const source=fs.readFileSync('static/newsletters.js','utf8');vm.runInContext(source.slice(0,source.indexOf('function isSkillOutput(')),c);vm.runInContext("artifactLibrary={artifacts:[{id:'artifact',sessionIds:['record']}]}",c);
 const ux=fs.readFileSync('static/ux.js','utf8');vm.runInContext(ux.slice(ux.indexOf('function detachLinkButton('),ux.indexOf('function toggleSidebar(')),c);
 await c.detachLinkButton(record,'artifactIds','artifact').onclick();assert.deepEqual(Array.from(rendered),[]);assert.equal(libraryReads,1);await undo();assert.deepEqual(Array.from(rendered),['artifact']);assert.equal(libraryReads,2);
});
