import {VirtualizedFileDiff,Virtualizer,registerCustomTheme,getSharedHighlighter,getFiletypeFromFileName} from '@pierre/diffs';
import rosePine from '@shikijs/themes/rose-pine';
registerCustomTheme('truffle-rose-pine',()=>Promise.resolve({...rosePine,name:'truffle-rose-pine'}));
const registeredThemes=new Set();
function themeName(){const data=window.truffleThemeCatalog?.[document.documentElement.dataset.theme];if(!data)return 'truffle-rose-pine';if(!registeredThemes.has(data.syntaxName)){registerCustomTheme(data.syntaxName,()=>Promise.resolve({...data.theme,name:data.syntaxName}));registeredThemes.add(data.syntaxName);}return data.syntaxName;}
export async function highlightReviewCode(root,path){
  const blocks=[...root.querySelectorAll('code')];
  await Promise.all(blocks.map(async code=>{
    const text=code.textContent;if(!text||text.length>8000)return;
    const language=[...code.classList].find(name=>name.startsWith('language-'))?.slice(9)||getFiletypeFromFileName(path||'');
    if(!language)return;
    try{
      const highlighter=await getSharedHighlighter({themes:[themeName()],langs:[language]});
      if(!code.isConnected)return;
      const result=highlighter.codeToTokens(text,{lang:language,theme:themeName()}),fragment=document.createDocumentFragment();
      // Highlighting may normalize CRLF or a final newline; never change the source.
      if(result.tokens.map(line=>line.map(token=>token.content).join('')).join('\n')!==text)return;
      for(const [index,line] of result.tokens.entries()){if(index)fragment.append(document.createTextNode('\n'));for(const token of line){const span=document.createElement('span');span.textContent=token.content;if(token.color)span.style.color=token.color;fragment.append(span);}}
      code.replaceChildren(fragment);
    }catch(_){}
  }));
}
export function createDiffReview(root,style,renderComments){
  const virtualizer=new Virtualizer();virtualizer.setup(root);
  const views=new Map();
  return {
    mount(container,data,annotations,onSelect){
      const options={theme:themeName(),themeType:document.documentElement.dataset.colorScheme==='light'?'light':'dark',diffStyle:style,disableFileHeader:true,overflow:'scroll',enableLineSelection:true,onLineSelected:range=>onSelect(range),renderAnnotation:annotation=>renderComments(annotation.metadata,data.path),unsafeCSS:`:host{--diffs-font-size:${getComputedStyle(document.documentElement).getPropertyValue('--diff-font-size')||'10px'};--diffs-line-height:${getComputedStyle(document.documentElement).getPropertyValue('--diff-line-height')||'16px'};--diffs-bg:${getComputedStyle(document.documentElement).getPropertyValue('--overlay')||'#26233a'}!important}pre{--diffs-bg:${getComputedStyle(document.documentElement).getPropertyValue('--overlay')||'#26233a'}!important}`};
      const view=new VirtualizedFileDiff(options,virtualizer,{lineHeight:parseInt(getComputedStyle(document.documentElement).getPropertyValue('--diff-line-height'))||16});
      view.render({oldFile:{name:data.path,contents:data.oldContent},newFile:{name:data.path,contents:data.newContent},containerWrapper:container,lineAnnotations:annotations});
      views.set(data.path,{view,options});
    },
    annotate(path,items){const entry=views.get(path);if(entry){entry.view.setLineAnnotations(items);entry.view.rerender();}},
    setStyle(next){style=next;for(const entry of views.values()){entry.options={...entry.options,diffStyle:next};entry.view.setOptions(entry.options);entry.view.rerender();}},
    dispose(){for(const {view}of views.values())view.cleanUp();views.clear();virtualizer.cleanUp();}
  };
}

