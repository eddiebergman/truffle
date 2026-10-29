const test=require('node:test');
const assert=require('node:assert/strict');
const {guidanceEligible}=require('../static/guidance.js');
test('completed, dismissed and disabled tips do not repeat across contexts',()=>{
 const progress={enabled:true,tips:{'ci.investigate':'completed','chat.shell':'dismissed'}};
 assert.equal(guidanceEligible(progress,'ci.investigate'),false);
 assert.equal(guidanceEligible(progress,'chat.shell'),false);
 assert.equal(guidanceEligible(progress,'review.agent-scope'),true);
 assert.equal(guidanceEligible({...progress,enabled:false},'review.agent-scope'),false);
});
const {guidanceReadyToSnooze}=require('../static/guidance.js');
test('snooze lesson requires three hours of known readiness with no background work',()=>{
 const now=20000,ready={state:'waiting',reason:'finished',at:9000,backgroundCounts:{}};
 assert.equal(guidanceReadyToSnooze(ready,8000,now),true);
 assert.equal(guidanceReadyToSnooze({...ready,at:now-10799},8000,now),false);
 for(const patch of [{state:'working'},{reason:'question'},{reason:'approval'},{backgroundCounts:{task:1}},{subagents:1},{scheduledWakeups:1},{at:21000}])assert.equal(guidanceReadyToSnooze({...ready,...patch},8000,now),false);
 assert.equal(guidanceReadyToSnooze(ready,10000,now),false);
});

const fs=require('node:fs'),vm=require('node:vm');
function guidanceFixture({shadow=false}={}){
 const source=fs.readFileSync('static/guidance.js','utf8');
 const node=(tag,cls,text)=>({tag,cls,textContent:text||'',style:{},attrs:{},children:[],contains(target){return this===target||this.children.some(child=>child.contains(target));},append(...children){for(const child of children){child.parent=this;this.children.push(child);}},setAttribute(key,value){this.attrs[key]=value;},getAttribute(key){return this.attrs[key]||null;},removeAttribute(key){delete this.attrs[key];},remove(){this.removed=true;this.parent.children=this.parent.children.filter(child=>child!==this);},getBoundingClientRect:()=>({width:260,height:120})});
 const body=node('body'),dialog=node('dialog');class ShadowRoot{}
 let rect={left:500,top:100,width:20,height:20,right:520,bottom:120};
 const anchor=node('button');anchor.isConnected=true;anchor.focus=()=>{context.document.activeElement=anchor;};anchor.attrs['aria-describedby']='existing-help';anchor.closest=()=>null;anchor.getBoundingClientRect=()=>rect;
 const root=shadow?Object.assign(new ShadowRoot(),{host:{closest:()=>dialog}}):{};anchor.getRootNode=()=>root;
 const context=vm.createContext({active:null,epoch:'agents|one|page',shownEpoch:'',data:{definitions:{tips:[{id:'ci.investigate',title:'CI failed',text:'Use this icon to investigate.'}]}},document:{body},ShadowRoot,el:node,featureButton:(text,onclick)=>Object.assign(node('button','',text),{onclick}),visible:()=>true,innerWidth:1000,innerHeight:700,getComputedStyle:()=>({zoom:'1.25'}),setting:()=>'',shortcutLabel:()=>'',update:()=>Promise.resolve(),toast(){}});
 for(const [start,end]of [[' function remove()',' function update('],[' function position()',' function show('],[' function show(',' function tick(']])vm.runInContext(source.slice(source.indexOf(start),source.indexOf(end)),context);
 return {context,anchor,body,dialog,move:value=>{rect=value;}};
}
test('active guidance marks the exact target and follows it at the current scale',()=>{
 const {context,anchor,move}=guidanceFixture();context.show('ci.investigate',anchor);
 const {highlight,node}=context.active;assert.equal(highlight.cls,'guidance-target-ring');assert.equal(highlight.attrs['aria-hidden'],'true');assert.equal(highlight.style.left,'396.8px');assert.equal(highlight.style.top,'76.8px');assert.equal(highlight.style.width,'22.4px');assert.equal(anchor.attrs['aria-describedby'],'existing-help '+node.id);
 move({left:600,top:200,width:24,height:24,right:624,bottom:224});context.position();assert.equal(highlight.style.left,'476.8px');assert.equal(highlight.style.height,'25.6px');
 context.remove();assert.equal(highlight.removed,true);assert.equal(node.removed,true);assert.equal(anchor.attrs['aria-describedby'],'existing-help');assert.equal(context.active,null);
});
test('guidance removes its target highlight when the control disappears',()=>{
 const {context,anchor,body}=guidanceFixture();context.show('ci.investigate',anchor);context.visible=()=>false;context.position();assert.equal(context.active,null);assert.equal(body.children.length,0);
});
test('shadow-root controls get the same highlight in the owning dialog without invalid ARIA references',()=>{
 const {context,anchor,body,dialog}=guidanceFixture({shadow:true});context.show('ci.investigate',anchor);assert.equal(body.children.length,0);assert.equal(context.active.highlight.parent,dialog);assert.equal(context.active.node.parent,dialog);assert.equal(anchor.attrs['aria-describedby'],'existing-help');context.remove();assert.equal(dialog.children.length,0);
});

test('keyboard focus keeps a tip alive during the idle pause and dismissal returns focus to its anchor',()=>{
 const {context:c,anchor}=guidanceFixture();c.show('ci.investigate',anchor);const node=c.active.node;c.document.activeElement=node.children[2];
 Object.assign(c,{lastInput:20000,Date:{now:()=>20001},epoch:'agents|one|page',source:'agents',selectedAgentKey:'one',pausedUntil:0,sessionCompleted:new Set(),typing:()=>false,candidate:()=>['ci.investigate',anchor],guidanceEligible:()=>true});
 c.data.progress={enabled:true};c.document.querySelectorAll=()=>[];c.$=()=>null;
 const script=fs.readFileSync('static/guidance.js','utf8');vm.runInContext(script.slice(script.indexOf(' function tick()'),script.indexOf(" document.addEventListener('pointerdown'")),c);
 c.tick();assert.equal(c.active?.node,node);c.remove();assert.equal(c.document.activeElement,anchor);
});

test('Escape dismisses guidance before Review and honors Review cancellation before closing',()=>{
 const app=fs.readFileSync('static/app.js','utf8');let active=true,closed=0,dismissed=0;
 const dialog=Object.assign(new EventTarget(),{id:'diff-review-dialog',close:()=>closed++});let saving=true;dialog.addEventListener('cancel',event=>{if(saving)event.preventDefault();});const c=vm.createContext({Event,truffleGuidance:{dismiss:()=>{if(!active)return false;active=false;dismissed++;return true;}},document:{querySelectorAll:()=>[dialog]}});
 vm.runInContext(app.slice(app.indexOf('function requestDialogClose('),app.indexOf("for (const button of document.querySelectorAll('.close-dialog'))")),c);
 vm.runInContext(app.slice(app.indexOf('function dismissOpenDialog('),app.indexOf("document.addEventListener('keydown',dismissOpenDialog")),c);
 const event=()=>({key:'Escape',target:{closest:()=>dialog},preventDefault(){this.prevented=true;},stopImmediatePropagation(){this.stopped=true;}});
 const first=event();c.dismissOpenDialog(first);assert.equal(dismissed,1);assert.equal(closed,0);assert.equal(first.prevented,true);assert.equal(first.stopped,true);
 c.dismissOpenDialog(event());assert.equal(closed,0);saving=false;c.dismissOpenDialog(event());assert.equal(closed,1);
});

test('visibility crosses shadow hosts and rejects gutters clipped by scroll containers',()=>{
 const script=fs.readFileSync('static/guidance.js','utf8'),rect=(top,bottom)=>({left:100,right:130,top,bottom,width:30,height:bottom-top});
 const scroller={getBoundingClientRect:()=>({left:50,right:800,top:150,bottom:600}),style:{overflowX:'auto',overflowY:'auto'}};
 const host={parentElement:scroller,getBoundingClientRect:()=>rect(100,500),style:{}};
 const gutter={isConnected:true,getBoundingClientRect:()=>rect(100,120),getRootNode:()=>({host}),style:{}};
 const c=vm.createContext({innerWidth:1000,innerHeight:700,getComputedStyle:node=>node.style});
 vm.runInContext(script.slice(script.indexOf(' function visible('),script.indexOf(' function typing(')),c);
 assert.equal(c.visible(gutter),false);gutter.getBoundingClientRect=()=>rect(200,220);assert.equal(c.visible(gutter),true);host.hidden=true;assert.equal(c.visible(gutter),false);
});

test('review guidance finds a later visible gutter when the first line is clipped',()=>{
 const script=fs.readFileSync('static/guidance.js','utf8'),first={},second={},host={shadowRoot:{querySelector:()=>first,querySelectorAll:()=>[first,second]}};
 const dialog={open:true,querySelectorAll:()=>[host]},nodes={'#diff-review-dialog':dialog,'#diff-review-composer':{hidden:true}};
 const c=vm.createContext({data:{progress:{}},sessionCompleted:new Set(),guidanceEligible:()=>true,$:selector=>nodes[selector],reviewState:{draft:{comments:[]}},visible:node=>node===second});
 vm.runInContext(script.slice(script.indexOf(' function candidate('),script.indexOf(' function position(')),c);
 assert.equal(c.candidate()[1],second);
});

test('tip width is constrained before measurement at browser UI zoom',()=>{
 const {context:c,anchor}=guidanceFixture();c.innerWidth=320;c.show('ci.investigate',anchor);const node=c.active.node;
 node.getBoundingClientRect=()=>({width:Math.min(290,parseFloat(node.style.maxWidth)||290)*1.25,height:120});
 c.position();assert.equal(node.style.maxWidth,'236.8px');const left=parseFloat(node.style.left)*1.25;assert.ok(left>=12);assert.ok(left+node.getBoundingClientRect().width<=c.innerWidth-12);
});
