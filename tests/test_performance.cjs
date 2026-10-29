const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
function fixture(){
  let tick=0,calls=0;
  const listeners={};
  const context=vm.createContext({Map,Number,JSON,performance:{now:()=>++tick,mark(){},clearMeasures(){},measure(){},clearMarks(){}},
    renderWorkspacePage:()=>{calls++;return 42;},allItems:()=>[],buildBundleIndex:()=>{},renderRow:()=>({}),api:async()=>({ok:true}),
    source:'sentry',state:{items:[{title:'private title'}]},serviceClient:{isDesktop:false},
    document:{hidden:false,addEventListener:(name,fn)=>listeners[name]=fn,querySelectorAll:()=>[]},requestAnimationFrame:fn=>fn(),
    el:(tag,cls,text)=>({tag,cls,text,children:[],append(...nodes){this.children.push(...nodes);}}),featureButton:(title,fn)=>({title,fn})});
  const app=fs.readFileSync('static/app.js','utf8');vm.runInContext(app.slice(app.indexOf('const renderHooks='),app.indexOf('function renderWorkPage()')),context);
  vm.runInContext(fs.readFileSync('static/performance.js','utf8'),context);
  return {context,run:code=>vm.runInContext(code,context),calls:()=>calls,listeners};
}
test('profiling is off by default and preserves render return values',()=>{
  const f=fixture();assert.equal(f.run('render()'),42);assert.equal(f.calls(),1);
  assert.equal(f.run('profileSummary().length'),0);
});
test('numeric samples are bounded and summaries omit work content',()=>{
  const f=fixture();f.run("profiling=true;for(let i=0;i<200;i++)profileRecord('render:sentry',i)");
  const values=JSON.parse(f.run('JSON.stringify(profileSummary())'));
  assert.equal(values[0].samples,120);assert.equal(values[0].maxMs,199);
  assert.equal(values[0].p95Ms,193);
  assert.equal(f.run("JSON.stringify(renderPerformancePanel()).includes('private title')"),false);
});
test('instrumented requests and rendering preserve errors and record duration',async()=>{
  const f=fixture();f.run('profiling=true');assert.equal((await f.run("api('/api/state')")).ok,true);
  f.run('render()');assert.equal(f.run("profileSummary().some(x=>x.name==='request:/api/state')"),true);
  assert.equal(f.run("profileSummary().some(x=>x.name==='render:sentry')"),true);
  f.context.renderRow=()=>{throw new Error('test');};
  assert.throws(()=>f.run("profileWrap('error',renderRow)()"),/test/);
  assert.equal(f.run("profileSummary().some(x=>x.name==='error')"),true);
});

test('render timing includes shared after hooks and records failures after cleanup',()=>{
 const f=fixture();f.run('profiling=true');f.context.afterRan=0;f.run("registerRenderHook('slow-shared-hook',()=>{performance.now();performance.now();afterRan++;},{order:40})");
 f.run('render()');assert.equal(f.context.afterRan,1);assert.equal(f.run("profileSummary().find(x=>x.name==='render:sentry').maxMs"),3);
 f.context.renderWorkspacePage=()=>{throw new Error('page failed');};assert.throws(()=>f.run('render()'),/page failed/);assert.equal(f.context.afterRan,2);assert.equal(f.run("profileSummary().find(x=>x.name==='render:sentry').samples"),2);
});
