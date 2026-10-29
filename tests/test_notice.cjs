const {test}=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const app=fs.readFileSync('static/app.js','utf8'),renderSync=app.slice(app.indexOf('function renderSync()'),app.indexOf('const projectEditableFields='));
function fixture(){
 const nodes=new Map(),storage=new Map(),calls=[],document={activeElement:null};
 class Element{
  constructor(tag='div',cls='',text=''){this.tagName=tag.toUpperCase();this.className=cls;this.textContent=text;this.children=[];this.dataset={};this.attrs={};this.classList={add:cls=>this.className+=' '+cls,toggle(){}};}
  contains(node){return this===node||this.children.some(child=>child.contains(node));}
  focus(){document.activeElement=this;}
  append(...children){for(const child of children){child.parentElement=this;this.children.push(child);}}
  replaceChildren(...children){this.children=[];this.append(...children);}
  setAttribute(key,value){this.attrs[key]=value;}
 }
 const $=id=>{if(!nodes.has(id))nodes.set(id,new Element());return nodes.get(id);};
 const c=vm.createContext({$,document,state:{sources:{github:{lastSuccess:1,nextAttempt:0,error:'Retry later.'},linear:{lastSuccess:1,nextAttempt:0,error:null},sentry:{disabled:true}},syncing:[],demo:false},sourceNames:{github:'GitHub',linear:'Linear',sentry:'Sentry'},Date,relative:()=> 'just now',localStorage:{getItem:key=>storage.get(key)||null,setItem:(key,value)=>storage.set(key,value),removeItem:key=>storage.delete(key)},el:(...args)=>new Element(...args),openSettings:()=>calls.push('settings:appearance'),settingsCategory:'appearance',openPreferences:({category})=>{c.settingsCategory=category;calls.push('settings:'+category);return true;},source:'agents',persist:()=>calls.push('persist'),render:()=>calls.push('render')});const pageSettings=fs.readFileSync('static/page-settings.js','utf8');vm.runInContext(pageSettings.slice(pageSettings.indexOf('async function openPageSettingsCategory('),pageSettings.indexOf('function pageSettingsButton(')),c);vm.runInContext(renderSync,c);
 return {c,$,storage,calls,document,render:()=>c.renderSync(),get notice(){return $('#connection-notice');}};
}
test('connection warning separates title, safe multiline body, named dismiss and shared action controls',()=>{
 const f=fixture(),message='Long error <script>unsafe()</script>\n'+'a'.repeat(5000);f.c.state.sources.github.error=message;f.render();
 const notice=f.notice,[header,body,actions]=notice.children;assert.equal(notice.parentElement,f.$('#sidebar-notices'));assert.equal(notice.hidden,false);assert.match(notice.className,/ui-notice/);assert.equal(notice.dataset.variant,'warning');
 assert.equal(header.className,'ui-notice-header');assert.equal(header.children[0].textContent,'Connections need attention');const dismiss=header.children[1];assert.equal(dismiss.attrs['aria-label'],'Dismiss connection warning');assert.equal(dismiss.dataset.size,'icon');assert.equal(dismiss.type,'button');assert.equal(dismiss.dataset.variant,'secondary');
 assert.equal(body.className,'ui-notice-body');assert.equal(body.tabIndex,0);assert.equal(body.children[0].textContent,'GitHub: '+message);assert.equal(body.children[0].children.length,0);
 assert.match(actions.className,/ui-actiongroup/);assert.equal(actions.attrs.role,'group');assert.equal(actions.attrs['aria-label'],'Connection warning actions');assert.deepEqual(actions.children.map(button=>button.textContent),['View connections','View logs']);for(const button of actions.children){assert.match(button.className,/ui-control/);assert.equal(button.dataset.variant,'secondary');assert.equal(button.type,'button');}
 actions.children[0].onclick();assert.deepEqual(f.calls,['settings:connections']);actions.children[1].onclick();assert.equal(f.c.source,'logs');assert.deepEqual(f.calls,['settings:connections','persist','render']);
});
test('dismissal stays tied to exact warning identity and clears after recovery without empty notice text',()=>{
 const f=fixture();f.render();const dismiss=()=>f.notice.children[0].children[1].onclick();dismiss();assert.equal(f.notice.hidden,true);assert.equal(f.storage.size,1);f.render();assert.equal(f.notice.hidden,true);
 f.c.state.sources.github.error='A different retry window.';f.render();assert.equal(f.notice.hidden,false);dismiss();f.c.state.sources.github.error=null;f.render();assert.equal(f.notice.hidden,true);assert.equal(f.notice.children.length,0);assert.equal(f.storage.size,0);
 f.c.state.sources.github.error='A different retry window.';f.render();assert.equal(f.notice.hidden,false);assert.equal(f.notice.children[1].children.length,1);
 f.c.state.demo=true;f.render();assert.equal(f.notice.hidden,true);assert.equal(f.notice.children.length,0);
});
test('unchanged warnings retain their controls through renders so focused action nodes remain usable',()=>{
 const f=fixture();f.render();const header=f.notice.children[0],body=f.notice.children[1],actions=f.notice.children[2];f.render();assert.equal(f.notice.children[0],header);assert.equal(f.notice.children[1],body);assert.equal(f.notice.children[2],actions);actions.children[0].onclick();assert.deepEqual(f.calls,['settings:connections']);
 f.c.state.sources.linear.error='Linear account needs attention.';f.render();assert.notEqual(f.notice.children[1],body);assert.equal(f.notice.children[1].children.length,2);
});
test('all loaded styles leave connection notice layout to the shared header and controls',()=>{
 // Inspect every shipped stylesheet in its actual document order. This asserts
 // selector ownership, not simulated browser layout or a partial CSS cascade.
 const html=fs.readFileSync('static/index.html','utf8');
 const styles=[...html.matchAll(/<link\b[^>]*rel="stylesheet"[^>]*href="([^"]+)"[^>]*>/g)].map(match=>({path:'static'+match[1],css:fs.readFileSync('static'+match[1],'utf8')}));
 assert.ok(styles.some(style=>style.path==='static/product-tools.css'));assert.ok(styles.some(style=>style.path==='static/ui-controls.css'));
 const rules=styles.flatMap(style=>[...style.css.replace(/\/\*[\s\S]*?\*\//g,'').matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(match=>({path:style.path,selector:match[1].trim(),body:match[2]})));
 for(const rule of rules.filter(rule=>/(?:\.|#)connection-notice(?:-dismiss)?\b/.test(rule.selector))){
  assert.doesNotMatch(rule.body,/(?:^|;)\s*(?:position|inset(?:-[\w-]+)?|top|right|bottom|left|padding(?:-[\w-]+)?|width|height)\s*:/,rule.path+' '+rule.selector+' must not override shared notice layout');
 }
 const shared=rules.find(rule=>rule.selector==='.ui-notice-header');assert.ok(shared);assert.match(shared.body,/display:grid/);assert.match(shared.body,/grid-template-columns:minmax\(0,1fr\) auto/);
 const dismiss=rules.find(rule=>rule.selector==='.ui-notice-header>.ui-notice-dismiss');assert.ok(dismiss);assert.match(dismiss.body,/grid-column:2/);
});

test('notice replacement restores the same focused action or body; dismissal and recovery move focus to Activity only from inside',()=>{
 for(const role of ['dismiss','body','connections','logs']){
  const f=fixture();f.render();const targets=()=>[f.notice.children[0].children[1],f.notice.children[1],...f.notice.children[2].children];
  const old=targets().find(node=>node.dataset.noticeFocus===role);old.focus();f.c.state.sources.github.error='New warning';f.render();
  assert.notEqual(f.document.activeElement,old);assert.equal(f.document.activeElement.dataset.noticeFocus,role);assert.ok(f.notice.contains(f.document.activeElement));
 }
 for(const recover of [false,true]){const f=fixture();f.render();f.notice.children[0].children[1].focus();if(recover){f.c.state.sources.github.error=null;f.render();}else f.notice.children[0].children[1].onclick();assert.equal(f.document.activeElement,f.$('#activity-open'));assert.equal(f.notice.hidden,true);}
 const f=fixture();f.render();const outside=f.$('#outside');outside.focus();f.c.state.sources.github.error='New warning';f.render();assert.equal(f.document.activeElement,outside);f.notice.children[0].children[1].onclick();assert.equal(f.document.activeElement,outside);f.c.state.sources.github.error=null;f.render();assert.equal(f.document.activeElement,outside);
});
