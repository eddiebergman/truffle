const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
function fixture(){
  const nodes=new Map(),calls=[],timers=[],messages=[],events={};
  function node(id){if(!nodes.has(id))nodes.set(id,{dataset:{},setAttribute(k,v){this[k]=v;},querySelector(){return node(id+'label');}});return nodes.get(id);}
  const initial={runId:'',running:false};
  let reply=initial;
  const context={window:{addEventListener:(name,fn)=>events[name]=fn},$:node,api:async(path,payload)=>{calls.push({path,payload});if(reply instanceof Error)throw reply;return reply;},toast:m=>messages.push(m),setTimeout:fn=>{timers.push(fn);return timers.length;},clearTimeout:()=>{},Date};
  vm.runInNewContext(fs.readFileSync(require.resolve('../static/gcp-auth.js'),'utf8'),context);
  return {node,calls,timers,messages,events,setReply(v){reply=v;}};
}
const flush=()=>new Promise(resolve=>setImmediate(resolve));
test('opening the app checks status without starting authentication',async()=>{
  const f=fixture();await flush();assert.deepEqual(f.calls,[{path:'/api/gcp-auth',payload:undefined}]);assert.equal(f.node('#gcp-login-status').hidden,true);assert.equal(f.node('#gcp-login').disabled,false);
});
test('a service restart rechecks previously unavailable login routes without starting login',async()=>{
  const f=fixture();await flush();f.setReply(new Error('Not found'));f.events['truffle-service-restarted']();await flush();assert.equal(f.node('#gcp-login').disabled,true);
  f.setReply({runId:'',running:false});f.events['truffle-service-restarted']();await flush();assert.equal(f.node('#gcp-login').disabled,false);assert.equal(f.calls.filter(c=>c.path.endsWith('/start')).length,0);
});
test('explicit login starts once and restores independent CLI and ADC statuses',async()=>{
  const f=fixture();await flush();f.setReply({runId:'run',running:true,cli:{status:'running',message:'Use browser'},adc:{status:'running',message:'Use browser'}});
  await f.node('#gcp-login').onclick();await f.node('#gcp-login').onclick();assert.equal(f.calls.filter(c=>c.path.endsWith('/start')).length,1);assert.equal(f.node('#gcp-login').disabled,true);assert.equal(f.node('#gcp-login-cancel').hidden,false);
  f.setReply({runId:'run',running:false,cli:{status:'succeeded',message:'Signed in'},adc:{status:'failed',message:'ADC rejected'}});await f.timers.at(-1)();
  assert.equal(f.node('#gcp-login-cli').textContent,'CLI · Signed in');assert.equal(f.node('#gcp-login-adc').textContent,'ADC · Failed');assert.equal(f.node('#gcp-login-adc').title,'ADC rejected');assert.equal(f.node('#gcp-login').disabled,false);
});
