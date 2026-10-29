'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync('static/connection-setup.js','utf8');
class Element {
 constructor(tag){this.tagName=tag;this.children=[];this.dataset={};this.attributes={};this.value='';this.textContent='';this.className='';this.disabled=false;}
 append(...children){this.children.push(...children);}
 replaceChildren(...children){this.children=children;}
 setAttribute(name,value){this.attributes[name]=value;}
 closest(){return null;}
 all(){return this.children.flatMap(child=>[child,...child.all()]);}
 querySelectorAll(selector){return this.all().filter(node=>selector==='input[type="password"]'?node.tagName==='input'&&node.type==='password':false);}
}
function fixture({available=true,preview,initialView}={}){
 const nativeCalls=[],httpCalls=[],created=[];
 const config={available,revision:0,providers:[]};
 const invoke=async(name,args)=>{
  nativeCalls.push({name,args});
  switch(name){
   case 'connection_setup_view':return initialView?await initialView(config):config;
   case 'connection_setup_cancel':return {cancelled:true};
   case 'connection_setup_preview':return preview?await preview(args.payload):{previewId:'opaque-fixture',expectedRevision:0,identity:'fixture-user',resources:['fixture/repo'],resourceLabel:'Repositories'};
   case 'connection_setup_commit':return {...config,revision:1};
   default:throw new Error('Unexpected native command '+name);
  }
 };
 const c=vm.createContext({window:{__TAURI__:{core:{invoke}}},document:{createElement:tag=>{const node=new Element(tag);created.push(node);return node;}},api:async(path,payload)=>{httpCalls.push({path,payload});return {errors:{}};}});
 vm.runInContext(source,c);const container=new Element('main');
 const find=(root,label)=>root.all().find(node=>node.tagName==='button'&&node.textContent===label);
 return {c,container,nativeCalls,httpCalls,created,find,render:()=>c.window.renderConnectionSetup(container)};
}

test('token payloads use known providers and keep custom hosts out of GitHub and Linear',()=>{
 const {c}=fixture();
 const github=c.connectionSetupPayload('github',{token:' benign-fixture ',base:'https://attacker.invalid'});
 assert.equal(github.token,'benign-fixture');assert.equal(github.provider,'github');assert.equal(github.base,undefined);
 assert.equal(c.connectionSetupPayload('github',{source:'github-cli'}).token,undefined);
 assert.throws(()=>c.connectionSetupPayload('invented',{token:'benign'}),/Unknown/);
 assert.throws(()=>c.connectionSetupPayload('linear',{source:'github-cli'}),/GitHub/);
 assert.throws(()=>c.connectionSetupPayload('sentry',{token:'benign'}),/organization/);
});

test('manual validation clears password before native IPC and only public preview metadata enters DOM',async()=>{
 const f=fixture();const root=await f.render();const input=root.querySelectorAll('input[type="password"]')[0];
 assert.equal(input.autocomplete,'off');assert.equal(input.attributes['data-agent-context-exclude'],'true');assert.equal(input.attributes['data-private'],'true');
 input.value='benign-fixture-secret';
 await f.find(root,'Validate token').onclick({isTrusted:true});
 assert.equal(input.value,'');const submitted=f.nativeCalls.find(call=>call.name==='connection_setup_preview');assert.equal(submitted.args.payload.token,'benign-fixture-secret');assert.equal(f.httpCalls.length,0);
 assert.ok(root.all().some(node=>node.textContent==='Account: fixture-user'));
 assert.ok(root.all().some(node=>node.textContent==='fixture/repo'));
 assert.ok(root.all().every(node=>!node.textContent.includes('benign-fixture-secret')));
});

test('save requires the staged preview and activation is a separate secret-free action',async()=>{
 const f=fixture();const root=await f.render();root.querySelectorAll('input[type="password"]')[0].value='benign-fixture-secret';await f.find(root,'Validate token').onclick({isTrusted:true});
 await f.find(root,'Save connection').onclick({isTrusted:true});
 const commit=f.nativeCalls.find(call=>call.name==='connection_setup_commit');assert.deepEqual(JSON.parse(JSON.stringify(commit.args)),{payload:{previewId:'opaque-fixture',expectedRevision:0}});
 assert.deepEqual(JSON.parse(JSON.stringify(f.httpCalls)),[{path:'/api/connections/reload',payload:{provider:'github'}}]);
 assert.ok(root.all().some(node=>node.textContent.includes('Existing synchronization settings still apply')));
 await f.find(root,'Import work items').onclick({isTrusted:true});
 assert.deepEqual(JSON.parse(JSON.stringify(f.httpCalls.at(-1))),{path:'/api/connections/reload',payload:{provider:'github',activate:true}});
 assert.ok(!JSON.stringify(f.httpCalls).includes('benign-fixture-secret'));
});

test('untrusted clicks cannot request manual or GitHub CLI credentials',async()=>{
 const f=fixture();const root=await f.render();root.querySelectorAll('input[type="password"]')[0].value='benign-fixture';
 await f.find(root,'Validate token').onclick({isTrusted:false});await f.find(root,'Use GitHub CLI login').onclick({isTrusted:false});
 assert.equal(f.nativeCalls.some(call=>call.name==='connection_setup_preview'),false);
});

test('disposed components cancel a pending preview and cannot display or save its late result',async()=>{
 let release;const f=fixture({preview:()=>new Promise(resolve=>release=resolve)});const root=await f.render();root.querySelectorAll('input[type="password"]')[0].value='benign-fixture';
 const validation=f.find(root,'Validate token').onclick({isTrusted:true});await new Promise(resolve=>setImmediate(resolve));
 await root.disposeConnectionSetup();release({previewId:'late-preview',expectedRevision:0,identity:'late-user',resources:[]});await validation;
 assert.equal(f.find(root,'Save connection'),undefined);assert.ok(root.all().every(node=>!node.textContent.includes('late-user')));
 assert.ok(f.nativeCalls.filter(call=>call.name==='connection_setup_cancel').length>=3);
 assert.equal(f.nativeCalls.some(call=>call.name==='connection_setup_commit'),false);
});

test('unavailable secure storage keeps all credential controls disabled and accepts no tokens',async()=>{
 const f=fixture({available:false});const root=await f.render();
 assert.ok(root.all().filter(node=>['button','input','select'].includes(node.tagName)).every(node=>node.disabled));
 await f.find(root,'Validate token').onclick({isTrusted:true});assert.equal(f.nativeCalls.some(call=>call.name==='connection_setup_preview'),false);
 assert.ok(root.all().some(node=>node.textContent.includes('does not accept or store tokens')));
});


test('disposal is available before initial native view completes and prevents late controls',async()=>{
 let release;const f=fixture({initialView:config=>new Promise(resolve=>release=()=>resolve(config))});
 const rendering=f.render(),root=f.container.children[0];assert.equal(typeof root.disposeConnectionSetup,'function');
 await root.disposeConnectionSetup();release();assert.equal(await rendering,root);
 assert.equal(root.all().some(node=>['button','input','select'].includes(node.tagName)),false);
 assert.equal(f.nativeCalls.filter(call=>call.name==='connection_setup_cancel').length,1);
 await root.disposeConnectionSetup();assert.equal(f.nativeCalls.filter(call=>call.name==='connection_setup_cancel').length,1);
});

test('closing during initial view load cancels setup and removes its listener',async()=>{
 let release;const listeners=new Map(),removed=[];
 const dialog={addEventListener:(name,callback)=>listeners.set(name,callback),removeEventListener:(name,callback)=>{assert.equal(listeners.get(name),callback);listeners.delete(name);removed.push(name);}};
 const f=fixture({initialView:config=>new Promise(resolve=>release=()=>resolve(config))});f.container.closest=()=>dialog;
 const rendering=f.render();assert.equal(typeof listeners.get('close'),'function');listeners.get('close')();
 release();const root=await rendering;await new Promise(resolve=>setImmediate(resolve));
 assert.equal(root.all().some(node=>node.tagName==='input'),false);assert.equal(listeners.has('close'),false);assert.deepEqual(removed,['close']);
 assert.equal(f.nativeCalls.filter(call=>call.name==='connection_setup_cancel').length,1);
});

test('reopened saved disabled connection offers explicit import without validating or saving credentials again',async()=>{
 const f=fixture({initialView:config=>({...config,providers:[{id:'github',configured:true,identity:'fixture-user',syncEnabled:false}]})});
 const root=await f.render(),importButton=f.find(root,'Import work items');assert.ok(importButton);assert.equal(importButton.disabled,false);assert.equal(f.httpCalls.length,0);
 assert.deepEqual(f.nativeCalls.map(call=>call.name),['connection_setup_view']);
 await importButton.onclick({isTrusted:false});assert.equal(f.httpCalls.length,0);
 await importButton.onclick({isTrusted:true});assert.deepEqual(JSON.parse(JSON.stringify(f.httpCalls)),[{path:'/api/connections/reload',payload:{provider:'github',activate:true}}]);
 assert.equal(f.nativeCalls.some(call=>['connection_setup_preview','connection_setup_commit'].includes(call.name)),false);
 await root.disposeConnectionSetup();await importButton.onclick({isTrusted:true});assert.equal(f.httpCalls.length,1);
});
test('configured account import remains unavailable when private native storage is unavailable',async()=>{
 const f=fixture({available:false,initialView:config=>({...config,providers:[{id:'linear',configured:true,identity:'fixture-user'}]})}),root=await f.render(),button=f.find(root,'Import work items');
 assert.equal(button.disabled,true);await button.onclick({isTrusted:true});assert.equal(f.httpCalls.length,0);
});
