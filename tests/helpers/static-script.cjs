'use strict';
// Load whole static scripts into a vm context, the way index.html loads them.
// Tests never depend on where code sits inside a file.
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const {browserHost}=require('./fake-dom.cjs');

const root=path.resolve(__dirname,'../..');
const scripts=new Map(),expressions=new Map(),raws=new WeakMap();
const identifier=/^[A-Za-z_$][\w$]*$/;
const slot='__staticScriptValue';

function compiled(file){
 const filename=path.resolve(root,file);
 if(!scripts.has(filename))scripts.set(filename,new vm.Script(fs.readFileSync(filename,'utf8'),{filename}));
 return scripts.get(filename);
}
function expression(code){
 if(!expressions.has(code))expressions.set(code,new vm.Script(code));
 return expressions.get(code);
}
function rawContext(c){return raws.get(c)||c;}
// Read a global the way script code sees it: a top-level let/const/class wins over a property.
function readGlobal(raw,name){
 try{return expression(`typeof ${name}==='undefined'?undefined:${name}`).runInContext(raw);}
 catch(error){if(error?.name==='ReferenceError')return undefined;throw error;}
}
function writeGlobal(raw,name,value){
 raw[slot]=value;
 try{expression(`${name}=${slot}`).runInContext(raw);}
 catch(error){if(error?.name==='TypeError')throw new TypeError(`Cannot replace const ${name} declared by a static script`);throw error;}
 finally{delete raw[slot];}
}

// Like vm.createContext, but reads and writes through the returned object also
// reach top-level let/const bindings of the loaded scripts.
function browserContext(globals={}){
 const raw=vm.createContext(globals);
 const proxy=new Proxy(raw,{
  get(target,key){return typeof key==='string'&&identifier.test(key)&&key!==slot?readGlobal(target,key):Reflect.get(target,key);},
  set(target,key,value){if(typeof key==='string'&&identifier.test(key)&&key!==slot)writeGlobal(target,key,value);else Reflect.set(target,key,value);return true;},
 });
 raws.set(proxy,raw);
 return proxy;
}

// Run static scripts in order. Globals the test already set win over the
// declarations of the scripts, so tests can stub sibling functions and state.
// While the scripts load, browser host globals (document, window, timers, ...)
// come from a tolerant fake; names in `keepHost` use the test's value instead.
// `elements` maps selectors to the objects that load-time lookups return, so the
// handlers that a script attaches at load land on the test's own nodes.
// Returns the load-time host, so tests can reach the other attached handlers.
function loadScripts(c,files,{keepHost=[],elements={}}={}){
 const raw=rawContext(c),host=browserHost();
 for(const [selector,element] of Object.entries(elements))host.document.elements.set(selector,element);
 const provided=new Map(Object.keys(raw).filter(name=>identifier.test(name)).map(name=>[name,raw[name]]));
 for(const name of Object.keys(host))if(!keepHost.includes(name))raw[name]=host[name];
 try{for(const file of [files].flat())compiled(file).runInContext(raw);}
 finally{
  for(const name of Object.keys(host)){
   if(keepHost.includes(name))continue;
   if(provided.has(name))raw[name]=provided.get(name);else delete raw[name];
  }
 }
 for(const [name,value] of provided)if(!Object.hasOwn(host,name)&&readGlobal(raw,name)!==value)writeGlobal(raw,name,value);
 return host;
}

// Evaluate code inside the context, for example to read a top-level binding.
function evaluate(c,code){return expression(code).runInContext(rawContext(c));}

module.exports={browserContext,loadScripts,evaluate,rawContext};
