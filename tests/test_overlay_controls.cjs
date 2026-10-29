const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),views=fs.readFileSync('static/views.js','utf8');
const helper=app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog'))"));
class Dialog extends EventTarget{
 constructor(){super();this.open=true;this.closed=0;this.children=[];}
 close(){if(!this.open)return;this.closed++;this.open=false;}
 append(...nodes){this.children.push(...nodes);}
 getBoundingClientRect(){return {left:0,right:100,top:0,bottom:100};}
}
function fixture(){
 const dialog=new Dialog(),button={closest:()=>dialog};let cancels=0;
 const c=vm.createContext({Event,dialog,saving:true,document:{querySelectorAll:selector=>selector==='.close-dialog'?[button]:[dialog]},$:()=>({}),view:{name:'Mine'}});
 vm.runInContext(helper,c);vm.runInContext(views.split('\n').find(line=>line.includes("dialog.addEventListener('cancel'")),c);
 dialog.addEventListener('cancel',()=>cancels++);
 return {c,dialog,button,cancels:()=>cancels};
}
const escape=dialog=>({key:'Escape',target:{closest:()=>dialog},preventDefault(){},stopImmediatePropagation(){}});
test('Escape honors the actual saved-view saving cancel guard after dismissing a child layer',()=>{
 const f=fixture();let layers=1;f.dialog.dismissLayer=()=>layers-- >0;
 vm.runInContext(app.slice(app.indexOf('function dismissOpenDialog('),app.indexOf("document.addEventListener('keydown',dismissOpenDialog")),f.c);
 f.c.dismissOpenDialog(escape(f.dialog));assert.equal(f.cancels(),0);assert.equal(f.dialog.open,true);
 f.c.dismissOpenDialog(escape(f.dialog));assert.equal(f.cancels(),1);assert.equal(f.dialog.open,true);
 f.c.saving=false;f.c.dismissOpenDialog(escape(f.dialog));assert.equal(f.cancels(),2);assert.equal(f.dialog.closed,1);
});
test('shared close buttons and backdrop dismissal honor feature-owned cancel guards',()=>{
 for(const route of ['button','backdrop']){
  const f=fixture();let dismiss;
  if(route==='button'){
   vm.runInContext(app.slice(app.indexOf("for (const button of document.querySelectorAll('.close-dialog'))"),app.indexOf('function dismissOpenDialog(')),f.c);dismiss=()=>f.button.onclick();
  }else{
   const line=app.split('\n').find(line=>line.startsWith("for (const dialog of document.querySelectorAll('dialog'))"));vm.runInContext(line,f.c);
   dismiss=()=>{const event=new Event('click');event.clientX=120;event.clientY=120;f.dialog.dispatchEvent(event);};
  }
  dismiss();assert.equal(f.dialog.open,true,route);assert.equal(f.cancels(),1,route);f.c.saving=false;dismiss();assert.equal(f.dialog.closed,1,route);
 }
});
test('saved-view header Close uses the same cancellation policy as Escape',()=>{
 const f=fixture(),node=()=>({children:[],append(...nodes){this.children.push(...nodes);},setAttribute(){}});
 Object.assign(f.c,{el:node,savedViewButton:(_,onclick)=>({...node(),onclick})});
 const header=views.split('\n').find(line=>line.includes("header=el('div','saved-view-editor-heading')"));vm.runInContext(header,f.c);
 const close=f.dialog.children[0].children[1];close.onclick();assert.equal(f.dialog.open,true);assert.equal(f.cancels(),1);f.c.saving=false;close.onclick();assert.equal(f.dialog.closed,1);
});
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};};
function themeFixture(){
 let document;const nodes=[],calls=[],notices=[];
 class Node extends EventTarget{
  constructor(tag,cls='',text=''){super();this.tag=tag;this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.value='';this.hidden=false;this.classList={contains:name=>this.className.split(' ').includes(name),add:name=>this.className+=' '+name};nodes.push(this);}
  append(...children){for(const child of children){child.parentElement=this;this.children.push(child);}}
  replaceChildren(...children){if(this.contains(document.activeElement))document.activeElement=body;for(const child of this.children)child.parentElement=null;this.children=[];this.append(...children);}
  get isConnected(){return this===body||!!this.parentElement?.isConnected;}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  closest(selector){for(let node=this;node;node=node.parentElement)if(selector.split(',').some(part=>part===node.tag||part[0]==='#'&&node.id===part.slice(1)||part[0]==='.'&&node.classList.contains(part.slice(1))))return node;return null;}
  querySelectorAll(selector){return this.children.flatMap(child=>[...(selector.split(',').includes(child.tag)?[child]:[]),...child.querySelectorAll(selector)]);}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  setAttribute(key,value){this.attrs[key]=value;}
  set disabled(value){this._disabled=value;if(value&&document?.activeElement===this)document.activeElement=body;}
  get disabled(){return Boolean(this._disabled);}
  focus(){if(this.isConnected&&!this.hidden&&!this.disabled)document.activeElement=this;}
  click(){if(!this.disabled)return this.onclick?.();}
  showModal(){this.open=true;}
  close(){this.open=false;if(this.contains(document.activeElement))document.activeElement=body;this.dispatchEvent(new Event('close'));}
 }
 const body=new Node('body');document={body,activeElement:body,querySelectorAll:selector=>selector==='dialog[open]'?nodes.filter(node=>node.tag==='dialog'&&node.open&&node.isConnected):[]};
 const dialog=new Node('dialog'),form=new Node('form'),settingsDialog=new Node('dialog'),settingsError=new Node('p');dialog.id='quick-action-picker';form.id='quick-action-picker-form';dialog.append(form);body.append(dialog,settingsDialog,settingsError);
 const catalog={dark:{id:'dark',label:'Dark',type:'dark'},light:{id:'light',label:'Light',type:'light'}};
 let saved={revision:1,contentHash:'a',values:{theme:'dark',other:'latest'}};
 const {MutationObserver}=require(require('node:path').resolve('tests/helpers/dialog-mutations.cjs'))();
 const c=vm.createContext({Event,MutationObserver,source:'all',document,window:{truffleThemeCatalog:catalog},appSettings:saved,
  $:selector=>({'#quick-action-picker':dialog,'#quick-action-picker-form':form,'#settings-dialog':settingsDialog,'#settings-error':settingsError})[selector],
  el:(...args)=>new Node(...args),featureButton:(label,onclick,cls)=>Object.assign(new Node('button',cls,label),{onclick,type:'button'}),
  setting:key=>c.appSettings.values[key],toast:text=>notices.push(text),applyAppSettings(){},
  api:async(path,payload)=>{calls.push({path,payload});if(payload){saved={revision:2,contentHash:'b',values:{...saved.values,...payload.patch}};return saved;}return path==='/api/themes'?{themes:catalog}:saved;}});
 const settings=fs.readFileSync('static/settings.js','utf8'),features=fs.readFileSync('static/features.js','utf8'),product=fs.readFileSync('static/product-tools.js','utf8');
 vm.runInContext(helper+app.slice(app.indexOf('function holdFormFocus('),app.indexOf('function projectCorrectionTarget('))+features.slice(features.indexOf('function freezeFeatureForm('),features.indexOf('function presentFeatureFormResult('))+settings.slice(settings.indexOf('function settingsButton('),settings.indexOf('function settingsReset('))+settings.slice(settings.indexOf('async function loadAppSettings('),settings.indexOf('function focusPageSearch('))+product.slice(product.indexOf('async function openThemeChooser('),product.indexOf('// Sharing has its own CAS document')),c);
 return {c,dialog,form,body,document,nodes,calls,notices,Node,settingsDialog,settingsError,buttons:()=>form.querySelectorAll('button'),button:label=>form.querySelectorAll('button').find(node=>node.textContent===label),choices:()=>form.querySelectorAll('button').filter(node=>node.className==='theme-choice'),posts:()=>calls.filter(call=>call.payload)};
}
test('theme chooser exposes a shared secondary Close action without applying a theme',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const actions=f.form.children.find(node=>node.className==='ui-actiongroup');assert.equal(actions.attrs.role,'group');assert.equal(actions.attrs['aria-label'],'Theme chooser actions');const close=f.button('Close');assert.equal(close.type,'button');assert.equal(close.dataset.variant,'secondary');close.click();assert.equal(f.dialog.open,false);assert.equal(f.posts().length,0);
});
test('theme pending owner blocks duplicate choice, submit, Close, Escape and navigation before one write',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const post=deferred(),original=f.c.api;f.c.api=async(path,payload)=>payload?(f.calls.push({path,payload}),post.promise):original(path,payload);
 const choice=f.choices()[1];choice.focus();const pending=choice.onclick();assert.equal(f.document.activeElement.tag,'p');assert.ok(f.buttons().every(button=>button.disabled));assert.equal(f.form.children[1].disabled,true);
 const duplicate=choice.onclick();f.form.onsubmit({preventDefault(){}});assert.equal(f.posts().length,1);f.button('Close').onclick();assert.equal(f.c.requestDialogNavigation(),false);assert.equal(f.dialog.open,true);assert.equal(f.posts().length,1);assert.deepEqual(JSON.parse(JSON.stringify(f.posts()[0].payload)),{revision:1,contentHash:'a',patch:{theme:'light'}});
 post.resolve({values:{theme:'OLD RESPONSE',other:'stale'}});await pending;await duplicate;assert.equal(f.dialog.open,false);assert.equal(f.c.appSettings.values.other,'latest');assert.notEqual(f.c.appSettings.values.theme,'OLD RESPONSE');
});
test('theme successful owned save closes after presentation without a recovery warning',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();await f.choices()[1].onclick();assert.equal(f.posts().length,1);assert.equal(f.c.appSettings.values.theme,'light');assert.equal(f.dialog.open,false);assert.deepEqual(f.notices,[]);
});
test('theme accepted read failure locks choices and retries GET only before closing',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const original=f.c.api;let fail=true;f.c.api=async(path,payload)=>{if(!payload&&path==='/api/settings'&&fail)throw Error('Read unavailable');return original(path,payload);};const choice=f.choices()[1];choice.focus();await choice.onclick();assert.equal(f.dialog.open,true);assert.equal(f.document.activeElement,f.button('Refresh view'));assert.match(f.form.children[3].textContent,/Theme saved/);assert.equal(f.posts().length,1);await choice.onclick();assert.equal(f.posts().length,1);fail=false;await f.button('Refresh view').onclick();assert.equal(f.dialog.open,false);assert.equal(f.posts().length,1);
});
test('theme unknown postcommit outcome stays locked after read-only refresh without resubmitting',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const original=f.c.api;f.c.api=async(path,payload)=>{const result=await original(path,payload);if(payload)throw Object.assign(Error('Lost reply'),{status:503});return result;};const choice=f.choices()[1];choice.focus();await choice.onclick();assert.equal(f.dialog.open,true);assert.equal(f.document.activeElement,f.button('Close'));assert.match(f.form.children[3].textContent,/outcome is unknown/);assert.ok(f.choices().every(button=>button.disabled));await f.button('Refresh view').onclick();assert.match(f.form.children[3].textContent,/remains unknown/);assert.equal(f.c.appSettings.values.theme,'light');assert.equal(f.choices()[1].attrs['aria-pressed'],'true');assert.equal(f.choices()[0].attrs['aria-pressed'],'false');assert.ok(f.choices().every(button=>button.disabled));await choice.onclick();assert.equal(f.posts().length,1);f.button('Close').click();assert.equal(f.dialog.open,false);
});
test('theme failed read-only recovery keeps accepted and uncertain save truth visible',async()=>{
 for(const unknown of [false,true]){
  const f=themeFixture();await f.c.openThemeChooser();const original=f.c.api;f.c.api=async(path,payload)=>{if(!payload)throw Error('Read unavailable');const result=await original(path,payload);if(unknown)throw Error('Lost reply');return result;};await f.choices()[1].onclick();await f.button('Refresh view').onclick();assert.match(f.form.children[3].textContent,unknown?/save outcome remains unknown/:/Theme saved/);assert.match(f.form.children[3].textContent,/could not be displayed/);assert.ok(f.choices().every(button=>button.disabled));assert.equal(f.posts().length,1);assert.equal(f.dialog.open,true);
 }
});
test('theme conflict retains search draft and requires explicit current baseline reload',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const original=f.c.api;let conflict=true;f.c.api=async(path,payload)=>{if(payload&&conflict){f.calls.push({path,payload});throw Object.assign(Error('Changed'),{status:409});}return original(path,payload);};const query=f.form.children[1];query.value='Light';query.oninput();f.choices()[0].focus();await f.choices()[0].onclick();assert.equal(query.value,'Light');assert.equal(f.document.activeElement,f.button('Reload themes'));assert.ok(f.choices()[0].disabled);await f.button('Reload themes').onclick();assert.equal(query.value,'Light');assert.equal(f.document.activeElement,query);assert.equal(f.choices()[0].disabled,false);conflict=false;await f.choices()[0].onclick();assert.equal(f.posts().length,2);
});
test('theme ordinary rejected correction restores owned choice and preserves baseline-disabled controls',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const close=f.button('Close');close.disabled=true;const choice=f.choices()[0];choice.focus();f.c.api=async()=>{throw Object.assign(Error('Invalid theme'),{status:400});};await choice.onclick();assert.equal(f.document.activeElement,choice);assert.equal(choice.disabled,false);assert.equal(close.disabled,true);assert.match(f.form.children[3].textContent,/Invalid theme/);
});
test('theme redraw prunes discarded choice controls and stale initial loads cannot replace newer pickers',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();f.c.api=async()=>{throw Object.assign(Error('Rejected'),{status:400});};await f.choices()[0].onclick();const first=f.choices()[0];f.form.children[1].oninput();assert.equal(f.c.openThemeChooser.current.controls.has(first),false);f.dialog.close();
 const a=deferred(),b=deferred();let call=0;f.c.api=async path=>path==='/api/settings'?(++call===1?a.promise:b.promise):{themes:{dark:{id:'dark',label:'Dark',type:'dark'}}};const old=f.c.openThemeChooser(),latest=f.c.openThemeChooser();a.resolve({revision:2,values:{theme:'dark'}});assert.equal(await old,false);assert.equal(f.dialog.open,false);b.resolve({revision:3,values:{theme:'dark'}});assert.equal(await latest,true);assert.equal(f.dialog.open,true);
});
test('theme settlement never closes replacement picker or steals outside/child focus',async()=>{
 for(const change of ['replacement','closed','source','outside','child']){
  const f=themeFixture();await f.c.openThemeChooser();const post=deferred(),original=f.c.api;f.c.api=async(path,payload)=>payload?(f.calls.push({path,payload}),post.promise):original(path,payload);f.choices()[0].focus();const pending=f.choices()[0].onclick();
  let outside;if(change==='replacement'){f.form.replaceChildren(new f.Node('input'));f.dialog.open=true;}else if(change==='closed'){f.dialog.close();}else if(change==='source')f.c.source='github';else{outside=new f.Node(change==='child'?'dialog':'input');f.body.append(outside);if(change==='child')outside.showModal();outside.focus();}
  post.reject(Object.assign(Error('Lost response'),{status:503}));await pending;if(outside)assert.equal(f.document.activeElement,outside,change);if(change==='replacement')assert.equal(f.dialog.open,true);assert.equal(f.posts().length,1);f.c.requestDialogClose(f.dialog);assert.equal(f.dialog.open,false,change+' listener released after pending');
 }
});
test('theme accepted GET cannot close or paint a replaced owner and child modal retains focus',async()=>{
 for(const change of ['replacement','closed','source','child']){
  const f=themeFixture();await f.c.openThemeChooser();const read=deferred(),original=f.c.api;f.c.api=async(path,payload)=>!payload&&path==='/api/settings'?read.promise:original(path,payload);
  const pending=f.choices()[1].onclick();await Promise.resolve();await Promise.resolve();let child;
  if(change==='replacement')f.form.replaceChildren(new f.Node('input'));if(change==='closed')f.dialog.close();if(change==='source')f.c.source='github';if(change==='child'){child=new f.Node('dialog');f.body.append(child);child.showModal();child.focus();}
  read.resolve({revision:2,values:{theme:'light',other:'read'}});await pending;
  if(change!=='closed')assert.equal(f.dialog.open,true,change);if(child){assert.equal(f.document.activeElement,child);assert.equal(f.button('Refresh view').hidden,false);assert.equal(f.button('Close').disabled,false);}else assert.notEqual(f.c.appSettings.values.other,'read',change);
  assert.equal(f.posts().length,1);
 }
});
test('theme uncertain response alone invalidates a pre-write settings GET without overwriting latest fields',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const old=deferred(),initial=f.c.appSettings,original=f.c.api;let hold=true;
 f.c.api=async(path,payload)=>{if(payload){f.calls.push({path,payload});throw Object.assign(Error('Committed but reply lost'),{status:503});}if(path==='/api/settings'&&hold){hold=false;return old.promise;}return original(path,payload);};
 const read=f.c.loadAppSettings();await f.choices()[1].onclick();assert.notEqual(f.c.appSettings,initial);assert.equal(f.c.appSettings.values,initial.values);old.resolve({revision:0,values:{theme:'old',other:'stale'}});assert.equal(await read,null);assert.equal(f.c.appSettings.values.other,'latest');assert.equal(f.posts().length,1);
});
test('theme accepted POST document is never installed when both authoritative GET attempts fail',async()=>{
 const f=themeFixture();await f.c.openThemeChooser();const latest=f.c.appSettings;f.c.api=async(path,payload)=>{if(payload){f.calls.push({path,payload});return {revision:2,values:{theme:'stale response',other:'lost'}};}throw Error('Read failed');};await f.choices()[1].onclick();assert.equal(f.c.appSettings.values,latest.values);assert.equal(f.c.appSettings.values.other,'latest');assert.equal(f.dialog.open,true);assert.match(f.form.children[3].textContent,/Theme saved/);assert.equal(f.posts().length,1);
});
test('theme accepted and unknown writes fence old settings reads while keeping latest unrelated values',async()=>{
 for(const unknown of [false,true]){
  const f=themeFixture();await f.c.openThemeChooser();const old=deferred(),original=f.c.api;let hold=true;f.c.api=async(path,payload)=>{if(!payload&&path==='/api/settings'&&hold){hold=false;return old.promise;}const result=await original(path,payload);if(payload&&unknown)throw Error('Lost reply');return result;};
  const read=f.c.loadAppSettings();f.c.appSettings={...f.c.appSettings,values:{theme:'dark',other:'concurrent'}};await f.choices()[1].onclick();old.resolve({revision:0,values:{theme:'old',other:'stale'}});assert.equal(await read,null);assert.notEqual(f.c.appSettings.values.other,'stale');assert.notEqual(f.c.appSettings.values.theme,'old');assert.equal(f.posts().length,1);
 }
});
test('theme accepted paint failures remain read-only until a GET actually paints successfully',async()=>{
 for(const failCount of [1,10]){
  const f=themeFixture();await f.c.openThemeChooser();let paints=0;f.c.applyAppSettings=()=>{paints++;if(paints<=failCount)throw Error('Theme paint failed');};
  f.choices()[1].focus();await f.choices()[1].onclick();assert.equal(f.posts().length,1);assert.equal(paints,2);
  if(failCount===1){assert.equal(f.dialog.open,false);assert.equal(f.c.appSettings.values.theme,'light');}
  else{assert.equal(f.dialog.open,true);assert.match(f.form.children[3].textContent,/Theme saved/);assert.equal(f.document.activeElement,f.button('Refresh view'));f.c.applyAppSettings=()=>{paints++;};await f.button('Refresh view').onclick();assert.equal(f.dialog.open,false);assert.equal(f.posts().length,1);}
 }
});
test('settings loader retries catalog-only presentation failure and never rolls back newer references',async()=>{
 const f=themeFixture(),prior=f.c.appSettings,catalog=f.c.window.truffleThemeCatalog,newCatalog={dark:{id:'dark',label:'Updated',type:'dark'}};let paints=0;f.c.api=async path=>path==='/api/settings'?structuredClone(prior):{themes:newCatalog};f.c.applyAppSettings=()=>{paints++;throw Error('Paint failed');};
 assert.equal(await f.c.loadAppSettings(),undefined);assert.equal(f.c.appSettings,prior);assert.equal(f.c.window.truffleThemeCatalog,catalog);assert.equal(await f.c.loadAppSettings(),undefined);assert.equal(paints,2);
 const newer={revision:99,values:{theme:'newer'}},newerCatalog={newer:{id:'newer'}};f.c.applyAppSettings=()=>{f.c.appSettings=newer;f.c.window.truffleThemeCatalog=newerCatalog;throw Error('Paint changed owner');};await f.c.loadAppSettings();assert.equal(f.c.appSettings,newer);assert.equal(f.c.window.truffleThemeCatalog,newerCatalog);
});
test('settings loader does not overwrite a newer document while awaiting themes or paint obsolete errors',async()=>{
 const f=themeFixture(),themes=deferred();f.c.api=async path=>path==='/api/settings'?{revision:1,values:{theme:'old'}}:themes.promise;const read=f.c.loadAppSettings();await Promise.resolve();const latest={revision:3,values:{theme:'latest',other:'kept'}};f.c.appSettings=latest;themes.resolve({themes:{}});assert.equal(await read,null);assert.equal(f.c.appSettings,latest);
 const failed=deferred();f.c.api=()=>failed.promise;f.settingsDialog.open=true;f.settingsError.textContent='current error';const failure=f.c.loadAppSettings();f.c.appSettings={...latest};failed.reject(Error('obsolete failure'));await failure;assert.equal(f.settingsError.textContent,'current error');
});
test('normal App chat width remains inside scaled viewport gutters according to its actual CSS constraint',()=>{
 const css=fs.readFileSync('static/app-chat.css','utf8'),block=css.slice(css.indexOf('.app-chat-panel{'),css.indexOf('}',css.indexOf('.app-chat-panel{'))),width=block.split(';').find(value=>value.startsWith('width:'));
 const [,cap,calc]=width.match(/width:min\(([\d.]+)px,calc\((.*)\)\)/),right=Number(block.match(/right:([\d.]+)px/)[1]);
 for(const viewport of [280,500,800])for(const scale of [1,1.25,1.5,2]){
  const expression=calc.replace('100vw',String(viewport)).replace('var(--ui-scale,1)',String(scale)).replace(/px/g,'');assert.match(expression,/^[\d.\s+\-*/()]+$/);
  const physicalWidth=Math.min(Number(cap),vm.runInNewContext(expression))*scale;
  assert.ok(physicalWidth>0);assert.ok(physicalWidth+right*scale<=viewport,JSON.stringify({viewport,scale,physicalWidth}));
 }
});
