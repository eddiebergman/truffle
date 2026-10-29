const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
test('skill picker finds abbreviations across name, description and path and prefers compatible agents',()=>{
 const root=path.join(__dirname,'..');
 const app=fs.readFileSync(path.join(root,'static/app.js'),'utf8');
 const picker=fs.readFileSync(path.join(root,'static/skill-picker.js'),'utf8');
 const context=vm.createContext({});
 vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function render() {')),context);
 vm.runInContext(picker.slice(picker.indexOf('function skillPickerMatches('),picker.indexOf('function installSkillPicker(')),context);
 const skills=[{name:'latency review',description:'Find slow paths',path:'/skills/latency/SKILL.md',agents:['claude']},{name:'latency review',description:'Find slow paths',path:'/codex/latency/SKILL.md',agents:['codex']}];
 assert.equal(context.skillPickerMatches(skills,'ltncy rvw','codex')[0].path,'/codex/latency/SKILL.md');
 assert.equal(context.skillPickerMatches(skills,'slow paths','claude').length,2);
 assert.equal(context.skillPickerMatches(skills,'no such installed thing','claude').length,0);
});

function picker(){
 const app=fs.readFileSync(path.join(__dirname,'../static/app.js'),'utf8'),source=fs.readFileSync(path.join(__dirname,'../static/skill-picker.js'),'utf8'),context=vm.createContext({});
 vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function render() {')),context);
 vm.runInContext(source.slice(source.indexOf('function skillPickerMatches('),source.indexOf('function installSkillPicker(')),context);return (skills,query,agent='codex')=>Array.from(context.skillPickerMatches(skills,query,agent));
}
const skill=(name,description='',extra={})=>({name,description,path:'/Users/reviewer/.agents/skills/'+name+'/SKILL.md',agents:['codex'],...extra});

test('direct names and commands rank before description matches or typo fallbacks',()=>{
 const find=picker(),exact=skill('review'),command=skill('Custom workflow','',{commandName:'team-review'}),description=skill('notes','Review changes in source documents.'),fuzzy=skill('revenue-indexing-workflow','');
 const matches=find([description,fuzzy,command,exact],'review');
 assert.deepEqual(matches.map(item=>item.name),['review','Custom workflow','notes']);
});

test('review search excludes scattered description matches and absolute home path clutter',()=>{
 const find=picker(),review=skill('code-review'),unrelated=[skill('utm-link','Rewrite tracking links for every visitor with useful web metadata.'),skill('docs','Read documents, edit files, inspect values and write prose.'),skill('xlsx','Read tables, edit workbooks and view output.'),skill('morning','Refresh messages, explain updates, verify schedules and write notes.')];
 assert.deepEqual(find([...unrelated,review],'review').map(item=>item.name),['code-review']);
 assert.deepEqual(find(unrelated,'review'),[]);
});

test('description searches require every query token to match a real word prefix',()=>{
 const find=picker(),matching=skill('analyze','Find slow paths and summarize evidence.'),missing=skill('create','Find slow operations without a matching location.'),preview=skill('image','Build previews and process images.');
 assert.deepEqual(find([missing,matching],'slow paths').map(item=>item.name),['analyze']);
 assert.deepEqual(find([preview],'review'),[]);
});

test('punctuation-normalized names, commands and explicit file paths remain searchable',()=>{
 const find=picker(),named=skill('eddie-review'),command=skill('Check changes','',{commandName:'team:inspect'}),nested=skill('analysis','',{path:'/Users/reviewer/work/project/custom/sketch/SKILL.md'});
 assert.deepEqual(find([named,command,nested],'eddiereview').map(item=>item.name),['eddie-review']);
 assert.deepEqual(find([named,command,nested],'teaminspect').map(item=>item.name),['Check changes']);
 assert.deepEqual(find([named,command,nested],'/work/project/custom/sketch').map(item=>item.name),['analysis']);
});

test('name abbreviation and omitted-letter typo fallback remain available when there are no direct matches',()=>{
 const find=picker(),matching=skill('latency review'),unrelated=skill('docs','Very long description '.repeat(40));
 assert.deepEqual(find([unrelated,matching],'ltncy rvw').map(item=>item.name),['latency review']);
 assert.deepEqual(find([unrelated,matching],'revew').map(item=>item.name),['latency review']);
});

test('empty searches keep compatible agent ordering and the result limit',()=>{
 const find=picker(),skills=Array.from({length:45},(_,index)=>skill('skill-'+index,'',{agents:index===44?['codex']:['claude']}));
 const matches=find(skills,'');assert.equal(matches.length,30);assert.equal(matches[0].name,'skill-44');
 assert.deepEqual(find(skills,'::'),[]);
});

function installedPickerFixture(catalog,{load}={}){
 let observer,active,body;
 class Element{
  constructor(tag='div',className='',text=''){this.tagName=tag.toUpperCase();this.className=className;this.textContent=text;this.children=[];this.attrs={};this.listeners={};this.value='';this.options=[];this.hidden=false;}
  append(...children){for(const child of children){child.parent=this;this.children.push(child);}}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  replaceChildren(...children){if(this.children.some(child=>child.contains(active)))active=body;this.children=[];this.append(...children);}
  setAttribute(key,value){this.attrs[key]=value;}
  addEventListener(name,handler){(this.listeners[name]??=[]).push(handler);}
  dispatch(name){for(const handler of this.listeners[name]||[])handler({target:this});}
  focus(){active=this;}
  after(node){this.afterNode=node;}
  closest(selector){return selector==='.form-grid'?grid:this.label;}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(selector==='button'&&child.tagName==='BUTTON'?[child]:[]),...child.querySelectorAll(selector)]);}
 }
 body=new Element('body');const grid=new Element(),heading=new Element(),hint=new Element(),save=new Element('button'),dialog=new Element('dialog');dialog.dataset={};
 const elements=Object.fromEntries(['title','cwd','agent','command','skill','description'].map(name=>{const field=new Element(name==='agent'?'select':'input');field.label=new Element('label');return [name,field];}));
 elements.agent.value='claude';elements.cwd.value='/repo';elements.agent.options=['claude','codex'].map(value=>Object.assign(new Element('option','',value==='claude'?'Claude':'Codex'),{value}));
 const form={elements,querySelector:selector=>selector==='.dialog-heading'?heading:selector===':scope>.form-hint'?hint:save};
 const c=vm.createContext({$:selector=>selector==='#skill-editor'?dialog:form,el:(...args)=>new Element(...args),featureButton:(label,onclick,cls)=>Object.assign(new Element('button',cls,label),{onclick}),agentName:agent=>agent==='codex'?'Codex':'Claude',api:async()=>load?load():({skills:catalog}),MutationObserver:class{constructor(callback){observer=callback;}observe(){}},Node:{TEXT_NODE:3},clearTimeout(){},setTimeout(){},document:{body,get activeElement(){return active;}}});
 const owner={isCurrent:()=>dialog.open,pending:false,result:null};c.featureForms=new Map([[dialog,owner]]);const features=fs.readFileSync(path.join(__dirname,'../static/features.js'),'utf8'),editor=fs.readFileSync(path.join(__dirname,'../static/skills-editor.js'),'utf8');vm.runInContext(features.slice(features.indexOf('function featureFormMutable('),features.indexOf('function freezeFeatureForm(')),c);vm.runInContext(editor.slice(editor.indexOf('function skillEditorMutable('),editor.indexOf('function skillScheduleMutable(')),c);
 const app=fs.readFileSync(path.join(__dirname,'../static/app.js'),'utf8');vm.runInContext(app.slice(app.indexOf('function fuzzyScore('),app.indexOf('function render() {')),c);
 vm.runInContext(fs.readFileSync(path.join(__dirname,'../static/skill-picker.js'),'utf8'),c);
 const picker=heading.afterNode,status=picker.children[2],results=picker.children[4],modes=picker.children[0];
 return {c,owner,elements,status,save,dialog,results,get active(){return active;},outside:()=>new Element('input'),async open(prefill={}){Object.assign(elements.title,{value:prefill.title||''});Object.assign(elements.skill,{value:prefill.path||''});Object.assign(elements.command,{value:prefill.command||''});if(prefill.agent)elements.agent.value=prefill.agent;dialog.open=true;observer();await Promise.resolve();await Promise.resolve();},select(index=0){results.children[index].onclick();},changeAgent(agent){elements.agent.value=agent;elements.agent.dispatch('change');},mode(mode){modes.children[mode==='file'?1:0].onclick();}};
}

test('selected installed skill disables incompatible agents and rejects incompatible change events; File path is unrestricted',async()=>{
 const f=installedPickerFixture([skill('review')]);await f.open();f.select();
 assert.equal(f.elements.agent.value,'codex');assert.equal(f.elements.command.value,'$review');assert.equal(f.save.disabled,false);
 const claude=f.elements.agent.options[0];assert.equal(claude.disabled,true);assert.match(claude.textContent,/unavailable/);assert.match(f.status.textContent,/Available for Codex/);
 f.changeAgent('claude');assert.equal(f.elements.agent.value,'codex');assert.equal(f.elements.command.value,'$review');
 f.mode('file');assert.equal(claude.disabled,false);assert.equal(claude.textContent,'Claude');f.changeAgent('claude');assert.equal(f.elements.agent.value,'claude');assert.equal(f.elements.command.value,'$review');
 f.mode('installed');assert.equal(f.elements.agent.value,'codex');assert.equal(claude.disabled,true);
});

test('dual-agent installed skill switches generated invocation prefix and preserves deliberately edited commands',async()=>{
 const f=installedPickerFixture([skill('Review changes','',{commandName:'team-review',agents:['claude','codex']})]);await f.open();f.select();
 assert.equal(f.elements.command.value,'/team-review');f.changeAgent('codex');assert.equal(f.elements.command.value,'$team-review');f.changeAgent('claude');assert.equal(f.elements.command.value,'/team-review');
 assert.ok(f.elements.agent.options.every(option=>!option.disabled));
 f.elements.command.value='/custom-review --scope local';f.changeAgent('codex');assert.equal(f.elements.command.value,'/custom-review --scope local');assert.match(f.status.textContent,/Custom command kept; check it for Codex/);
 f.elements.command.value='';f.changeAgent('claude');assert.equal(f.elements.command.value,'/team-review');
});

test('prefilled installed skill resolves catalog compatibility and command name before agent switching',async()=>{
 const known=skill('Review changes','',{commandName:'team-review',agents:['claude','codex']}),f=installedPickerFixture([known]);
 await f.open({title:known.name,path:known.path,command:'$team-review',agent:'codex'});assert.ok(f.elements.agent.options.every(option=>!option.disabled));f.changeAgent('claude');assert.equal(f.elements.command.value,'/team-review');
});

test('catalog resolution preserves prefilled custom commands and clearing selection restores agent availability',async()=>{
 const known=skill('Review changes','',{commandName:'team-review'}),f=installedPickerFixture([known]);
 await f.open({title:known.name,path:known.path,command:'run-custom-review --flag',agent:'codex'});assert.equal(f.elements.command.value,'run-custom-review --flag');
 f.mode('file');f.changeAgent('claude');f.mode('installed');assert.equal(f.elements.agent.value,'codex');assert.equal(f.elements.command.value,'run-custom-review --flag');
 // The actual Change action clears the selection and lifts installed-skill restrictions.
 const change=f.results.parent.children[3].children[2];change.onclick();assert.ok(f.elements.agent.options.every(option=>!option.disabled));assert.equal(f.save.disabled,true);
});

test('delayed catalog cannot edit locked payload fields or unlock Save, and superseded opening ignores old results',async()=>{
 let release;const f=installedPickerFixture([],{load:()=>new Promise(resolve=>release=resolve)});await f.open({title:'Review',path:'/skill',command:'$kept',agent:'codex'});f.owner.pending=true;f.save.disabled=true;release({skills:[skill('Review','',{path:'/skill',agents:['claude']})]});await new Promise(resolve=>setImmediate(resolve));assert.equal(f.elements.agent.value,'codex');assert.equal(f.elements.command.value,'$kept');assert.equal(f.save.disabled,true);f.changeAgent('claude');assert.equal(f.elements.command.value,'$kept');
 let old;const g=installedPickerFixture([],{load:()=>new Promise(resolve=>old=resolve)});await g.open();g.c.featureForms.set(g.dialog,{isCurrent:()=>true,pending:false});g.status.textContent='Replacement opening';old({skills:[skill('Old result')]});await new Promise(resolve=>setImmediate(resolve));assert.equal(g.status.textContent,'Replacement opening');assert.equal(g.results.children.length,0);
});

test('preselected skill retains the focused Change action across delayed catalog updates without stealing outside focus',async()=>{
 const known=skill('Review changes','',{commandName:'team-review',agents:['claude','codex']});let release;const f=installedPickerFixture([],{load:()=>new Promise(resolve=>release=resolve)});await f.open({title:known.name,path:known.path,command:'$team-review',agent:'codex'});const selected=f.results.parent.children[3],change=selected.children[2];change.focus();release({skills:[known]});await new Promise(resolve=>setImmediate(resolve));assert.equal(f.active,change);assert.equal(selected.children[2],change);assert.equal(change.disabled,false);assert.equal(f.elements.command.value,'$team-review');
 let next;const g=installedPickerFixture([],{load:()=>new Promise(resolve=>next=resolve)});await g.open({title:known.name,path:known.path,command:'$team-review',agent:'codex'});const outside=g.outside();outside.focus();next({skills:[known]});await new Promise(resolve=>setImmediate(resolve));assert.equal(g.active,outside);
});
