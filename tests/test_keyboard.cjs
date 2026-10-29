const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8');
const workspace=fs.readFileSync('static/workspace.js','utf8');
function fuzzy(){const c=vm.createContext({});vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function render() {')),c);return c.fuzzyScore;}
test('fuzzy search accepts abbreviations and ranks contiguous words above scattered matches',()=>{
  const score=fuzzy();assert.ok(Number.isFinite(score('pymt svr','payments-server')));
  assert.ok(score('server','payments-server')>score('server','Some very early review request'));
  assert.equal(score('xyz','payments-server'),-Infinity);assert.equal(score('','anything'),0);
  assert.ok(Number.isFinite(score('eng 1211','Clean up ENG-1211')));
});
function keyboard(){
  const events={},nodes=new Map(),opened=[],moves=[];
  const node=()=>({dataset:{},append(){},setAttribute(){},classList:{toggle(){}},focus(){},select(){},closest(){return null;},scrollIntoView(){},click(){},remove(){}});
  const rows=['one','two','three'].map(id=>Object.assign(node(),{dataset:{workId:id}}));
  const $=id=>{if(!nodes.has(id))nodes.set(id,node());return nodes.get(id);};
  const search=$('#search');search.oninput=()=>{};search.closest=()=>search;
  const c=vm.createContext({$,navigator:{platform:'MacIntel'},terminalFocus:false,source:'github',githubView:'all',scopePrefs:{},document:{querySelectorAll:()=>rows,querySelector:()=>null,body:{append(link){opened.push(link.href);}},addEventListener(name,handler){events[name]=handler;}},el:node,allItems:()=>rows.map(row=>({id:row.dataset.workId,source:'github',url:'https://github.com/repo/pull/'+row.dataset.workId})),openItem:item=>moves.push(item.id),openNote:item=>moves.push(item.id),persist(){},render(){},liveCandidates:()=>[],beginAgentRename(){moves.push('rename');},openLocationPrompt(){moves.push('new');},toggleSidebar:side=>moves.push('toggle:'+side),cycleAgentTab:()=>false,runTabCommand:command=>moves.push('tab:'+command)});
  vm.runInContext(workspace.slice(workspace.indexOf('const sidebarKeys='),workspace.indexOf("document.addEventListener('click',event=>")),c);
  const key=(value,target=rows[0],flags={})=>{let consumed=false;events.keydown({key:value,target,isComposing:false,altKey:false,preventDefault(){consumed=true;},stopImmediatePropagation(){},...flags});return consumed;};
  return {c,key,search,rows,opened,moves};
}
test('work search arrows select rows; O opens the source and Enter opens details',()=>{
  const f=keyboard();assert.equal(f.key('/'),true);assert.equal(f.key('ArrowDown',f.search),true);f.key('ArrowDown');f.key('o');assert.deepEqual(f.opened,['https://github.com/repo/pull/two']);f.key('Enter');assert.deepEqual(f.moves,['two']);
});
test('typing o stays in the search query and terminal Control shortcuts are preserved',()=>{
  const f=keyboard();assert.equal(f.key('o',f.search),false);assert.equal(f.key('d',f.rows[0],{ctrlKey:true}),false);assert.equal(f.key('n',f.rows[0],{metaKey:true}),true);assert.deepEqual(f.moves,['new']);
  f.c.source='agents';assert.equal(f.key('/'),false);
});

test('new session exits terminal focus so its inline folder prompt stays visible',()=>{
  const f=keyboard();f.c.terminalFocus=true;f.key('n',f.rows[0],{metaKey:true});assert.equal(f.c.terminalFocus,false);assert.deepEqual(f.moves,['new']);
});

test('sidebar shortcuts use noun initials and preserve standard editing keys',()=>{
  const f=keyboard();f.key('g',f.rows[0],{metaKey:true});assert.equal(f.c.source,'github');
  f.key('p',f.rows[0],{metaKey:true,shiftKey:true});assert.equal(f.c.source,'notes');
  f.key('s',f.rows[0],{metaKey:true});assert.equal(f.c.source,'sentry');
  f.key('a',f.rows[0],{metaKey:true,shiftKey:true});assert.equal(f.c.source,'agents');
  f.key('l',f.rows[0],{metaKey:true,shiftKey:true});assert.equal(f.c.source,'linear');
  for(const key of ['a','c','v'])assert.equal(f.key(key,f.rows[0],{metaKey:true}),false);
});

test('browser tab keys and All work use the expected modifiers',()=>{
  const f=keyboard();f.key('i',f.rows[0],{metaKey:true});assert.equal(f.c.source,'all');
  f.key('t',f.rows[0],{metaKey:true});f.key('w',f.rows[0],{metaKey:true});f.key('t',f.rows[0],{metaKey:true,shiftKey:true});assert.deepEqual(f.moves,['tab:new','tab:close','tab:reopen']);
});

test('Command-R renames only from an agent chat and preserves the shifted reload shortcut',()=>{
  const f=keyboard();assert.equal(f.key('r',f.rows[0],{metaKey:true}),false);f.c.source='agents';assert.equal(f.key('r',f.rows[0],{metaKey:true}),true);assert.deepEqual(f.moves,['rename']);assert.equal(f.key('r',f.rows[0],{metaKey:true,shiftKey:true}),false);
});

test('Escape closes an open dialog before search controls or terminals consume it',()=>{
  const c=vm.createContext({Event,document:{querySelectorAll:()=>[]}});
  vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("document.addEventListener('keydown',dismissOpenDialog")),c);
  let closed=0,prevented=0,stopped=0;const dialog={dispatchEvent:()=>true,close(){closed++;}};
  const event={key:'Escape',target:{closest:()=>dialog},preventDefault(){prevented++;},stopImmediatePropagation(){stopped++;}};
  c.dismissOpenDialog(event);assert.equal(closed,1);assert.equal(prevented,1);assert.equal(stopped,1);
  c.dismissOpenDialog({...event,isComposing:true});assert.equal(closed,1);
  c.dismissOpenDialog({...event,target:{closest:()=>null}});assert.equal(prevented,1);
});
test('palette action arrows wrap while search caret keys keep their normal behavior',()=>{
  const features=fs.readFileSync('static/features.js','utf8');let selected=0,prevented=0,chosen=0;
  const actions=[0,1,2].map(index=>({focus(){selected=index;doc.activeElement=this;}}));
  const doc={activeElement:actions[0]},query={},footer={querySelectorAll:()=>actions};
  const c=vm.createContext({document:doc,$:id=>id==='#command-actions'?footer:query,choosePalette(){chosen++;},paletteEntries:[{}],paletteSelected:0});
  vm.runInContext(features.slice(features.indexOf('function handlePaletteKey('),features.indexOf("$('#command-query').oninput=")),c);
  const event=key=>({key,target:{closest:()=>footer},preventDefault(){prevented++;}});
  c.handlePaletteKey(event('ArrowLeft'));assert.equal(selected,2);c.handlePaletteKey(event('ArrowRight'));assert.equal(selected,0);
  c.handlePaletteKey({...event('ArrowRight'),target:{closest:()=>null}});assert.equal(prevented,2);
  c.handlePaletteKey({...event('Enter'),target:query});assert.equal(chosen,1);
});

test('Command-H and Command-L toggle the sidebars instead of navigating',()=>{const f=keyboard();f.key('h',f.rows[0],{metaKey:true});f.key('l',f.rows[0],{metaKey:true});assert.deepEqual(f.moves,['toggle:left','toggle:right']);assert.equal(f.c.source,'github');});


test('Escape dismisses a dialog child layer before closing the dialog itself',()=>{
 let closed=0,layers=1;const dialog={dispatchEvent:()=>true,dismissLayer(){if(layers){layers--;return true;}return false;},close(){closed++;}};
 const c=vm.createContext({Event,document:{querySelectorAll:()=>[dialog],addEventListener(){}}});
 vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("document.addEventListener('keydown',dismissOpenDialog,true);")),c);
 const event={key:'Escape',target:{closest:()=>dialog},preventDefault(){},stopImmediatePropagation(){}};c.dismissOpenDialog(event);assert.equal(closed,0);assert.equal(layers,0);c.dismissOpenDialog(event);assert.equal(closed,1);
});

function shortcutErrorFixture(){
 const settingsSource=fs.readFileSync('static/settings.js','utf8');
 class Node{
  constructor(tag,cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.children=[];this.attrs={};this.dataset={};this.hidden=false;this.classList={add:()=>{}};}
  append(...nodes){this.children.push(...nodes);for(const node of nodes)node.parentElement=this;}
  contains(node){return this===node||this.children.some(child=>child.contains?.(node));}
  closest(selector){for(let node=this;node;node=node.parentElement)if(selector==='[hidden]'&&node.hidden)return node;return null;}
  setAttribute(key,value){this.attrs[key]=value;}
  getAttribute(key){return this.attrs[key]||null;}
  removeAttribute(key){delete this.attrs[key];}
  addEventListener(){}
  setCustomValidity(value){this.validationMessage=value;}
  focus(){this.focused=true;}
 }
 let rows=[];const footer={'#settings-save':{},'#settings-status':{}},walk=node=>[node,...node.children.flatMap(walk)],all=()=>rows.flatMap(walk);
 const document={getElementById:id=>all().find(node=>node.id===id)||null,querySelectorAll:selector=>all().filter(node=>selector==='.shortcut-input'?node.className.split(' ').includes('shortcut-input'):selector==='[data-settings-reset]'?node.dataset.settingsReset:false)};
 const commands=[{id:'commands',label:'Commands'},{id:'search',label:'Search'}],defaults={keybindings:{commands:'Primary+K',search:'Primary+F'}},values=structuredClone(defaults);
 const c=vm.createContext({structuredClone,document,settingsBaseline:{values:structuredClone(values)},settingsDraft:structuredClone(values),settingsSaving:false,shortcutRecording:null,filterSettings(){},appSettings:{commands,defaults,values},$:id=>footer[id],el:(tag,cls,text)=>new Node(tag,cls,text),settingsButton:(label,onclick,cls)=>Object.assign(new Node('button',cls,label),{onclick}),paintSettingsBody:()=>{rows=commands.map(command=>c.shortcutInput(command));}});
 for(const [start,end]of [['function canonicalShortcut(','function recordedShortcut('],['function settingsReset(','function settingInput('],['function shortcutInput(','function settingsPanel(']])vm.runInContext(settingsSource.slice(settingsSource.indexOf(start),settingsSource.indexOf(end)),c);
 c.paintSettingsBody();c.updateSettingsDirty();return {c,document,footer,get rows(){return rows;},input:id=>document.getElementById('shortcut-'+id),error:id=>document.getElementById('shortcut-'+id+'-error')};
}
test('typed shortcut errors expose the exact persistent visible reason and retain existing descriptions',()=>{
 const f=shortcutErrorFixture(),input=f.input('search');input.setAttribute('aria-describedby','existing-help');input.value='Primary+K';input.oninput();const error=f.error('search');assert.ok(error,'Invalid shortcut needs visible inline error, not just a tooltip');assert.match(error.textContent,/Already assigned to Commands/);assert.equal(error.hidden,false);assert.equal(input.getAttribute('aria-invalid'),'true');assert.equal(input.getAttribute('aria-describedby'),'existing-help '+error.id);assert.equal(f.footer['#settings-save'].disabled,true);
 input.value='Primary+Alt+F';input.oninput();assert.equal(error.textContent,'');assert.equal(error.hidden,true);assert.equal(input.getAttribute('aria-invalid'),'false');assert.equal(input.getAttribute('aria-describedby'),'existing-help');assert.equal(input.title,'Type a shortcut or choose Record');assert.equal(f.footer['#settings-save'].disabled,false);
 input.value='Primary+Q';input.oninput();assert.match(error.textContent,/reserved/);input.value='Primary+F5';input.oninput();assert.match(error.textContent,/Use Primary/);
});
test('shortcut reset and settings rebuild remove stale errors and their description references',()=>{
 const f=shortcutErrorFixture();f.input('search').value='Primary+K';f.input('search').oninput();assert.equal(f.error('search')?.hidden,false);const reset=f.rows.flatMap(row=>row.children.flatMap(child=>child.children||[])).find(node=>node.dataset.settingsReset==='search');assert.ok(reset);reset.onclick();assert.equal(f.input('search').value,'Primary+F');assert.equal(f.error('search').hidden,true);assert.equal(f.error('search').textContent,'');assert.equal(f.input('search').getAttribute('aria-describedby'),null);
 f.input('search').value='Primary+Q';f.input('search').oninput();assert.equal(f.error('search').hidden,false);f.c.settingsDraft=structuredClone(f.c.appSettings.values);f.c.paintSettingsBody();f.c.updateSettingsDirty();assert.equal(f.error('search').hidden,true);assert.equal(f.error('search').textContent,'');assert.equal(f.input('search').getAttribute('aria-invalid'),'false');
});

test('configured page search consumes Cmd+F only when a visible contextual search can take focus',()=>{
 const settings=fs.readFileSync('static/settings.js','utf8'),events={},calls=[];
 const input={getClientRects:()=>[{}],focus:()=>calls.push('focus'),select:()=>calls.push('select')};let available=true,dialog=null;
 const c=vm.createContext({appSettings:{values:{keybindings:{'page-search':'Primary+F'}}},source:'agents',$:()=>({...input,getClientRects:()=>available?[{}]:[]}),document:{querySelectorAll:()=>dialog?[dialog]:[],querySelector:()=>null,addEventListener:(name,fn)=>events[name]=fn},chordFromEvent:()=> 'Primary+F',runConfiguredCommand:id=>calls.push(id)});
 vm.runInContext(settings.slice(settings.indexOf('function focusPageSearch('),settings.indexOf("document.addEventListener('keydown',event=>{if(event.key==='/'")),c);
 vm.runInContext(settings.slice(settings.indexOf("document.addEventListener('keydown',event=>{if(!appSettings"),settings.indexOf('function settingsButton(')),c);
 const press=()=>{let prevented=0,stopped=0;events.keydown({target:{closest:()=>null},preventDefault:()=>prevented++,stopImmediatePropagation:()=>stopped++});return {prevented,stopped};};
 assert.deepEqual(press(),{prevented:1,stopped:1});assert.deepEqual(calls,['focus','select']);calls.length=0;
 available=false;assert.deepEqual(press(),{prevented:0,stopped:0});assert.deepEqual(calls,[]);
 dialog={querySelector:()=>null};assert.deepEqual(press(),{prevented:0,stopped:0});
 dialog={querySelector:()=>input};assert.deepEqual(press(),{prevented:1,stopped:1});assert.deepEqual(calls,['focus','select']);
});
