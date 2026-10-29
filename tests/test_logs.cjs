'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const {appChatDiagnosticRows}=require('../static/app-chat.js'),source=fs.readFileSync('static/features.js','utf8');
function fixture(entries){
 class Node{constructor(tag,cls='',text=''){this.tag=tag;this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};}append(...items){this.children.push(...items);}replaceChildren(...items){this.children=items;}setAttribute(key,value){this.attrs[key]=value;}querySelectorAll(){return [];}}
 const copied=[],c=vm.createContext({el:(...args)=>new Node(...args),Option:function(text,value){this.text=text;this.value=value;},appChatDiagnosticRows,logData:{entries},logLoading:false,logLoadError:'',logFilters:{source:'',outcome:'failure'},openLogEntries:new Set(),sourceNames:{github:'GitHub',linear:'Linear',sentry:'Sentry'},state:{sources:{}},document:{activeElement:null},loadLogs(){},featureButton:(text,onclick)=>Object.assign(new Node('button','',text),{onclick}),navigator:{clipboard:{writeText:async text=>copied.push(text)}},toast(){}});
 vm.runInContext(source.slice(source.indexOf('function logAppChatRows(')),c);return {c,list:new Node('list'),controls:new Node('controls'),copied};
}
function descendants(node){return [node,...node.children.flatMap(descendants)];}
const entry={time:'2026-10-27T12:00:00Z',source:'app-chat',operation:'app-chat-reply',outcome:'failure',requestId:'request-1',diagnostics:{requestId:'request-1',stage:'cli-envelope',code:'missing_result',exitCode:0,durationMs:1234,stdoutBytes:81,stderrBytes:0,stdoutShape:'object',responseShape:'empty',stderr:'private-error-marker',prompt:'private-prompt-marker'}};
test('App chat failures use fixed diagnostic summaries while sync failures retain their existing causes',()=>{
 const {c}=fixture([]);assert.equal(c.logSummary(entry),'CLI returned no reply · CLI result');assert.equal(c.logSummary({...entry,diagnostics:{code:'invalid_assistant_json',stage:'assistant-response'}}),'Assistant reply is not valid JSON · assistant reply');assert.equal(c.logSummary({...entry,diagnostics:{code:'private-marker',stage:'private-marker'}}),'App chat failed');assert.equal(c.logSummary({outcome:'failure',kind:'authentication'}),'Authentication rejected');assert.equal(c.logSummary({outcome:'success',itemCount:3}),'Sync completed · 3 items');
});
test('logs discover App chat sources and display only safe diagnostic metadata without the old-sync warning',async()=>{
 const {c,list,controls,copied}=fixture([entry]);c.renderLogsView(list,controls,'request-1');const all=descendants(list),fields=all.find(node=>node.className==='log-fields');
 assert.ok(controls.children[0].children.some(option=>option.value==='app-chat'&&option.text==='App chat'));assert.match(controls.children.at(-1).textContent,/Local diagnostics/);assert.ok(!JSON.stringify(all).includes('older entry retained sync'));assert.ok(!JSON.stringify(all).includes('private-'));
 const pairs=Object.fromEntries(Array.from({length:fields.children.length/2},(_,i)=>[fields.children[i*2].textContent,fields.children[i*2+1].textContent]));assert.equal(pairs['Request ID'],'request-1');assert.equal(pairs.Stage,'cli-envelope');assert.equal(pairs.Code,'missing_result');assert.equal(pairs.Duration,'1.23 s');assert.equal(pairs['Exit code'],'0');assert.equal(pairs['CLI output bytes'],'81');
 await all.find(node=>node.tag==='button'&&node.textContent==='Copy JSON').onclick();assert.ok(!copied[0].includes('private-'));assert.match(copied[0],/missing_result/);
 const filtered=fixture([entry]);filtered.c.logFilters.source='app-chat';filtered.c.renderLogsView(filtered.list,filtered.controls,'missing_result');assert.equal(descendants(filtered.list).filter(node=>node.className==='log-entry').length,1);assert.equal(filtered.controls.children[0].value,'app-chat');
});
