'use strict';
// One offline parser, shared by all briefing renders. Raw HTML is never enabled.
const briefingMarkdown = markdownit({ html:false, linkify:true, typographer:false, maxNesting:40 });
const defaultBriefingLinkCheck = briefingMarkdown.validateLink;
briefingMarkdown.validateLink = value => {
  if(!/^(?:https?:|mailto:)/i.test(value)||!defaultBriefingLinkCheck(value))return false;
  try{const url=new URL(value);return !url.username&&!url.password;}catch(_){return false;}
};
briefingMarkdown.linkify.set({fuzzyEmail:false,fuzzyLink:false});
briefingMarkdown.renderer.rules.link_open = (tokens,index,options,env,renderer) => {
  tokens[index].attrSet('target','_blank');
  tokens[index].attrSet('rel','noopener noreferrer');
  return renderer.renderToken(tokens,index,options);
};
// Reports do not fetch remote images or load tracking pixels.
briefingMarkdown.renderer.rules.image = (tokens,index) => briefingMarkdown.utils.escapeHtml(tokens[index].content);
briefingMarkdown.renderer.rules.table_open = () => '<div class="markdown-table"><table>\n';
briefingMarkdown.renderer.rules.table_close = () => '</table></div>\n';
for(const tag of ['th_open','td_open'])briefingMarkdown.renderer.rules[tag]=(tokens,index,options,env,renderer)=>{
  const token=tokens[index],style=token.attrGet('style')||'';
  token.attrs=(token.attrs||[]).filter(([key])=>key!=='style');
  const align=style.match(/^text-align:(left|center|right)$/)?.[1];
  if(align)token.attrJoin('class','markdown-align-'+align);
  return renderer.renderToken(tokens,index,options);
};
briefingMarkdown.core.ruler.after('inline','briefing-task-lists',state=>{
  for(let index=2;index<state.tokens.length;index++){
    const token=state.tokens[index];
    if(token.type!=='inline'||state.tokens[index-2].type!=='list_item_open')continue;
    const first=token.children?.[0],match=first?.type==='text'&&first.content.match(/^\[([ xX])\]\s+/);
    if(!match)continue;
    first.content=first.content.slice(match[0].length);
    const checkbox=new state.Token('html_inline','',0);
    checkbox.content=match[1].toLowerCase()==='x'?'<input type="checkbox" disabled checked aria-label="Completed task">':'<input type="checkbox" disabled aria-label="Incomplete task">';
    token.children.unshift(checkbox);
    state.tokens[index-2].attrJoin('class','task-list-item');
  }
});
// Resolve only verified references; a bare PR number must identify one item.
const workReferencePattern=/(^|[^A-Za-z0-9_/#.-])((?:(?:[A-Za-z0-9_.-]+\/)?[A-Za-z0-9_.-]+)?#\d+|[A-Z][A-Z0-9]*-\d+)\b/g;
function workReferenceIndex(urls){
  const index=new Map();
  const add=(key,url)=>{if(!index.has(key))index.set(key,new Set());index.get(key).add(url);};
  for(const value of urls){
    if(!briefingMarkdown.validateLink(value))continue;
    const url=new URL(value);
    if(url.hostname==='github.com'){
      const match=url.pathname.match(/^\/([^/]+)\/([^/]+)\/(pull|issues)\/(\d+)\/?$/);
      if(!match)continue;
      const [,owner,repo,kind,number]=match,canonical=`https://github.com/${owner.toLowerCase()}/${repo.toLowerCase()}/${kind}/${number}`;
      for(const key of [`#${number}`,`${repo.toLowerCase()}#${number}`,`${owner.toLowerCase()}/${repo.toLowerCase()}#${number}`])add(key,canonical);
    }else if(url.hostname==='linear.app'){
      const match=url.pathname.match(/^\/([^/]+)\/issue\/([A-Za-z][A-Za-z0-9]*-\d+)(?:\/|$)/);
      if(match)add(match[2].toLowerCase(),`https://linear.app/${match[1]}/issue/${match[2].toUpperCase()}`);
    }
  }
  return index;
}
function briefingItemUrls(items){
  const urls=new Set();
  for(const item of items){
    if(item.url&&/^https:\/\/(?:github\.com|linear\.app)\//i.test(item.url))urls.add(item.url);
    let parent=item.parent;for(let depth=0;parent&&depth<8;depth++,parent=parent.parent)if(parent.url)urls.add(parent.url);
    for(const child of item.children||[])if(child.url)urls.add(child.url);
  }
  return [...urls].sort();
}
briefingMarkdown.core.ruler.after('linkify','briefing-work-references',state=>{
  const reportUrls=[];
  for(const block of state.tokens)for(const token of block.children||[])if(token.type==='link_open'&&token.attrGet('href'))reportUrls.push(token.attrGet('href'));
  const report=workReferenceIndex(reportUrls),catalog=workReferenceIndex(state.env.workUrls||[]);
  for(const block of state.tokens){
    if(block.type!=='inline')continue;
    const children=block.children||[],local=workReferenceIndex(children.filter(x=>x.type==='link_open').map(x=>x.attrGet('href')).filter(Boolean));
    let linkDepth=0;const output=[];
    for(const token of children){
      if(token.type==='link_open')linkDepth++;
      if(token.type==='link_close')linkDepth--;
      if(token.type!=='text'||linkDepth){output.push(token);continue;}
      let cursor=0;
      for(const match of token.content.matchAll(workReferencePattern)){
        const reference=match[2],key=reference.toLowerCase();let target=null;
        for(const index of [local,report,catalog])if(index.has(key)){const candidates=index.get(key);if(candidates.size===1)target=[...candidates][0];break;}
        if(!target)continue;
        const start=match.index+match[1].length;
        if(start>cursor){const text=new state.Token('text','',0);text.content=token.content.slice(cursor,start);output.push(text);}
        const open=new state.Token('link_open','a',1);open.attrSet('href',target);open.attrSet('title','Open '+reference);
        const text=new state.Token('text','',0);text.content=reference;
        output.push(open,text,new state.Token('link_close','a',-1));cursor=start+reference.length;
      }
      if(cursor){if(cursor<token.content.length){const tail=new state.Token('text','',0);tail.content=token.content.slice(cursor);output.push(tail);}}
      else output.push(token);
    }
    block.children=output;
  }
});

const briefingMarkdownCache=new Map();
function renderBriefingMarkdown(text,items=[]){
  const workUrls=briefingItemUrls(items),key=text+"\u0000"+workUrls.join("\n");
  if(briefingMarkdownCache.has(key))return briefingMarkdownCache.get(key);
  const rendered=briefingMarkdown.render(text,{workUrls});
  if(briefingMarkdownCache.size>=30)briefingMarkdownCache.delete(briefingMarkdownCache.keys().next().value);
  briefingMarkdownCache.set(key,rendered);
  return rendered;
}
