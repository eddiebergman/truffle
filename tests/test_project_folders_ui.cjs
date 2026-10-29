const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/sessions.js','utf8'),tick=()=>new Promise(resolve=>setImmediate(resolve));
function fixture(){
 const nodes=new Map(),calls=[],pending=[],outside={name:'untouched focus'};
 const node=id=>{if(!nodes.has(id))nodes.set(id,{textContent:'',title:'',value:'claude',children:[],dataset:{},attrs:{},open:false,isConnected:true,setAttribute(key,value){this.attrs[key]=value;},append(...children){this.children.push(...children);},replaceChildren(...children){this.children=children;}});return nodes.get(id);};
 const c=vm.createContext({source:'notes',currentItem:null,state:{itemSettings:{},folderMappings:{},sessions:[]},inventory:{workRoot:'/work',folders:[],directories:[]},quickStarting:false,$:node,el:(tag,cls,text)=>Object.assign({tag,className:cls,textContent:text},node(Symbol(tag))),linkedSessions:()=>[],sessionCard(){},agentName:value=>value==='claude'?'Claude':'Codex',itemFolderContexts:item=>['item:'+item.id],api:async(path,args)=>{calls.push({path,...args});return new Promise((resolve,reject)=>pending.push({resolve,reject}));},document:{activeElement:outside},quickStart(){throw Error('Preview must never launch.');},changeProjectStatus(){},openNote(){},mutate(){throw Error('Preview must never mutate.');}});
 vm.runInContext(source.slice(source.indexOf('function folderHint('),source.indexOf('function renderFolderSelector(')),c);
 vm.runInContext(source.slice(source.indexOf('function itemSessionChip('),source.indexOf('function sessionCard(')),c);
 vm.runInContext(source.slice(source.indexOf('function renderItemSessions('),source.indexOf('function setRadio(')),c);
 const open=item=>{c.currentItem=item;c.renderItemSessions(item);node('#item-dialog').open=true;};
 return {c,calls,pending,node,outside,open,label:node('#item-folder-path'),dialog:node('#item-dialog')};
}
const project=(id='note:a',cwd='/configured/repo')=>({id,source:'notes',title:'Project',status:'todo',cwd});
test('compact project rows expose configured metadata without validating against inventory or starting requests',()=>{
 const f=fixture(),withPath=f.c.itemSessionChip(project()),withoutPath=f.c.itemSessionChip(project('note:b',''));assert.match(withPath.children[0].title,/Configured folder: \/configured\/repo/);assert.match(withoutPath.children[0].title,/Folder resolved when launching/);assert.equal(f.calls.length,0);assert.equal(f.c.folderHint(project()),'/configured/repo');
});
test('actual item sessions uses the authoritative canonical folder even when it is absent from inventory',async()=>{
 const f=fixture();f.open(project());assert.equal(f.label.textContent,'Folder: checking saved project folder…');assert.equal(f.label.attrs['aria-busy'],'true');assert.deepEqual(JSON.parse(JSON.stringify(f.calls)),[{path:'/api/commands/invoke',name:'projects.folder',args:{id:'note:a'}}]);f.pending[0].resolve({id:'note:a',cwd:'/outside/actual-worktree',requiresSelection:false});await tick();assert.equal(f.label.textContent,'Folder: /outside/actual-worktree');assert.equal(f.label.attrs.role,'status');assert.equal(f.label.attrs['aria-busy'],'false');assert.equal(f.c.document.activeElement,f.outside);assert.match(f.label.title,/Launch checks.*again/);
});
test('invalid configured paths may resolve a valid fallback while no automatic folder requires selection',async()=>{
 for(const cwd of ['/valid/fallback',null]){const f=fixture();f.open(project('note:a','/removed/configuration'));f.pending[0].resolve({id:'note:a',cwd,requiresSelection:cwd===null});await tick();assert.equal(f.label.textContent,cwd?'Folder: '+cwd:'Folder: choose when launching');}
});
test('read errors remain unknown and keep configured-folder context rather than asserting it was cleared',async()=>{
 const f=fixture();f.open(project());f.pending[0].reject(Error('Storage unavailable'));await tick();assert.equal(f.label.textContent,'Folder: could not verify. Configured folder: /configured/repo');assert.equal(f.label.attrs['aria-busy'],'false');assert.ok(!f.label.textContent.includes('choose when launching'));
});
test('late A folder response cannot overwrite project B or same-project close and reopen',async()=>{
 for(const same of [false,true]){const f=fixture(),a=project(),b=same?a:project('note:b');f.open(a);f.dialog.open=false;f.open(b);f.pending[1].resolve({id:b.id,cwd:'/new-preview',requiresSelection:false});await tick();f.pending[0].resolve({id:a.id,cwd:'/old-preview',requiresSelection:false});await tick();assert.equal(f.label.textContent,'Folder: /new-preview');assert.equal(f.label.attrs['aria-busy'],'false');}
});
test('closed or changed-source modal rejects pending folder reads without repainting or changing maps',async()=>{
 for(const close of [true,false]){const f=fixture(),before=JSON.stringify(f.c.state);f.open(project());const pendingText=f.label.textContent;if(close)f.dialog.open=false;else f.c.source='github';f.pending[0].resolve({id:'note:a',cwd:'/must-not-appear',requiresSelection:false});await tick();assert.equal(f.label.textContent,pendingText);assert.equal(JSON.stringify(f.c.state),before);assert.equal(f.c.document.activeElement,f.outside);}
});
test('project presentation rereads current owner data and only the latest read can paint the hint',async()=>{
 const f=fixture(),a=project();f.open(a);f.pending[0].resolve({id:a.id,cwd:'/first',requiresSelection:false});await tick();f.c.currentItem={...a,cwd:'/configured/new'};f.c.renderItemSessions(f.c.currentItem);assert.equal(f.calls.length,2);assert.equal(f.label.textContent,'Folder: checking saved project folder…');f.pending[1].resolve({id:a.id,cwd:'/canonical/new',requiresSelection:false});await tick();assert.equal(f.label.textContent,'Folder: /canonical/new');
});
test('non-project hints stay synchronous and invalidate older pending project previews',async()=>{
 const f=fixture();f.open(project());const remote={id:'github:pr',source:'github',title:'PR',status:'todo',project:'org/repo'};f.c.state.itemSettings[remote.id]={explicit:true,cwd:'/saved/github'};f.open(remote);assert.equal(f.label.textContent,'Folder: /saved/github');assert.equal(f.calls.length,1);assert.equal(f.label.attrs['aria-busy'],'false');f.pending[0].resolve({id:'note:a',cwd:'/old-project',requiresSelection:false});await tick();assert.equal(f.label.textContent,'Folder: /saved/github');
});
test('a mismatched or malformed owner response cannot become a validated folder claim',async()=>{
 for(const result of [{id:'note:other',cwd:'/wrong',requiresSelection:false},{id:'note:a',cwd:'/contradictory',requiresSelection:true},{id:'note:a',cwd:null,requiresSelection:false}]){const f=fixture();f.open(project());f.pending[0].resolve(result);await tick();assert.match(f.label.textContent,/could not verify/);assert.equal(f.label.attrs['aria-busy'],'false');}
});
