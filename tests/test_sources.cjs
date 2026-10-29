const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),source=app.slice(app.indexOf('async function refreshSources('),app.indexOf("$('#export').onclick"));
function fixture(){
 const buttons={'#sync':{disabled:false},'#force-refresh':{disabled:false}},calls=[],messages=[];let finish;
 const c=vm.createContext({$:id=>buttons[id],state:{old:true},api:async(path,payload)=>{calls.push({path,payload});if(path==='/api/commands/invoke')return new Promise(resolve=>finish=resolve);return {sources:{},syncing:[]};},render:()=>{calls.push({render:true});for(const button of Object.values(buttons))button.disabled=false;},toast:message=>messages.push(message)});vm.runInContext(app.slice(app.indexOf('function sameSnapshot('),app.indexOf('async function refresh()')),c);vm.runInContext(source,c);
 return {c,buttons,calls,messages,resolve:receipt=>finish(receipt)};
}
test('both source refresh buttons use the named owner and wait for its receipt before reloading state',async()=>{
 for(const [id,force] of [['#sync',false],['#force-refresh',true]]){
  const f=fixture(),pending=f.buttons[id].onclick();assert.equal(f.buttons[id].disabled,true);assert.deepEqual(JSON.parse(JSON.stringify(f.calls)),[{path:'/api/commands/invoke',payload:{name:'sources.refresh',args:{force}}}]);
  f.resolve({status:'started',message:'Refreshing github…'});await pending;assert.equal(f.calls[1].path,'/api/state');assert.deepEqual(f.calls[2],{render:true});assert.equal(f.buttons[id].disabled,false);assert.deepEqual(f.messages,['Refreshing github…']);assert.equal(f.c.state.old,undefined);
 }
});
test('skipped refresh explains owner outcome and does not claim completion; failed request does not navigate or replace state',async()=>{
 const f=fixture(),pending=f.buttons['#force-refresh'].onclick();f.resolve({status:'skipped',message:'Nothing started: waiting for retry window.'});await pending;assert.deepEqual(f.messages,['Nothing started: waiting for retry window.']);
 const original=f.c.state;f.c.api=async()=>{throw new Error('This action just started. Give it a moment.');};await f.buttons['#sync'].onclick();assert.equal(f.c.state,original);assert.equal(f.messages.at(-1),'This action just started. Give it a moment.');assert.equal(f.buttons['#sync'].disabled,false);
});

function deferredRead(){let resolve;const promise=new Promise(yes=>resolve=yes);return {promise,resolve};}
test('source refresh retries obsolete reads and preserves newer metadata, receipt and force cooldown',async()=>{
 for(const exhausted of [false,true]){
  const f=fixture(),reads=[],cooldown=Date.now()/1000+60;
  f.c.state={revision:'fixture:1',sessions:[{id:'chat',title:'Before',hidden:false}],sources:{},syncing:[]};
  vm.runInContext(app.slice(app.indexOf('async function mutateChatMetadata('),app.indexOf('function workStatusLabel(')),f.c);
  f.c.api=async(path,payload)=>{
   if(path==='/api/state'){const read=deferredRead();reads.push(read);return read.promise;}
   if(payload.name==='sources.refresh')return {message:'Owner accepted source refresh.',forceRefreshAfter:cooldown};
   return {chat:{id:'chat',title:payload.args.title||'Before',hidden:payload.args.hidden??true,updatedAt:2}};
  };
  f.c.render=()=>{f.buttons['#sync'].disabled=false;f.buttons['#force-refresh'].disabled=f.c.state.forceRefreshAfter>Date.now()/1000;};
  const pending=f.c.refreshSources(true,f.buttons['#force-refresh']);await new Promise(resolve=>setImmediate(resolve));
  assert.equal(reads.length,1);await f.c.mutateChatMetadata('chats.visibility',{id:'chat',hidden:true});
  reads[0].resolve({revision:'fixture:1',sessions:[{id:'chat',title:'Before',hidden:false}],sources:{},syncing:[]});await new Promise(resolve=>setImmediate(resolve));
  assert.equal(reads.length,2);assert.equal(f.c.state.sessions[0].hidden,true);
  if(exhausted)await f.c.mutateChatMetadata('chats.rename',{id:'chat',title:'Newer'});
  reads[1].resolve({revision:'fixture:2',sessions:[{id:'chat',title:'Before',hidden:true}],sources:{},syncing:[],forceRefreshAfter:cooldown});await pending;
  assert.equal(reads.length,2);assert.equal(f.c.state.sessions[0].hidden,true);assert.equal(f.c.state.sessions[0].title,exhausted?'Newer':'Before');
  assert.equal(f.c.state.forceRefreshAfter,cooldown);assert.equal(f.buttons['#force-refresh'].disabled,true);assert.match(f.messages.at(-1),/^Owner accepted source refresh\./);
  if(exhausted)assert.match(f.messages.at(-1),/Dashboard changed.*newer changes were kept/);
 }
});
test('source refresh keeps the successful owner receipt and cooldown when its state read fails',async()=>{
 const f=fixture(),cooldown=Date.now()/1000+60;f.c.api=async path=>{if(path==='/api/commands/invoke')return {message:'Owner accepted.',forceRefreshAfter:cooldown};throw new Error('Read unavailable');};
 await f.c.refreshSources(true,f.buttons['#force-refresh']);assert.equal(f.c.state.forceRefreshAfter,cooldown);assert.match(f.messages.at(-1),/^Owner accepted\. Dashboard could not refresh: Read unavailable/);
});
