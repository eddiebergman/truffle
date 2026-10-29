const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/github-actions.js','utf8');
function fixture(extra={}){
 let clock=100000;const calls=[],records=[{id:'one',items:[{id:'pr-one'}]},{id:'two',items:[{id:'pr-two'}]}];
 const c=vm.createContext({chatDispatchRecovery:()=>({}),chatDispatchBlocked:()=>false,queueMicrotask(){},selectedAgentKey:'one',Date:class extends Date{static now(){return clock;}},document:{hidden:false,querySelectorAll:()=>[]},source:'agents',selectedAgentKey:'one',state:{sessions:records,sources:{github:{}}},liveCandidates:()=>records,candidateKey:r=>r.id,recordForCandidate:r=>r,agentWork:r=>r.items.map(i=>({...i,source:'github',kind:'Pull request'})),allItems:()=>[],workReference:item=>item.id,ciInfo:()=>({label:'Checks running'}),relative:()=> 'just now',api:async(route,payload)=>{calls.push({route,payload});return {status:'checked',itemIds:['pr-'+payload.args.recordId],checkedAt:clock/1000};},loadStateSnapshot:async()=>({accepted:true}),render:()=>{},...extra});
 vm.runInContext(source.slice(source.indexOf('const activeChatCI='),source.indexOf("if(typeof window!==")),c);
 return {c,calls,advance:n=>clock+=n};
}
test('focused CI polling runs on switch and every 30 seconds, pauses when hidden or outside chats',async()=>{
 const {c,calls,advance}=fixture();await c.syncActiveChatCI();assert.equal(calls.length,1);assert.equal(calls[0].route,'/api/commands/invoke');assert.equal(calls[0].payload.args.recordId,'one');
 advance(29999);await c.syncActiveChatCI();assert.equal(calls.length,1);advance(1);await c.syncActiveChatCI();assert.equal(calls.length,2);
 c.selectedAgentKey='two';await c.syncActiveChatCI();assert.equal(calls.at(-1).payload.args.recordId,'two');
 c.document.hidden=true;advance(30000);await c.syncActiveChatCI();assert.equal(calls.length,3);
 c.document.hidden=false;await c.syncActiveChatCI();assert.equal(calls.length,4);
 c.source='github';advance(30000);await c.syncActiveChatCI();assert.equal(calls.length,4);
});
test('a switch during an in-flight request refreshes the new chat without overlapping requests',async()=>{
 let release,active=0,maxActive=0;const gate=new Promise(resolve=>release=resolve),ids=[];
 const {c}=fixture({api:async(route,p)=>{ids.push(p.args.recordId);maxActive=Math.max(maxActive,++active);if(p.args.recordId==='one')await gate;active--;return {status:'checked',itemIds:[],checkedAt:100};}});
 const first=c.syncActiveChatCI();c.selectedAgentKey='two';await c.syncActiveChatCI();assert.deepEqual(ids,['one']);release();await first;await Promise.resolve();assert.deepEqual(ids,['one','two']);assert.equal(maxActive,1);
});
test('failed refresh keeps cached CI and exposes the error in its tooltip; a later success clears it',async()=>{
 let fail=true;const {c,advance}=fixture({api:async()=>{if(fail)throw new Error('Unavailable');return {status:'checked',itemIds:['pr-one'],checkedAt:130};}});
 await c.syncActiveChatCI();assert.match(c.chatCIHint({id:'pr-one'}),/showing cached checks/);fail=false;advance(30000);await c.syncActiveChatCI();assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/failed/);assert.match(c.chatCIHint({id:'pr-one'}),/Last checked just now/);
});

test('disabled and failed-source backoff defer automatic requests while a healthy sync interval does not',async()=>{
 const {c,calls,advance}=fixture();c.state.sources.github={disabled:true};await c.syncActiveChatCI();advance(60000);await c.syncActiveChatCI();assert.equal(calls.length,0);assert.match(c.chatCIHint({id:'pr-one'}),/disabled.*cached checks/);
 c.state.sources.github={failures:1,nextAttempt:200};await c.syncActiveChatCI();advance(39000);await c.syncActiveChatCI();assert.equal(calls.length,0);assert.match(c.chatCIHint({id:'pr-one'}),/paused.*cached checks/);
 c.state.sources.github={failures:0,nextAttempt:9999};await c.syncActiveChatCI();assert.equal(calls.length,1);assert.equal(calls[0].payload.name,'chats.ci.refresh');
});
test('skip receipts retain old freshness, avoid state reads, and honor the backend retry time',async()=>{
 let reads=0;const {c,calls,advance}=fixture({loadStateSnapshot:async()=>{reads++;return {accepted:true};}});await c.syncActiveChatCI();assert.equal(reads,1);
 c.api=async(route,payload)=>{calls.push({route,payload});return {status:'skipped',reason:'ci-backoff',itemIds:['pr-one'],checkedAt:null,retryAt:230};};advance(30000);await c.syncActiveChatCI();assert.equal(reads,1);assert.match(c.chatCIHint({id:'pr-one'}),/Last checked.*paused/);assert.equal(vm.runInContext("chatCIChecks.get('pr-one').at",c),100);
 advance(99999);await c.syncActiveChatCI();assert.equal(calls.length,2);advance(1);await c.syncActiveChatCI();assert.equal(calls.length,3);assert.equal(reads,1);
 for(const reason of ['cooldown','already-running','no-linked-prs','demo','disabled','backoff','state-changed']){c.api=async()=>({status:'skipped',reason,itemIds:['pr-one'],checkedAt:null});advance(30000);await c.syncActiveChatCI();assert.equal(reads,1);assert.equal(vm.runInContext("chatCIChecks.get('pr-one').at",c),100);assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/refresh failed/);}
});
test('a checked receipt becomes visible freshness only after the guarded snapshot is accepted',async()=>{
 let release;const gate=new Promise(resolve=>release=resolve),seen=[];const {c}=fixture({loadStateSnapshot:async options=>{seen.push(options);await gate;return {accepted:true};}});
 const pending=c.syncActiveChatCI();await Promise.resolve();assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/Last checked/);release();await pending;assert.match(c.chatCIHint({id:'pr-one'}),/Last checked just now/);assert.equal(seen[0].attempts,2);
});
test('accepted checks with failed or declined presentation recover using reads only and retain previous visible age',async()=>{
 for(const mode of ['throw','decline']){
  let reads=0;const {c,calls,advance}=fixture();await c.syncActiveChatCI();advance(30000);c.loadStateSnapshot=async()=>{reads++;if(mode==='throw')throw Error('private-read-error');return {accepted:false};};await c.syncActiveChatCI();assert.equal(calls.length,2);assert.equal(vm.runInContext("chatCIChecks.get('pr-one').at",c),100);assert.match(c.chatCIHint({id:'pr-one'}),/Checks refreshed on server; showing last loaded checks/);assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/private-read-error/);
  advance(30000);await c.syncActiveChatCI();assert.equal(calls.length,2);assert.equal(reads,2);
  c.state.sources.github={disabled:true};c.loadStateSnapshot=async()=>({accepted:true});advance(30000);await c.syncActiveChatCI();assert.equal(calls.length,2);assert.equal(vm.runInContext("chatCIChecks.get('pr-one').at",c),130);assert.equal(vm.runInContext('activeChatCI.presentation',c),null);
 }
});
test('a response after the selected chat or link set changes cannot repaint the replacement context',async()=>{
 for(const change of [c=>{c.selectedAgentKey='two';},c=>{c.state.sessions[0].items.push({id:'new-link'});},c=>{c.source='github';}]){
  let release,paints=0;const gate=new Promise(resolve=>release=resolve),{c}=fixture({loadStateSnapshot:async()=>{await gate;return {accepted:true};},render:()=>paints++});const pending=c.syncActiveChatCI();await Promise.resolve();change(c);release();await pending;assert.equal(paints,0);
 }
});
test('CI badge accessible text includes paused cache status both at creation and after an asynchronous receipt',async()=>{
 const item={id:'pr-one',ci:{state:'passed'}},badge={dataset:{chatCi:'pr-one'},setAttribute(name,value){this[name]=value;}},children=[];
 const {c}=fixture({document:{hidden:false,querySelectorAll:()=>[badge]},allItems:()=>[],el:()=>({append(...nodes){children.push(...nodes);}}),featureButton:()=>badge});c.state.sources.github={disabled:true};
 const workspace=fs.readFileSync('static/workspace.js','utf8');vm.runInContext(workspace.slice(workspace.indexOf('function linkedWorkCI('),workspace.indexOf('function renderAgentWorkspace(')),c);c.linkedWorkCI({},item);assert.match(badge['aria-label'],/disabled.*cached checks/);
 c.state.sources.github={};c.api=async()=>({status:'skipped',reason:'ci-backoff',itemIds:['pr-one'],checkedAt:null,retryAt:300});await c.syncActiveChatCI();assert.match(badge['aria-label'],/paused.*cached checks/);assert.match(badge.title,/paused.*cached checks/);
});

 test('partial checked receipts only freshen accepted IDs and discarded rows retain their visible age',async()=>{
 const {c,advance}=fixture();await c.syncActiveChatCI();advance(30000);c.api=async()=>({status:'checked',itemIds:['accepted'],skippedItemIds:['pr-one'],checkedAt:130});await c.syncActiveChatCI();
 assert.equal(vm.runInContext("chatCIChecks.get('accepted').at",c),130);assert.equal(vm.runInContext("chatCIChecks.get('pr-one').at",c),100);assert.match(c.chatCIHint({id:'pr-one'}),/Checks changed during refresh/);
 });

test('a changed source policy clears obsolete source retry and hints while unchanged snapshots preserve the receipt',async()=>{
 let requests=0;const {c,advance}=fixture({api:async()=>++requests===1?{status:'skipped',reason:'backoff',itemIds:['pr-one'],checkedAt:null,retryAt:1000}:{status:'checked',itemIds:['pr-one'],checkedAt:160}});
 await c.syncActiveChatCI();advance(60000);await c.syncActiveChatCI();assert.equal(requests,1);assert.match(c.chatCIHint({id:'pr-one'}),/paused/);
 c.state.sources.github={failures:2,nextAttempt:1000};await c.syncActiveChatCI();assert.equal(requests,1);
 c.state.sources.github={failures:0,nextAttempt:9999};await c.syncActiveChatCI();assert.equal(requests,2);assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/paused/);assert.equal(vm.runInContext('activeChatCI.retryAt',c),0);
});
test('source policy changes leave independent CI backoff intact and healthy full-sync intervals do not restart cadence',async()=>{
 let requests=0;const {c,advance}=fixture({api:async()=>{requests++;return {status:'skipped',reason:'ci-backoff',itemIds:['pr-one'],checkedAt:null,retryAt:1000};}});
 await c.syncActiveChatCI();c.state.sources.github={disabled:true};await c.syncActiveChatCI();c.state.sources.github={disabled:false};await c.syncActiveChatCI();assert.equal(requests,1);assert.equal(vm.runInContext('activeChatCI.retryAt',c),1000);assert.match(c.chatCIHint({id:'pr-one'}),/paused/);
 const other=fixture();await other.c.syncActiveChatCI();other.c.state.sources.github={failures:0,nextAttempt:99999};other.advance(1000);await other.c.syncActiveChatCI();assert.equal(other.calls.length,1);
});
test('an old source skip callback cannot reinstall retry after a policy transition, even when policy returns to ready',async()=>{
 let resolve,requests=0;const gate=new Promise(yes=>resolve=yes),{c}=fixture({api:async()=>{requests++;return requests===1?gate:{status:'checked',itemIds:['pr-one'],checkedAt:100};}});
 const pending=c.syncActiveChatCI();c.state.sources.github={disabled:true};await c.syncActiveChatCI();c.state.sources.github={disabled:false};await c.syncActiveChatCI();resolve({status:'skipped',reason:'backoff',itemIds:['pr-one'],checkedAt:null,retryAt:1000});await pending;
 assert.equal(vm.runInContext('activeChatCI.retryAt',c),0);assert.doesNotMatch(c.chatCIHint({id:'pr-one'}),/paused/);await c.syncActiveChatCI();assert.equal(requests,2);
});
