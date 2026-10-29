/* Installed skills are read-only until the existing preset form is saved. */
function skillPickerMatches(skills, query, agent) {
  const lower=value=>String(value||'').normalize('NFKC').toLowerCase();
  const compact=value=>lower(value).replace(/[^\p{L}\p{N}]/gu,'');
  const words=value=>lower(value).match(/[\p{L}\p{N}]+/gu)||[];
  const needle=lower(query).trim(),tokens=words(needle),normalized=compact(needle),explicitPath=/[\\/]/.test(needle);
  if(needle&&!normalized&&!explicitPath)return [];
  const strong=value=>{const text=lower(value);return Boolean(normalized)&&(text.includes(needle)||compact(text).includes(normalized)||tokens.every(token=>words(text).some(word=>word.startsWith(token))));};
  const candidates=skills.map((skill,index)=>{
    const parts=lower(skill.path).split(/[\\/]/).filter(Boolean),marker=Math.max(parts.lastIndexOf('skills'),parts.lastIndexOf('commands'));
    const segments=parts.slice(marker>=0?marker+1:-2).filter(part=>part!=='skill.md');
    const identity=[skill.name,skill.commandName].filter(Boolean),names=[...identity,...segments];
    let tier=-1;
    if(!needle)tier=0;
    else if(identity.some(name=>compact(name)===normalized))tier=0;
    else if(identity.some(strong))tier=1;
    else if((explicitPath&&lower(skill.path).includes(needle))||segments.some(strong))tier=2;
    else if(tokens.length&&tokens.every(token=>words(skill.description).some(word=>word.startsWith(token))))tier=3;
    const score=needle?Math.max(...names.map(name=>fuzzyScore(needle,name))):0;
    return {skill,index,tier,score,compatible:skill.agents.includes(agent)};
  });
  const direct=candidates.filter(row=>row.tier>=0);
  const matches=direct.length?direct:candidates.filter(row=>Number.isFinite(row.score));
  return matches.sort((a,b)=>a.tier-b.tier||b.score-a.score||Number(b.compatible)-Number(a.compatible)||a.index-b.index).slice(0,30).map(row=>row.skill);
}
function installSkillPicker(){
 const dialog=$('#skill-editor'),form=$('#skill-editor-form');if(!dialog||!form)return;
 const picker=el('section','skill-picker'),modes=el('div','skill-picker-modes');modes.setAttribute('role','group');modes.setAttribute('aria-label','Choose one skill source');
 const installed=featureButton('Installed skills',()=>setMode('installed'),'skill-source-tab'),file=featureButton('File path',()=>setMode('file'),'skill-source-tab');installed.type=file.type='button';modes.append(installed,file);
 const label=el('label','','Find an installed skill'),input=el('input');input.type='search';input.placeholder='Search name, description or path…';input.autocomplete='off';label.append(input);
 const status=el('p','form-hint');status.setAttribute('role','status');const results=el('div','skill-picker-results'),selection=el('div','skill-picker-selection');results.setAttribute('aria-label','Matching installed skills');
 picker.append(modes,label,status,selection,results);form.querySelector('.dialog-heading').after(picker);
 const pathLabel=form.elements.skill.closest('label'),common=[form.elements.title.closest('label'),form.elements.cwd.closest('label'),form.elements.agent.closest('.form-grid'),form.querySelector(':scope>.form-hint')],save=form.querySelector('[type="submit"]');
 let skills=[],mode='installed',selected=null,generation=0,timer,selectedAgent='',generatedCommand='',customCommandKept=false;
 const agentLabels=new Map([...form.elements.agent.options].map(option=>[option,option.textContent]));
 const invocation=(skill,agent)=>(agent==='codex'?'$':'/')+(skill.commandName||skill.name);
 // Catalog refreshes update the selected skill without replacing its focused action.
 const selectionName=el('strong'),selectionAgents=el('span'),change=featureButton('Change',()=>{if(!skillEditorMutable())return;selected=null;input.value='';paintMode();renderMatches();input.focus();},'text-button');selection.append(selectionName,selectionAgents,change);

 function syncSelectedAgent(){
  if(mode!=='installed'||!selected||!skillEditorMutable())return;
  const agent=form.elements.agent,command=form.elements.command;
  if(!selected.agents.includes(agent.value))agent.value=selected.agents.includes(selectedAgent)?selectedAgent:selected.agents[0];
  const next=invocation(selected,agent.value);customCommandKept=Boolean(command.value.trim()&&command.value!==generatedCommand);
  if(!customCommandKept)command.value=next;
  generatedCommand=next;selectedAgent=agent.value;
 }
 function pills(skill){const row=el('span','skill-picker-pills');for(const agent of skill.agents)row.append(el('span','skill-picker-pill',agentName(agent)));for(const scope of skill.sources||[])row.append(el('span','skill-picker-pill skill-scope-pill',scope));return row;}
 function paintMode(){
  for(const [option,label] of agentLabels){option.disabled=mode==='installed'&&Boolean(selected)&&!selected.agents.includes(option.value);option.textContent=label+(option.disabled?' · unavailable for this skill':'');}
  if(mode==='installed'&&selected)status.textContent='Available for '+selected.agents.map(agentName).join(' and ')+'.'+(customCommandKept?' Custom command kept; check it for '+agentName(form.elements.agent.value)+'.':'');
  installed.setAttribute('aria-pressed',String(mode==='installed'));file.setAttribute('aria-pressed',String(mode==='file'));label.hidden=status.hidden=results.hidden=mode==='file';selection.hidden=mode==='file'||!selected;
  for(const field of common)if(field)field.hidden=mode==='installed'&&!selected;
  pathLabel.hidden=mode==='installed';save.disabled=!skillEditorMutable()||mode==='installed'&&!selected;input.disabled=!skillEditorMutable();installed.disabled=file.disabled=!skillEditorMutable();
  selectionName.textContent=selected?.name||'';selectionAgents.replaceChildren(...(selected?[pills(selected)]:[]));selection.title=selected?.path||'';change.disabled=!skillEditorMutable();
 }
 function setMode(next){if(!skillEditorMutable())return;mode=next;syncSelectedAgent();paintMode();if(mode==='installed')input.focus();else form.elements.skill.focus();}
 function renderMatches(){
  results.replaceChildren();if(selected){paintMode();return;}
  const matches=skillPickerMatches(skills,input.value,form.elements.agent.value);
  status.textContent=skills.length?matches.length+' matches · select one to configure':'No installed skills found. Choose File path to add one directly.';
  for(const skill of matches){
   const button=el('button','skill-picker-result'),head=el('span','skill-picker-result-heading');button.type='button';button.title=skill.path;head.append(el('strong','',skill.name),pills(skill));button.append(head,el('span','skill-picker-description',skill.description));
   button.disabled=!skillEditorMutable();button.onclick=()=>{if(!skillEditorMutable())return;generation++;clearTimeout(timer);selected=skill;if(!skill.agents.includes(form.elements.agent.value))form.elements.agent.value=skill.agents[0];form.elements.title.value=skill.name;form.elements.skill.value=skill.path;generatedCommand=invocation(skill,form.elements.agent.value);selectedAgent=form.elements.agent.value;customCommandKept=false;form.elements.command.value=generatedCommand;if(form.elements.description)form.elements.description.value=skill.description;input.value=skill.name;results.replaceChildren();paintMode();form.elements.cwd.focus();};
   results.append(button);
  }
 }
 async function load(){const request=++generation,owner=featureForms.get(dialog);status.textContent='Finding installed skills…';try{const catalog=await api('/api/skills/catalog',{cwd:form.elements.cwd.value});if(request!==generation||!dialog.open||featureForms.get(dialog)!==owner)return;skills=catalog.skills||[];if(selected&&skillEditorMutable()){const known=skills.find(skill=>skill.path===selected.path);if(known){if(form.elements.command.value===invocation(known,selectedAgent))generatedCommand=form.elements.command.value;selected=known;syncSelectedAgent();}}renderMatches();if(catalog.truncated)status.textContent+=' · More skills may be available by file path.';}catch(error){if(request===generation&&dialog.open&&featureForms.get(dialog)===owner){results.replaceChildren();status.textContent='Could not discover skills: '+error.message+'. Choose File path to add one directly.';}}}
 input.oninput=()=>{if(!skillEditorMutable())return;selected=null;paintMode();renderMatches();};form.elements.agent.addEventListener('change',()=>{if(!skillEditorMutable())return;if(mode==='installed'&&selected){syncSelectedAgent();paintMode();}else if(mode==='installed')renderMatches();});form.elements.cwd.addEventListener('input',()=>{if(!skillEditorMutable()||selected)return;clearTimeout(timer);timer=setTimeout(load,300);});
 input.addEventListener('keydown',event=>{if(['Enter','ArrowDown'].includes(event.key)){event.preventDefault();results.querySelector('button')?.focus();}});
 results.addEventListener('keydown',event=>{const buttons=[...results.querySelectorAll('button')],index=buttons.indexOf(document.activeElement);if(['ArrowDown','ArrowUp'].includes(event.key)){event.preventDefault();buttons[Math.max(0,Math.min(buttons.length-1,index+(event.key==='ArrowDown'?1:-1)))]?.focus();}});
 new MutationObserver(()=>{if(dialog.open){input.value='';mode=dialog.dataset.skillSource==='file'?'file':'installed';delete dialog.dataset.skillSource;selected=form.elements.skill.value?{name:form.elements.title.value,path:form.elements.skill.value,agents:[form.elements.agent.value],sources:[]}:null;selectedAgent=form.elements.agent.value;generatedCommand=selected?invocation(selected,selectedAgent):'';customCommandKept=false;paintMode();void load();}}).observe(dialog,{attributes:true,attributeFilter:['open']});
 if(pathLabel.firstChild?.nodeType===Node.TEXT_NODE)pathLabel.firstChild.textContent='Skill file path';
}
installSkillPicker();
