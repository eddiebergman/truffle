const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8');
const workspace=fs.readFileSync('static/workspace.js','utf8'),newsletters=fs.readFileSync('static/newsletters.js','utf8');
function helpers(globals={}){const ctx=vm.createContext(globals);vm.runInContext(newsletters.slice(newsletters.indexOf('function isSkillOutput('),newsletters.indexOf('function artifactLink(')),ctx);vm.runInContext(workspace.slice(workspace.indexOf('function skillOutputs('),workspace.indexOf('function renderSkills(')),ctx);return ctx;}
const output={id:'report',presetId:'daily',role:'skill-output',at:20};
const attachments=[{id:'ledger',sessionIds:['run'],sourcePath:'/reports/ledger.md',at:30},{id:'permissions',presetId:'daily',role:'attachment',at:30},{id:'other',presetId:'other',role:'skill-output',at:20}];
test('skill results exclude supporting attachments even in the same chat and folder',()=>{
 const ctx=helpers(),artifacts=[output,...attachments],before=JSON.stringify(artifacts);
 assert.equal(ctx.skillOutputs({id:'daily'},artifacts).length,1);assert.equal(ctx.skillOutputs({id:'daily'},artifacts)[0].id,'report');assert.equal(JSON.stringify(artifacts),before);
 for(const artifact of [null,{role:'skill-output'},{role:'skill-output',presetId:''},{role:'attachment',presetId:'daily',title:'audit.html'}])assert.equal(ctx.isSkillOutput(artifact),false);
});
test('Skills badge counts only unread declared outputs while preserving failed and skipped run notices',()=>{
 const badge={},ctx=helpers({state:{skillsReadAt:10,artifacts:[output,...attachments.slice(0,2)],skillRuns:[]},$:()=>badge});
 const start=workspace.indexOf('  const unread=skillInboxCounts(state);'),end=workspace.indexOf('\n}',start),paint=workspace.slice(start,end);
 vm.runInContext(paint,ctx);assert.equal(badge.textContent,1);assert.equal(badge.hidden,false);
 ctx.state.skillRuns=[{at:25,outcome:'failed'},{at:26,outcome:'skipped'},{at:40,outcome:'succeeded'},{at:10,outcome:'failed'}];
 assert.equal(ctx.skillInboxCounts(ctx.state).outputs,1);assert.equal(ctx.skillInboxCounts(ctx.state).notices,2);assert.equal(ctx.skillInboxCounts(ctx.state).total,3);
 ctx.state.skillsReadAt=30;assert.equal(ctx.skillInboxCounts(ctx.state).total,0);
 // Re-evaluate the actual badge update in a fresh block, retaining the existing read timestamp semantics.
 vm.runInContext('{'+paint+'}',ctx);assert.equal(badge.hidden,true);assert.equal(badge.textContent,0);
 ctx.state.artifacts.push({id:'new-support',role:'attachment',presetId:'daily',at:100});assert.equal(ctx.skillInboxCounts(ctx.state).total,0);
});
function inboxFixture({disabled=false}={}){
 const calls=[],messages=[],button={disabled,attrs:{},getAttribute(key){return this.attrs[key]??null;},setAttribute(key,value){this.attrs[key]=value;},removeAttribute(key){delete this.attrs[key];}},other={disabled:true};let renders=0;
 const state={revision:'one',skillsReadAt:10,artifacts:[output,...attachments.slice(0,2)],sessions:[{id:'chat',title:'Before',hidden:false}]};
 const ctx=helpers({state,source:'all',$:()=>button,persist(){},render:()=>renders++,api:(path,args)=>new Promise((resolve,reject)=>calls.push({path,args,resolve,reject})),toast:message=>messages.push(message)});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),ctx);
 vm.runInContext(app.slice(app.indexOf('async function presentLocalWrite('),app.indexOf('function workStatusLabel(')),ctx);
 const start=workspace.indexOf("$('#skills-inbox').onclick="),end=workspace.indexOf('function parseCSV(',start);vm.runInContext(workspace.slice(start,end),ctx);
 return {ctx,calls,messages,button,other,get renders(){return renders;},click:()=>button.onclick()};
}
const tick=()=>new Promise(resolve=>setImmediate(resolve));
test('Skills inbox opens immediately, sends one marker POST, discards its snapshot and commits only guarded state',async()=>{
 const f=inboxFixture(),pending=f.click();assert.equal(f.ctx.source,'skills');assert.equal(f.renders,1);assert.equal(f.button.disabled,false);assert.equal(f.button.attrs['aria-busy'],'true');assert.equal(f.ctx.skillInboxCounts(f.ctx.state).total,1);f.ctx.source='notes';const duplicate=f.click();assert.equal(f.ctx.source,'skills');assert.equal(f.calls.length,1);await duplicate;assert.equal(f.calls[0].path,'/api/skills/read');assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].args)),{});
 f.ctx.state={...f.ctx.state,sessions:[{id:'chat',title:'Concurrent title',hidden:true}]};const latest=f.ctx.state;f.calls[0].resolve({...latest,revision:'stale-post',sessions:[],skillsReadAt:999});await tick();assert.equal(f.ctx.state.sessions[0].title,'Concurrent title');assert.equal(f.ctx.state.skillsReadAt,10);assert.equal(f.calls[1].path,'/api/state');assert.equal(f.ctx.skillInboxCounts(f.ctx.state).total,1);
 f.ctx.source='notes';f.calls[1].resolve({...latest,revision:'fresh',skillsReadAt:30});await pending;assert.equal(f.ctx.source,'notes');assert.equal(f.ctx.skillInboxCounts(f.ctx.state).total,0);assert.equal(f.ctx.state.sessions[0].hidden,true);assert.equal(f.button.disabled,false);assert.equal(f.other.disabled,true);assert.equal(f.calls.filter(c=>c.path==='/api/skills/read').length,1);assert.equal(f.renders,3);assert.equal(f.button.attrs['aria-busy'],undefined);assert.deepEqual(f.messages,[]);
});
test('Skills inbox fences pre-write GET on accepted and unknown outcomes without replay or late navigation',async()=>{
 for(const outcome of ['accepted','unknown']){
  const f=inboxFixture({disabled:true});f.button.attrs['aria-busy']='false';const old=f.ctx.loadStateSnapshot(),baseline=f.ctx.state,pending=f.click();assert.equal(f.calls[0].path,'/api/state');assert.equal(f.calls[1].path,'/api/skills/read');
  f.ctx.state={...f.ctx.state,sessions:[{id:'chat',title:'Newest',hidden:true}]};const latest=f.ctx.state;
  if(outcome==='accepted')f.calls[1].resolve({revision:'old-post',sessions:[]});else f.calls[1].reject(Object.assign(new Error('Marker unavailable'),{status:503}));await tick();assert.notEqual(f.ctx.state,latest);assert.equal(f.ctx.state.sessions[0].title,'Newest');f.calls[0].resolve({...baseline,skillsReadAt:0});assert.equal((await old).accepted,false);
  f.ctx.source='github';if(outcome==='accepted'){f.calls[2].resolve({...latest,revision:'fresh',skillsReadAt:30});}await pending;assert.equal(f.ctx.source,'github');assert.equal(f.button.disabled,true);assert.equal(f.button.attrs['aria-busy'],'false');assert.equal(f.calls.filter(c=>c.path==='/api/skills/read').length,1);if(outcome==='unknown'){assert.match(f.messages[0],/outcome is unknown.*Inspect/);assert.equal(f.calls.length,2);assert.equal(f.ctx.state.skillsReadAt,10);}
 }
});
test('Skills inbox two conflicting reads preserve later metadata and report accepted presentation failure',async()=>{
 const f=inboxFixture(),pending=f.click();f.calls[0].resolve({skillsReadAt:999});await tick();
 const metadata=f.ctx.mutateChatMetadata('chats.visibility',{id:'chat',hidden:true},{renderAfter:false});f.calls[2].resolve({chat:{id:'chat',hidden:true}});await metadata;
 f.calls[1].resolve({...f.ctx.state,revision:'stale-one',sessions:[{id:'chat',title:'Before',hidden:false}]});await tick();assert.equal(f.calls[3].path,'/api/state');assert.equal(f.ctx.state.sessions[0].hidden,true);
 const rename=f.ctx.mutateChatMetadata('chats.rename',{id:'chat',title:'Newest'},{renderAfter:false});f.calls[4].resolve({chat:{id:'chat',title:'Newest',updatedAt:2}});await rename;const latest=f.ctx.state;f.ctx.source='linear';f.calls[3].resolve({...latest,revision:'stale-two',sessions:[]});await pending;
 assert.equal(f.ctx.state,latest);assert.equal(f.ctx.state.sessions[0].title,'Newest');assert.equal(f.ctx.state.sessions[0].hidden,true);assert.equal(f.ctx.state.skillsReadAt,10);assert.equal(f.ctx.source,'linear');assert.equal(f.button.disabled,false);assert.equal(f.calls.filter(c=>c.path==='/api/state').length,2);assert.equal(f.calls.filter(c=>c.path==='/api/skills/read').length,1);assert.match(f.messages[0],/Skills inbox marked read.*could not refresh.*do not repeat/);assert.equal(f.renders,1);
});
test('Skills inbox reports rejected, unknown and accepted read/render failure honestly and releases busy state',async()=>{
 for(const outcome of ['rejected','unknown','read-failed','paint-failed']){
  const f=inboxFixture(),before=f.ctx.state,pending=f.click();if(outcome==='rejected'||outcome==='unknown')f.calls[0].reject(Object.assign(new Error('Marker request failed'),{status:outcome==='rejected'?409:503}));else{f.calls[0].resolve({skillsReadAt:999});await tick();if(outcome==='read-failed')f.calls[1].reject(new Error('Read failed'));else{f.ctx.render=()=>{throw new Error('Paint failed');};f.calls[1].resolve({...before,revision:'fresh',skillsReadAt:30});}}await pending;
  assert.equal(f.button.disabled,false);assert.equal(f.calls.filter(c=>c.path==='/api/skills/read').length,1);if(outcome==='rejected'){assert.equal(f.ctx.state,before);assert.equal(f.messages[0],'Marker request failed');}else if(outcome==='unknown'){assert.notEqual(f.ctx.state,before);assert.match(f.messages[0],/outcome is unknown/);}else assert.match(f.messages[0],/Skills inbox marked read.*could not refresh.*do not repeat/);
 }
});
class Node{constructor(tag,cls='',text=''){this.tagName=tag;this.className=cls;this.textContent=text;this.children=[];}append(...nodes){this.children.push(...nodes);}setAttribute(){}}
function libraryFixture(source){
 const rendered=[],library={presets:[{id:'daily',title:'Daily',command:'/daily',description:'Fixture',agent:'claude',cwd:'/project',available:true}],artifacts:[output,...attachments]},state={sessions:[{id:'run',title:'Daily run',newsletterId:'daily',createdAt:1}]};
 const ctx=helpers({source,artifactLibrary:library,libraryError:'',state,artifactSessionFilter:'',newsletterOpening:false,el:(...args)=>new Node(...args),featureButton:(label)=>new Node('button','',label),Option:function(label,value){this.label=label;this.value=value;},libraryRow:artifact=>{rendered.push(artifact.id);return new Node('article','',artifact.title);},agentName:()=> 'Claude',dated:()=> 'Today',loadArtifactLibrary:()=>{}});
 vm.runInContext(newsletters.slice(newsletters.indexOf('function renderNewsletterView('),newsletters.indexOf('let artifactSessionFilter=')),ctx);
 return {ctx,rendered,list:new Node('main'),controls:new Node('div'),library};
}
test('legacy skill result cards use explicit ownership instead of shared chat or preset inference',()=>{
 const f=libraryFixture('newsletters'),before=JSON.stringify(f.library.artifacts);f.ctx.renderNewsletterView(f.list,f.controls,'');assert.deepEqual(f.rendered,['report']);assert.equal(JSON.stringify(f.library.artifacts),before);
});
test('generic artifact library continues to expose supporting attachments and all registered outputs',()=>{
 const f=libraryFixture('artifacts');f.ctx.renderNewsletterView(f.list,f.controls,'');assert.deepEqual(f.rendered,['report','ledger','permissions','other']);
});

test('Run skill uses the named command receipt and selects its exact chat without claiming completion',async()=>{
 const calls=[],selected=[],messages=[],button={disabled:false},record={id:'exact-chat'},fresh={sessions:[{id:'unrelated'},record]};
 const c=vm.createContext({source:'skills',selectedAgentKey:'',state:{sessions:[]},api:async(path,args)=>{calls.push({path,args});return path==='/api/commands/invoke'?{presetId:'fixture',recordId:'exact-chat',runId:'exact-run',status:'preview'}:fresh;},loadInventory:async()=>calls.push({path:'inventory'}),candidateForRecord:record=>({recordId:record.id}),selectAgent:(record,candidate)=>selected.push([record.id,candidate?.recordId||null]),toast:message=>messages.push(message)});
 vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);
 vm.runInContext(workspace.slice(workspace.indexOf('async function runSkill('),workspace.indexOf('function skillOutputs(')),c);
 await c.runSkill({id:'fixture'},button);assert.equal(button.disabled,false);assert.deepEqual(JSON.parse(JSON.stringify(calls[0])),{path:'/api/commands/invoke',args:{name:'skills.run',args:{presetId:'fixture'}}});assert.equal(calls[1].path,'/api/state');assert.deepEqual(selected,[['exact-chat',null]]);assert.match(messages[0],/Preview.*No agent was started/);
 c.api=async()=>{throw new Error('Launch failed');};await c.runSkill({id:'fixture'},button);assert.equal(button.disabled,false);assert.equal(selected.length,1);assert.equal(messages.at(-1),'Launch failed');
});

test('Run skill retries stale state reads without losing metadata or claiming a missing chat',async()=>{
 for(const exhausted of [false,true]){
  const reads=[],selected=[],messages=[],button={disabled:false};let commandCalls=0,inventoryCalls=0;
  const c=vm.createContext({source:'skills',selectedAgentKey:'',state:{revision:'fixture:1',sessions:[{id:'existing',title:'Before',hidden:false}]},api:async(path,payload)=>{
   if(path==='/api/state'){let resolve;const promise=new Promise(yes=>resolve=yes);reads.push({resolve});return promise;}
   if(payload.name==='skills.run'){commandCalls++;return {presetId:'fixture',recordId:'exact-chat',runId:'run',status:'launched'};}
   return {chat:{id:'existing',title:payload.args.title||'Before',hidden:payload.args.hidden??true,updatedAt:2}};
  },loadInventory:async()=>{inventoryCalls++;return {candidates:[{recordId:'exact-chat',host:'embedded',live:true}]};},candidateForRecord:record=>({recordId:record.id}),selectAgent:record=>selected.push(record.id),render(){},toast:message=>messages.push(message)});
  vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);
  vm.runInContext(app.slice(app.indexOf('async function mutateChatMetadata('),app.indexOf('function workStatusLabel(')),c);
  vm.runInContext(workspace.slice(workspace.indexOf('async function runSkill('),workspace.indexOf('function skillOutputs(')),c);
  const pending=c.runSkill({id:'fixture'},button);await new Promise(resolve=>setImmediate(resolve));
  await c.mutateChatMetadata('chats.visibility',{id:'existing',hidden:true});
  reads[0].resolve({revision:'fixture:1',sessions:[{id:'existing',title:'Before',hidden:false}]});await new Promise(resolve=>setImmediate(resolve));
  assert.equal(reads.length,2);assert.equal(c.state.sessions[0].hidden,true);
  if(exhausted)await c.mutateChatMetadata('chats.rename',{id:'existing',title:'Newer'});
  reads[1].resolve({revision:'fixture:2',sessions:[{id:'existing',title:'Before',hidden:true},{id:'exact-chat'}]});await pending;
  assert.equal(commandCalls,1);assert.equal(reads.length,2);assert.equal(button.disabled,false);assert.equal(c.state.sessions[0].hidden,true);
  if(exhausted){assert.equal(c.state.sessions[0].title,'Newer');assert.equal(selected.length,0);assert.equal(inventoryCalls,0);assert.match(messages.at(-1),/request accepted for chat exact-chat.*Dashboard changed.*do not run the skill again/);assert.doesNotMatch(messages.at(-1),/unavailable|missing/);}
  else{assert.deepEqual(selected,['exact-chat']);assert.equal(inventoryCalls,1);}
 }
});

function libraryLoaderFixture(){
 const calls=[];const c=vm.createContext({Date,source:'all',selectedSession:null,api:()=>new Promise((resolve,reject)=>calls.push({resolve,reject})),render(){}});
 const source=fs.readFileSync('static/newsletters.js','utf8');vm.runInContext(source.slice(0,source.indexOf('function isSkillOutput(')),c);
 return {c,calls,load:force=>c.loadArtifactLibrary(force),library:()=>vm.runInContext('artifactLibrary',c),fetched:()=>vm.runInContext('libraryFetched',c)};
}
test('forced library invalidation during an older read joins one fresh followup and never commits old data',async()=>{
 const f=libraryLoaderFixture(),first=f.load(),forced=f.load(true),also=f.load(true);assert.equal(first,forced);assert.equal(forced,also);assert.equal(f.calls.length,1);
 f.calls[0].resolve({artifacts:[{id:'old'}]});for(let i=0;i<6;i++)await Promise.resolve();assert.equal(f.calls.length,2);assert.equal(f.library(),null);assert.equal(f.fetched(),0);
 f.calls[1].resolve({artifacts:[{id:'new'}]});assert.equal((await forced).accepted,true);assert.equal((await first).accepted,true);assert.equal(f.library().artifacts[0].id,'new');assert.equal((await f.load()).accepted,true);assert.equal(f.calls.length,2);
});
test('library refresh failure returns a failed read receipt without discarding accepted cache',async()=>{
 const f=libraryLoaderFixture();let pending=f.load();f.calls[0].resolve({artifacts:[{id:'kept'}]});await pending;
 pending=f.load(true);f.calls[1].reject(new Error('fixture read failure'));const result=await pending;assert.equal(result.accepted,false);assert.equal(result.error,'fixture read failure');assert.equal(f.library().artifacts[0].id,'kept');assert.equal((await f.load()).accepted,false);
});
test('library invalidation during shared followup is bounded and cannot mark obsolete catalog fresh',async()=>{
 const f=libraryLoaderFixture(),pending=f.load();f.load(true);f.calls[0].resolve({artifacts:[{id:'old'}]});for(let i=0;i<6;i++)await Promise.resolve();const latest=f.load(true);f.calls[1].resolve({artifacts:[{id:'also-old'}]});assert.equal((await pending).accepted,false);assert.equal((await latest).accepted,false);assert.equal(f.calls.length,2);assert.equal(f.library(),null);assert.equal(f.fetched(),0);assert.equal((await f.load()).accepted,false);assert.equal(f.calls.length,2);
});
