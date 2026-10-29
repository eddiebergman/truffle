const {test}=require('node:test'),assert=require('node:assert/strict');
const {createConfigurationRefresh}=require('../static/configuration.js');
const token=letter=>letter.repeat(64);
function fixture(){
 const calls=[],errors=[],resources={settings:async()=>calls.push('settings'),views:async()=>calls.push('views'),pageSettings:async()=>calls.push('pageSettings')};
 let busy=false,status={version:1,resources:{settings:{token:token('a')},views:{token:token('b')},pageSettings:{token:token('c')}}},reads=0;
 const refresh=createConfigurationRefresh({fetchStatus:async()=>{reads++;return status;},resources,blocked:()=>busy,onError:name=>errors.push(name)});
 return {refresh,calls,errors,resources,set busy(value){busy=value;},get status(){return status;},set status(value){status=value;},get reads(){return reads;}};
}
test('configuration refresh loads changed domains only, including same-revision content changes',async()=>{
 const f=fixture();await f.refresh();assert.deepEqual(f.calls,['settings','views','pageSettings']);f.calls.length=0;await f.refresh();assert.deepEqual(f.calls,[]);
 f.status.resources.views.token=token('d');await f.refresh();assert.deepEqual(f.calls,['views']);
});
test('dirty settings and open dialogs defer reads and preserve pending updates',async()=>{
 const f=fixture();f.busy=true;assert.equal(await f.refresh(),false);assert.equal(f.reads,0);f.busy=false;await f.refresh();assert.equal(f.calls.length,3);
 f.calls.length=0;f.status.resources.settings.token=token('e');f.resources.settings=async()=>{f.busy=true;return false;};await f.refresh();assert.deepEqual(f.calls,[]);f.busy=false;f.resources.settings=async()=>f.calls.push('retry');await f.refresh();assert.deepEqual(f.calls,['retry']);
});
test('failed resources retry independently and report a persistent failure only once',async()=>{
 const f=fixture();f.status.resources.views={error:'Invalid views'};await f.refresh();await f.refresh();assert.deepEqual(f.errors,['views']);assert.deepEqual(f.calls,['settings','pageSettings']);
 f.status.resources.views={token:token('b')};let count=0;f.resources.views=async()=>{count++;throw new Error('Transient failure');};await f.refresh();await f.refresh();assert.equal(count,2);assert.deepEqual(f.errors,['views']);
 f.resources.views=async()=>f.calls.push('recovered');await f.refresh();f.status.resources.views={error:'Invalid again'};await f.refresh();assert.deepEqual(f.errors,['views','views']);
});
test('concurrent refreshes share one request and recheck idle state after the response',async()=>{
 let resolve,busy=false,reads=0,loads=0;const gate=new Promise(r=>resolve=r),refresh=createConfigurationRefresh({fetchStatus:()=>{reads++;return gate;},resources:{settings:async()=>loads++},blocked:()=>busy,onError(){assert.fail('No error expected');}});
 const first=refresh(),second=refresh();assert.equal(first,second);assert.equal(reads,1);busy=true;resolve({version:1,resources:{settings:{token:token('a')}}});assert.equal(await first,false);assert.equal(loads,0);busy=false;await refresh();assert.equal(loads,1);
});
test('unsupported status is bounded to one error and does not acknowledge resources',async()=>{
 const f=fixture();const good=f.status;f.status={version:2,resources:{}};await f.refresh();await f.refresh();assert.deepEqual(f.errors,['status']);assert.equal(f.calls.length,0);f.status=good;await f.refresh();assert.equal(f.calls.length,3);
});

const fs=require('node:fs'),vm=require('node:vm');
test('settings and visibility loaders discard delayed background responses after editing begins',async()=>{
 for(const [file,start,end,functionName,globalName,next]of [
  ['settings.js','async function loadAppSettings(','function focusPageSearch(','loadAppSettings','appSettings',{revision:2,values:{theme:'light'}}],
  ['page-settings.js','async function loadPageSettings(','function pageSettingsGear(','loadPageSettings','pageSettingsDocument',{revision:2,pages:{}}]
 ]){
  const source=fs.readFileSync('static/'+file,'utf8');let release,allow=true,applied=0;const gate=new Promise(resolve=>release=resolve),previous={revision:1};
  const c=vm.createContext({console,api:async path=>{await gate;return path==='/api/themes'?{themes:{}}:next;},$:()=>({open:false}),window:{truffleThemeCatalog:{}},applyAppSettings:()=>applied++,render:()=>applied++,[globalName]:previous});
  vm.runInContext(source.slice(source.indexOf(start),source.indexOf(end)),c);
  const pending=c[functionName]({canApply:()=>allow});allow=false;release();assert.equal(await pending,null);assert.equal(c[globalName],previous);assert.equal(applied,0);
  allow=true;await c[functionName]({canApply:()=>allow});assert.equal(c[globalName],next);assert.equal(applied,1);
 }
});
