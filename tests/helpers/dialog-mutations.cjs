'use strict';
// Model queued native attribute records; consumers decide when to deliver callbacks.
module.exports=function dialogMutations(){
 const observers=new Set();
 class MutationObserver{
  constructor(callback){this.callback=callback;this.records=[];}
  observe(root,options){if(!root)throw Error('Mutation observer requires a root');this.root=root;this.options=options;observers.add(this);}
  takeRecords(){const records=this.records;this.records=[];return records;}
  disconnect(){observers.delete(this);this.records=[];}
  static flush(){for(const observer of observers){const records=observer.takeRecords();if(records.length)observer.callback(records);}}
  static get activeCount(){return observers.size;}
 }
 const recordOpenChange=target=>{for(const observer of observers)if(observer.root===target||observer.options.subtree&&observer.root.contains(target))observer.records.push({type:'attributes',attributeName:'open',target});};
 return {MutationObserver,recordOpenChange};
};
