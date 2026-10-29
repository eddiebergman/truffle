'use strict';
let appSettings=null,settingsDraft=null,settingsBaseline=null,settingsDirty=false,settingsCategory='appearance',settingsSaving=false,settingsOpenGeneration=0,quickActionTimer=null,shortcutRecording=null,shortcutRecordingSync=Promise.resolve();
function setting(key,fallback){return appSettings?.values?.[key]??fallback;}
window.mydashThemeCatalog={};
function settingFields(){return (appSettings?.fields||[]).map(f=>[f.section,f.key,f.label,f.type,f.key==='theme'?[['system','Follow system'],...Object.values(window.mydashThemeCatalog).sort((a,b)=>a.type.localeCompare(b.type)||a.label.localeCompare(b.label)).map(t=>[t.id,t.label+' · '+t.type])]:f.type==='select'?f.choices:f.type==='number'?f.min:f.placeholder,f.max,f.unit]);}
function settingsCategories(){return [...(appSettings?.categories||[]).map(c=>[c.id,c.label]),['help','Help']];}
function themeDefinition(){return window.mydashThemeCatalog[resolvedTheme()]||window.mydashThemeCatalog['rose-pine'];}
function applyTheme(){
 const data=themeDefinition(),colors=data?.theme?.colors||{},color=(key,fallback)=>colors[key]||fallback;
 const base=data?.type==='light'?'#faf8f5':'#191724',foreground=data?.type==='light'?'#34313c':'#e0def4';
 const palette={'--bg':color('editor.background',base),'--panel':color('sideBar.background',color('editorGroupHeader.tabsBackground',base)),'--overlay':color('editorWidget.background',color('dropdown.background',base)),'--line':color('panel.border',color('editorGroup.border',data?.type==='light'?'#cec8bf':'#393552')),'--text':color('editor.foreground',foreground),'--muted':color('descriptionForeground',color('editorLineNumber.foreground',foreground)),'--iris':color('terminal.ansiMagenta',data?.type==='light'?'#72518e':'#c4a7e7'),'--foam':color('terminal.ansiCyan',data?.type==='light'?'#326a67':'#9ccfd8'),'--love':color('terminal.ansiRed',data?.type==='light'?'#b03e5a':'#eb6f92'),'--gold':color('terminal.ansiYellow',data?.type==='light'?'#8b601d':'#f6c177'),'--rose':color('terminal.ansiBrightMagenta',data?.type==='light'?'#a35d78':'#ebbcba')};
 for(const [key,value]of Object.entries(data?.theme?.mydash||{}))if(Object.hasOwn(palette,'--'+key)&&/^#[0-9a-f]{6,8}$/i.test(value))palette['--'+key]=value;
 Object.assign(palette,{'--sidebar':palette['--panel'],'--accent':palette['--iris'],'--green':palette['--foam'],'--subtle':palette['--muted'],'--review':palette['--iris'],'--progress':palette['--gold'],'--blocked':palette['--love']});
 for(const [key,value]of Object.entries(palette))document.documentElement.style.setProperty(key,value);document.documentElement.dataset.colorScheme=data?.type||'dark';document.documentElement.style.colorScheme=data?.type||'dark';
}
function resolvedTheme(){return setting('theme','rose-pine')==='system'?(matchMedia('(prefers-color-scheme: dark)').matches?'midnight':'light'):setting('theme','rose-pine');}
function terminalSettings(){const root=getComputedStyle(document.documentElement),color=k=>root.getPropertyValue(k).trim();return {fontSize:setting('terminalFontSize',12),scrollback:setting('terminalScrollback',3000),theme:{background:color('--bg')||'#191724',foreground:color('--text')||'#e0def4',cursor:color('--text')||'#e0def4',cursorAccent:color('--bg')||'#191724',selectionBackground:color('--line')||'#403d52',black:color('--panel'),red:color('--love'),green:color('--foam'),yellow:color('--gold'),blue:color('--foam'),magenta:color('--iris'),cyan:color('--rose'),white:color('--text'),brightBlack:color('--muted'),brightRed:color('--love'),brightGreen:color('--foam'),brightYellow:color('--gold'),brightBlue:color('--foam'),brightMagenta:color('--iris'),brightCyan:color('--rose'),brightWhite:color('--text')}};}
function shortcutLabel(chord){return (chord||'').replace('Primary+',/Mac/.test(navigator.platform)?'⌘':'Ctrl+').replace('Shift+','⇧').replace('Alt+','⌥').replace('Enter','↵');}
function applyShortcutHint(button,id){
 const chord=setting('keybindings',{})[id]||'',hint=button.querySelector('kbd');
 if(hint){hint.textContent=shortcutLabel(chord);hint.hidden=!chord;}
 if(chord)button.setAttribute('aria-keyshortcuts',chord.replace('Primary',/Mac/.test(navigator.platform)?'Meta':'Control'));
 else button.removeAttribute('aria-keyshortcuts');
}
function applyAppSettings(){
 document.documentElement.dataset.theme=resolvedTheme();applyTheme();document.documentElement.dataset.density=setting('density','compact');document.documentElement.style.setProperty('--ui-scale',serviceClient.isDesktop?1:setting('uiScale',100)/100);document.documentElement.style.setProperty('--diff-font-size',setting('diffFontSize',10)+'px');document.documentElement.style.setProperty('--diff-line-height',Math.ceil(setting('diffFontSize',10)*1.6)+'px');
 const terminalOptions=terminalSettings();for(const view of terminalViews.values()){Object.assign(view.terminal.options,terminalOptions);view.resize?.();}
 $('#quick-agent').value=setting('defaultAgent','claude');updateQuickAgent();
 const native=window.__TAURI__?.core;if(native&&appSettings)void native.invoke('apply_native_settings',{values:appSettings.values,light:themeDefinition()?.type==='light'}).catch(error=>toast('Native shortcuts could not update: '+String(error)));
 for(const node of document.querySelectorAll('[data-command-id]'))node.textContent=shortcutLabel(setting('keybindings',{})[node.dataset.commandId]);
 for(const node of document.querySelectorAll('#sources [data-source]'))applyShortcutHint(node,'view-'+node.dataset.source);
 applyShortcutHint($('#activity-open'),'view-sessions');applyShortcutHint($('#command-open'),'commands');applyShortcutHint($('#new-note'),'agent-new');$('#new-note').title='New chat · '+shortcutLabel(setting('keybindings',{})['agent-new']);
 const hint=$('.search kbd');if(hint){hint.textContent='/ · '+shortcutLabel(setting('keybindings',{})['page-search']);hint.title='Search this page';}
 renderQuickActions();window.dispatchEvent(new Event('mydash-settings-changed'));
}
async function loadAppSettings({canApply=()=>true}={}){
 const identity=appSettings,catalog=window.mydashThemeCatalog,current=()=>appSettings===identity&&canApply();
 try{const next=await api('/api/settings');let themes;try{themes=(await api('/api/themes')).themes;}catch(error){next.configError=error.message;}
  if(!current())return null;
  const changed=JSON.stringify(next)!==JSON.stringify(appSettings)||(themes&&JSON.stringify(themes)!==JSON.stringify(window.mydashThemeCatalog));appSettings=next;
  if(themes)window.mydashThemeCatalog=themes;
  if(changed)try{applyAppSettings();}catch(error){if(appSettings===next){appSettings=identity;if(themes&&window.mydashThemeCatalog===themes)window.mydashThemeCatalog=catalog;}throw error;}
  if(next.configError&&canApply()&&$('#settings-dialog').open)$('#settings-error').textContent=next.configError;return next;
 }catch(error){if((appSettings===identity&&canApply())&&$('#settings-dialog').open)$('#settings-error').textContent=error.message;}
}
function focusPageSearch(){
 const dialog=[...document.querySelectorAll('dialog[open]')].at(-1);
 const input=dialog?dialog.querySelector('input[type="search"]'):source==='chat-results'?document.querySelector('.chat-search-form input'):$('#search');
 if(!input||!input.getClientRects().length)return false;
 input.focus();input.select();return true;
}
document.addEventListener('keydown',event=>{if(event.key==='/'&&!event.metaKey&&!event.ctrlKey&&!event.altKey&&!event.isComposing&&!event.target.closest('input,textarea,select,[contenteditable="true"],.xterm')&&focusPageSearch()){event.preventDefault();event.stopImmediatePropagation();}},true);
function chordFromEvent(event){if(!(event.metaKey||event.ctrlKey)||(event.metaKey&&event.ctrlKey)||event.isComposing)return '';let key=event.key.length===1?event.key.toUpperCase():event.key;if(event.altKey&&/^Key[A-Z]$/.test(event.code||''))key=event.code.slice(3);if(event.shiftKey){if(/^Digit[0-9]$/.test(event.code||''))key=event.code.slice(5);else if(['Comma','Period','Slash'].includes(event.code))key={Comma:',',Period:'.',Slash:'/'}[event.code];}return 'Primary+'+(event.shiftKey?'Shift+':'')+(event.altKey?'Alt+':'')+key;}
function canonicalShortcut(value){
 const parts=value.trim().split('+');if(!value.trim())return '';
 const key=parts.pop(),modifiers=parts.map(part=>part.toLowerCase());
 if(modifiers[0]!=='primary'||new Set(modifiers).size!==modifiers.length||modifiers.some(part=>!['primary','shift','alt'].includes(part)))return null;
 const normalizedKey=key.toLowerCase()==='enter'?'Enter':key.toUpperCase();
 if(!/^(?:[A-Z0-9,./]|Enter)$/.test(normalizedKey))return null;
 return 'Primary+'+(modifiers.includes('shift')?'Shift+':'')+(modifiers.includes('alt')?'Alt+':'')+normalizedKey;
}
function shortcutValidationMessage(value,id){
 const chord=canonicalShortcut(value);
 if(chord===null)return 'Use Primary, optional Shift/Alt, and a letter, number, Enter, comma, period or slash.';
 if(['Primary+C','Primary+V','Primary+X','Primary+A','Primary+Z','Primary+Shift+Z','Primary+Q'].includes(chord))return 'This shortcut is reserved for editing or quitting.';
 const conflict=chord&&Object.entries(settingsDraft.keybindings).find(([otherId,binding])=>otherId!==id&&canonicalShortcut(binding)===chord);
 return conflict?'Already assigned to '+(appSettings.commands.find(command=>command.id===conflict[0])?.label||conflict[0])+'. Choose another shortcut.':'';
}
function recordedShortcut(event){
 if(event.isComposing||['Meta','Control','Shift','Alt'].includes(event.key))return '';
 const chord=chordFromEvent(event);
 if(!chord||canonicalShortcut(chord)===null)throw new Error('Use Command/Control with a letter, number, Enter, comma, period or slash.');
 if(['Primary+C','Primary+V','Primary+X','Primary+A','Primary+Z','Primary+Shift+Z','Primary+Q'].includes(chord))throw new Error('This shortcut is reserved for editing or quitting.');
 return chord;
}
function syncNativeShortcutRecording(recording){
 if(!serviceClient.isDesktop)return Promise.resolve();
 shortcutRecordingSync=shortcutRecordingSync.catch(()=>{}).then(()=>window.__TAURI__.core.invoke('set_shortcut_recording',{recording}));return shortcutRecordingSync;
}
async function finishShortcutRecording(){
 const current=shortcutRecording;if(!current)return;shortcutRecording=null;
 current.button.textContent='Record';current.button.setAttribute('aria-pressed','false');current.input.readOnly=false;
 $('#settings-status').textContent=settingsDirty?'Unsaved changes':'Saved on this computer';
 try{await syncNativeShortcutRecording(false);}catch(error){$('#settings-error').textContent='Could not restore app shortcuts: '+String(error);}
 filterSettings();
}
async function startShortcutRecording(button,input,id){
 if(settingsSaving)return;
 if(shortcutRecording?.button===button){await finishShortcutRecording();return;}
 void finishShortcutRecording();
 const current={button,input,id};shortcutRecording=current;button.textContent='Cancel';button.setAttribute('aria-pressed','true');input.readOnly=true;input.focus();
 $('#settings-error').textContent='';$('#settings-status').textContent='Press a shortcut. Escape cancels; Tab leaves recording.';
 try{await syncNativeShortcutRecording(true);}catch(error){if(shortcutRecording===current)await finishShortcutRecording();$('#settings-error').textContent='Could not record shortcut: '+String(error);}
}
window.addEventListener('keydown',event=>{
 if(settingsSaving||!shortcutRecording||event.isComposing)return;
 if(event.key==='Tab'){void finishShortcutRecording();return;}
 event.preventDefault();event.stopImmediatePropagation();
 if(event.key==='Escape'){void finishShortcutRecording();$('#settings-error').textContent='';return;}
 try{const chord=recordedShortcut(event);if(!chord)return;const current=shortcutRecording,error=shortcutValidationMessage(chord,current.id);if(error)throw new Error(error);current.input.value=chord;settingsDraft.keybindings[current.id]=chord;updateSettingsDirty();$('#settings-error').textContent='';void finishShortcutRecording();}catch(error){$('#settings-error').textContent=error.message;}
},true);
window.addEventListener('blur',()=>void finishShortcutRecording());
function runConfiguredCommand(id){
 if($('#settings-dialog').open&&(settingsDirty||settingsSaving)&&id!=='settings'&&id!=='page-search'){$('#settings-error').textContent='Save your changes, or use Discard before leaving Settings.';return;}
 if(id==='page-search'){focusPageSearch();return;}
 if(id==='settings'){openSettings();return;}
 if(id==='commands'){openCommands();return;}
 if(id==='find-work'){if($('#command-dialog').open)requestDialogClose($('#command-dialog'));else openPalette();return;}
 if(document.querySelector('dialog[open]'))return;
 if(id==='toggle-left'||id==='toggle-right'){toggleSidebar(id==='toggle-left'?'left':'right');return;}
 if(['agent-new','agent-close','agent-reopen','agent-rename'].includes(id)){void runTabCommand(id.slice(6));return;}
 if(id==='agent-location'){openLocationPrompt();return;}
 if(id==='chat-terminal'){if(source==='agents')void toggleChatTerminal();return;}
 if(id==='terminal-focus'){if(source==='agents'){terminalFocus=!terminalFocus;render();}return;}
 if(id.startsWith('view-')){source=id.slice(5);if(typeof savedViewUsePage==='function')savedViewUsePage(source);else{if(source==='github')githubView='all';if(source==='linear')scopePrefs.linear='';}persist();render();}
}
document.addEventListener('keydown',event=>{if(!appSettings||event.target.closest('.shortcut-input'))return;const chord=chordFromEvent(event),id=Object.keys(appSettings.values.keybindings).find(id=>appSettings.values.keybindings[id]&&appSettings.values.keybindings[id].toUpperCase()===chord.toUpperCase());if(id){if(id==='page-search'&&!focusPageSearch())return;event.preventDefault();event.stopImmediatePropagation();if(id!=='page-search')runConfiguredCommand(id);}else if(/^Primary\+[1-9]$/.test(chord)&&!document.querySelector('dialog[open]')){const candidate=visibleAgentCandidates()[Number(chord.at(-1))-1];if(candidate){event.preventDefault();event.stopImmediatePropagation();selectAgent(recordForCandidate(candidate),candidate);}}},true);
function settingsButton(label,action,className='button'){
 const button=featureButton(label,action,className);button.type='button';
 if(/(?:^| )(?:button|text-button|icon-button)(?: |$)/.test(className)){
  button.classList.add('ui-control');button.dataset.variant=className.includes('primary')?'primary':className.includes('danger')?'destructive':'secondary';
  if(className.includes('icon-button'))button.dataset.size='icon';
 }
 return button;
}
function settingsReset(key,label,{commandId}={}){
 const value=commandId?appSettings.defaults.keybindings[commandId]:appSettings.defaults[key];
 const description=typeof value==='boolean'?(value?'On':'Off'):value===''?'Unassigned':String(value);
 const reset=settingsButton('↶',()=>{
  if(settingsSaving)return;const owned=reset.parentElement.contains(document.activeElement),input=document.getElementById(commandId?'shortcut-'+commandId:'preference-'+key);
  if(commandId)settingsDraft.keybindings[commandId]=value;else settingsDraft[key]=structuredClone(value);
  if(input){if(input.type==='checkbox')input.checked=value;else input.value=value;}
  if(commandId&&shortcutRecording?.id===commandId)void finishShortcutRecording();
  if(owned)document.activeElement.blur();updateSettingsDirty();
  if(owned&&(document.activeElement===document.body||!document.activeElement?.isConnected)){if(input&&!input.closest('[hidden]'))input.focus({preventScroll:true});else $('#settings-modified').focus({preventScroll:true});}
 },'icon-button preference-reset');
 reset.dataset.settingsReset=commandId||key;if(commandId)reset.dataset.shortcutReset='true';
 reset.setAttribute('aria-label','Reset '+label+' to default: '+description);reset.title='Reset to default: '+description;
 return reset;
}
function settingsValueModified(key,commandId){return JSON.stringify(commandId?settingsDraft.keybindings[commandId]:settingsDraft[key])!==JSON.stringify(commandId?appSettings.defaults.keybindings[commandId]:appSettings.defaults[key]);}
function settingsRowModified(row){return Boolean(row.dataset.settingsKey)&&settingsValueModified(row.dataset.settingsKey,row.dataset.settingsCommand);}
function settingsDraftButton(label,action,className='button'){
 const button=settingsButton(label,()=>{if(!settingsSaving)return action();},className);button.dataset.settingsDraftControl='true';button.disabled=settingsSaving;return button;
}
function updateSettingsDirty(){
 settingsDirty=JSON.stringify(settingsDraft)!==JSON.stringify(settingsBaseline?.values??appSettings.values);
 for(const control of document.querySelectorAll('#settings-dialog [data-settings-draft-control]'))control.disabled=settingsSaving;
 let invalid=false;
 for(const input of document.querySelectorAll('.shortcut-input')){
  const error=shortcutValidationMessage(input.value,input.dataset.commandId);input.setCustomValidity(error);input.setAttribute('aria-invalid',String(Boolean(error)));input.title=error||'Type a shortcut or choose Record';invalid||=Boolean(error);
  const message=document.getElementById(input.id+'-error');if(message){message.textContent=error;message.hidden=!error;const descriptions=new Set((input.getAttribute('aria-describedby')||'').split(/\s+/).filter(Boolean));if(error)descriptions.add(message.id);else descriptions.delete(message.id);if(descriptions.size)input.setAttribute('aria-describedby',[...descriptions].join(' '));else input.removeAttribute('aria-describedby');}
 }
 for(const reset of document.querySelectorAll('[data-settings-reset]')){
  const key=reset.dataset.settingsReset,shortcut=reset.dataset.shortcutReset;
  reset.disabled=settingsSaving||!settingsValueModified(key,shortcut?key:null);
 }
 $('#settings-save').disabled=!settingsDirty||settingsSaving||invalid;$('#settings-status').textContent=settingsSaving?'Saving…':invalid?'Resolve shortcut errors before saving':settingsDirty?'Unsaved changes':'Saved on this computer';
 filterSettings();
}
function settingInput(field){
 const [category,key,label,type,options,max,unit]=field,row=el('div','preference-row');
 const heading=el('label','preference-label'),name=el('span','preference-name',label),description=appSettings.fields.find(f=>f.key===key)?.description;
 row.dataset.settingsKey=key;row.addEventListener('focusout',()=>queueMicrotask(filterSettings));row.dataset.search=(label+' '+category+' '+(description||'')).toLowerCase();heading.htmlFor='preference-'+key;heading.append(name);if(description)heading.append(el('small','preference-description',description));let input;
 if(type==='select'){input=el('select');for(const [value,text]of options)input.add(new Option(text,value));input.value=settingsDraft[key];}
 else{input=el('input');input.type=type;if(type==='number'){input.min=options;input.max=max;input.value=settingsDraft[key];}else if(type==='checkbox')input.checked=settingsDraft[key];else{input.value=settingsDraft[key];input.placeholder=options;input.maxLength=100;}}
 if(type!=='checkbox')input.classList.add('ui-field');input.id='preference-'+key;input.name=key;input.dataset.settingsDraftControl='true';input.setAttribute('aria-label',label);input.oninput=()=>{if(settingsSaving)return;settingsDraft[key]=type==='checkbox'?input.checked:type==='number'?Number(input.value):input.value;updateSettingsDirty();};
 const controls=el('span','preference-control');controls.append(input);if(key==='claudeReviewCommand'||key==='codexReviewCommand')controls.append(settingsDraftButton('Browse',()=>browseReviewSkill(key),'button'));if(unit)controls.append(el('small','',unit));controls.append(settingsReset(key,label));row.append(heading,controls);return row;
}
function shortcutInput(command){
 const row=el('div','preference-row'),label=el('label','preference-label',command.label),controls=el('span','preference-control'),input=el('input','shortcut-input ui-field');
 row.dataset.settingsKey='keybindings';row.dataset.settingsCommand=command.id;row.addEventListener('focusout',()=>queueMicrotask(filterSettings));row.dataset.search=command.label.toLowerCase();input.id='shortcut-'+command.id;label.htmlFor=input.id;input.dataset.commandId=command.id;input.value=settingsDraft.keybindings[command.id];input.placeholder='Unassigned';input.spellcheck=false;input.setAttribute('autocapitalize','off');input.setAttribute('aria-label',command.label+' shortcut');
 input.dataset.settingsDraftControl='true';input.oninput=()=>{if(settingsSaving)return;settingsDraft.keybindings[command.id]=canonicalShortcut(input.value)??input.value.trim();updateSettingsDirty();};
 input.addEventListener('blur',()=>{if(shortcutRecording?.input===input)void finishShortcutRecording();const chord=canonicalShortcut(input.value);if(chord!==null)input.value=chord;});
 const record=settingsDraftButton('Record',()=>void startShortcutRecording(record,input,command.id),'button shortcut-record');record.setAttribute('aria-label','Record '+command.label+' shortcut');record.setAttribute('aria-pressed','false');
 record.addEventListener('pointerdown',event=>{if(shortcutRecording?.button===record)event.preventDefault();});
 const error=el('small','form-hint shortcut-error');error.id=input.id+'-error';error.hidden=true;error.setAttribute('aria-live','polite');label.append(error);
 controls.append(input,record,settingsReset('keybindings',command.label+' shortcut',{commandId:command.id}));row.append(label,controls);return row;
}
function settingsPanel(id,title){const panel=el('section','preferences-panel');panel.dataset.category=id;panel.append(el('h2','',title));return panel;}
function paintSettingsBody(){for(const setup of document.querySelectorAll('#settings-body .connection-setup'))setup.disposeConnectionSetup?.();void finishShortcutRecording();const nav=$('.preferences-nav'),navHead=$('.preferences-nav-head');nav.replaceChildren();if(navHead)nav.append(navHead);for(const [id,label]of settingsCategories()){const button=settingsButton(label,()=>{settingsCategory=id;resetSettingsFilter();filterSettings();$('#settings-body').scrollTop=0;},'settings-category');button.dataset.settingsCategory=id;button.prepend(el('span','settings-chevron','›'));nav.append(button);}const body=$('#settings-body'),legacy=$('#legacy-connections');if(legacy)legacy.remove();body.replaceChildren();
 for(const [id,label]of settingsCategories()){const panel=settingsPanel(id,label);
 for(const field of settingFields().filter(f=>f[0]===id))panel.append(settingInput(field));
 if(id==='review')panel.append(el('p','form-hint','Font size applies to full review and file previews. Shift+Enter inserts a newline when Enter submits comments.'));
 if(id==='agents')panel.append(el('p','form-hint','Leave a review skill empty to disable agent review. Review follows the selected chat across its repositories. Skills define the reviewer roles and count. Zed opens diffs; VS Code and Cursor open the selected worktree. Changes apply on the next editor action.'));
 if(id==='keyboard'){panel.append(el('p','form-hint','Type a shortcut (for example Primary+K), or choose Record and press the keys. Primary is Command on macOS and Control elsewhere. Leave a shortcut empty to disable it. Tab / Shift+Tab cycle chat tabs; 1–9 with Primary select a tab. Editing shortcuts are reserved.'));
 for(const command of appSettings.commands)panel.append(shortcutInput(command));}
 if(id==='actions'){panel.append(settingsButton('View recent action output',()=>openActionOutput(), 'button'));panel.append(el('p','form-hint','Quick actions run a local zsh command when you click Run. They have no interactive terminal and stop after 10 minutes. Review commands before saving them. Their output stays in this section.'));
 const help=el('details','settings-help');help.append(el('summary','','Active-chat placeholders'));help.append(el('p','','Use shell environment variables, quoted as "$CHAT_CWD". Available: $CHAT_ID, $CHAT_TITLE, $CHAT_CWD, $CHAT_AGENT, $CHAT_SESSION_ID, $WORKTREE_PATH, $PR_URL, $PR_NUMBER, $TICKET_URL, $TICKET_ID. If multiple work items match, choose one before running. No text is substituted into the command.'));panel.append(help);
 for(const action of settingsDraft.quickActions){const row=el('section','quick-action-editor'),title=el('input'),command=el('textarea');title.dataset.settingsDraftControl='true';command.dataset.settingsDraftControl='true';title.value=action.title;title.placeholder='Action name';title.setAttribute('aria-label','Quick action name');command.value=action.command;command.placeholder='git -C "$CHAT_CWD" status --short';command.rows=3;command.spellcheck=false;command.setAttribute('autocorrect','off');command.setAttribute('autocapitalize','off');command.setAttribute('writingsuggestions','false');command.setAttribute('aria-label','Quick action command');title.oninput=()=>{if(settingsSaving)return;action.title=title.value;updateSettingsDirty();};const warning=el('p','form-hint');warning.setAttribute('role','status');const checkCommand=()=>{warning.textContent=/[“”‘’—–]/.test(command.value)?'This command contains typographic quotes or dashes. Shell commands usually need straight quotes and ordinary hyphens.':'';};checkCommand();command.oninput=()=>{if(settingsSaving)return;action.command=command.value;checkCommand();updateSettingsDirty();};row.append(warning);row.append(title,command,settingsDraftButton('Remove',()=>{settingsDraft.quickActions=settingsDraft.quickActions.filter(a=>a!==action);paintSettingsBody();updateSettingsDirty();},'button danger'));panel.append(row);}
 panel.append(settingsDraftButton('Add quick action',()=>{settingsDraft.quickActions.push({id:crypto.randomUUID(),title:'',command:''});paintSettingsBody();updateSettingsDirty();},'button'));const output=el('div');output.id='quick-action-runs';panel.append(output);void loadQuickActionRuns();}
 if(id==='connections'){const setup=el('div');panel.append(setup);void renderConnectionSetup(setup).catch(error=>{setup.textContent=error.message||String(error);});if(legacy){const help=el('details','settings-help');help.append(el('summary','','Existing CLI and environment connections'),legacy);panel.append(help);}}
 if(id==='help'){void renderGuidanceSettings(panel);}
 if(id==='sharing'){void renderSharingSettings(panel);panel.append(el('p','','Share selected context through files or company storage. Storage access controls determine who can read or write it. An inbox name is a routing label, not verified identity.'));}
 if(id==='data'){panel.append(settingsButton('Resume setup',()=>window.openAppChatSetup().catch(error=>toast(error.message)),'button'));for(const [name,path]of Object.entries(appSettings.files||{})){const row=el('div','settings-file-row');row.append(el('strong','',name),el('code','',path),settingsButton('Copy path',()=>serviceClient.copyText(path),'text-button'));panel.append(row);}panel.append(el('p','','Preferences are stored separately from chats and work history. Export a settings profile to move preferences to another installation. It contains quick-action commands but no connection credentials.'));panel.append(settingsButton('Export settings profile',exportSettingsProfile,'button'),settingsDraftButton('Import settings profile',()=>$('#settings-import').click(),'button'),settingsButton('Export workspace JSON',()=>$('#export').click(),'button'));panel.append(el('p','form-hint','Workspace JSON is a metadata export; local artifact files and credentials require separate backup.'));panel.append(el('code','settings-client-example','mydash-desktop --home "$MYDASH_HOME" settings --show'));panel.append(el('p','form-hint','Agents can read settings and apply routine patches at the current revision. Command, editor and review-workflow changes require you to save them here.'));}
 body.append(panel);
 }
 filterSettings();
}
function settingsFilterContext(){
 const search=$('#settings-search'),modified=$('#settings-modified');if(!search||!modified||typeof search.value!=='string')return null;
 const pressed=modified.getAttribute('aria-pressed');if(pressed!=='true'&&pressed!=='false')return null;
 let query='',queryTruncated=false,count=0;for(const character of search.value){if(count===200){queryTruncated=true;break;}query+=character;count++;}
 return {query,queryTruncated,modifiedOnly:pressed==='true',preferencesDirty:Boolean(settingsDirty)};
}
function resetSettingsFilter(){$('#settings-search').value='';$('#settings-modified').setAttribute('aria-pressed','false');}
function settingsFilterControls(search){
 const bar=el('div','settings-filterbar'),modified=settingsButton('Modified only',()=>{modified.setAttribute('aria-pressed',String(modified.getAttribute('aria-pressed')!=='true'));filterSettings();},'button settings-modified');modified.id='settings-modified';modified.setAttribute('aria-pressed','false');modified.title='Show preferences and shortcuts that differ from defaults';
 const scope=el('p','form-hint settings-filter-note','Individual preferences and shortcuts only. Quick actions, Connections and Sharing forms are not included.');scope.id='settings-filter-scope';scope.hidden=true;modified.setAttribute('aria-describedby',scope.id);
 const status=el('p','form-hint settings-filter-note');status.id='settings-filter-status';status.setAttribute('role','status');status.hidden=true;bar.append(search,modified);return [bar,scope,status];
}
function filterSettings(){
 const query=$('#settings-search').value.toLowerCase().trim(),modified=$('#settings-modified').getAttribute('aria-pressed')==='true';let visible=0;
 for(const panel of document.querySelectorAll('.preferences-panel')){
  const rows=[...panel.querySelectorAll('.preference-row')];
  if(modified){for(const row of rows){const pinned=row.contains(document.activeElement)||Boolean(shortcutRecording&&row.contains(shortcutRecording.input));row.hidden=!pinned&&(!settingsRowModified(row)||!(row.dataset.search||row.textContent.toLowerCase()).includes(query));if(!row.hidden)visible++;}panel.hidden=!rows.some(row=>!row.hidden);}
  else{panel.hidden=!query&&panel.dataset.category!==settingsCategory;if(query){panel.hidden=!panel.textContent.toLowerCase().includes(query);for(const row of rows)row.hidden=!(row.dataset.search||row.textContent.toLowerCase()).includes(query);}else for(const row of rows)row.hidden=false;}
 }
 for(const button of document.querySelectorAll('[data-settings-category]'))button.setAttribute('aria-current',String(button.dataset.settingsCategory===settingsCategory&&!query&&!modified));
 $('#settings-filter-scope').hidden=!modified;const status=$('#settings-filter-status');status.hidden=!modified||visible>0;status.textContent=modified&&visible===0?'No modified preferences or shortcuts match.':'';
}
function resetSettingsDraft(document){settingsBaseline={revision:document.revision,contentHash:document.contentHash,values:structuredClone(document.values)};settingsDraft=structuredClone(document.values);settingsDirty=false;}
async function openPreferences({category}={}){
 if(settingsSaving)return false;const dialog=$('#settings-dialog');if(!dialog?.isConnected)return false;
 if(dialog.open){
  if([...document.querySelectorAll('dialog[open]')].some(open=>open!==dialog&&!open.contains(dialog)))return false;
  if(category!==undefined){settingsCategory=category;resetSettingsFilter();filterSettings();$('#settings-body').scrollTop=0;}else $('#settings-search').focus();
  return true;
 }
 if(!requestDialogNavigation())return false;
 const generation=++settingsOpenGeneration,baseline=settingsBaseline,draft=settingsDraft,stamp=JSON.stringify(draft),previousCategory=settingsCategory,navigation=captureDialogNavigation(),current=()=>generation===settingsOpenGeneration&&$('#settings-dialog')===dialog&&dialog.isConnected&&!settingsSaving&&settingsBaseline===baseline&&settingsDraft===draft&&JSON.stringify(draft)===stamp&&settingsCategory===previousCategory&&navigation.isCurrent();
 let next;try{next=await loadAppSettings({canApply:current});}finally{navigation.dispose();}
 if(!next||!current())return false;
 resetSettingsDraft(next);if(category!==undefined)settingsCategory=category;$('#settings-error').textContent=next.configError||'';resetSettingsFilter();paintSettingsBody();updateSettingsDirty();dialog.showModal();return true;
}
async function savePreferences(){
 if(settingsSaving)return;
 for(const [id,value]of Object.entries(settingsDraft.keybindings)){const error=shortcutValidationMessage(value,id);if(error){$('#settings-error').textContent=error;return;}settingsDraft.keybindings[id]=canonicalShortcut(value);}
 const baseline=settingsBaseline,current=settingsDraft,submitted=structuredClone(current),stamp=JSON.stringify(current),focus=document.activeElement;settingsSaving=true;updateSettingsDirty();$('#settings-error').textContent='';
 try{
  await finishShortcutRecording();
  if(settingsBaseline!==baseline)throw new Error('Settings were reloaded while saving. Review them and try again.');
  appSettings=await api('/api/settings',{revision:baseline.revision,contentHash:baseline.contentHash,patch:submitted});
  if(settingsDraft===current&&JSON.stringify(settingsDraft)===stamp){resetSettingsDraft(appSettings);applyAppSettings();paintSettingsBody();}
  else{settingsBaseline={revision:appSettings.revision,contentHash:appSettings.contentHash,values:structuredClone(appSettings.values)};applyAppSettings();$('#settings-error').textContent='Earlier settings were saved. Your newer draft is still unsaved.';}
 }catch(error){$('#settings-error').textContent=error.message;}
 finally{settingsSaving=false;updateSettingsDirty();if($('#settings-dialog').open&&document.activeElement===document.body){const target=focus?.id?document.getElementById(focus.id):focus;if(target?.isConnected&&!target.disabled)target.focus({preventScroll:true});}}
}
async function discardSettingsDraft(){
 if(settingsSaving)return;const baseline=settingsBaseline,current=settingsDraft,stamp=JSON.stringify(current),unchanged=()=>!settingsSaving&&settingsBaseline===baseline&&settingsDraft===current&&JSON.stringify(current)===stamp;
 const next=await loadAppSettings({canApply:unchanged});if(!next||next.configError||!unchanged())return;resetSettingsDraft(next);paintSettingsBody();updateSettingsDirty();$('#settings-error').textContent='';
}
function mergeSettingsProfile(current,profile){
 const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
 if(!object(profile)||profile.format!=='mydash-settings'||profile.version!==1||!object(profile.values)||Object.keys(profile).some(key=>!['format','version','values'].includes(key)))throw new Error('Unsupported settings profile. Nothing was loaded.');
 for(const [key,value]of Object.entries(profile.values)){
  if(!Object.hasOwn(current,key))throw new Error('Profile contains an unsupported setting. Nothing was loaded.');
  if(key==='keybindings'){
   if(!object(value)||Object.entries(value).some(([id,chord])=>!Object.hasOwn(current.keybindings,id)||typeof chord!=='string'))throw new Error('Profile contains unsupported keyboard settings. Nothing was loaded.');
  }else if(key==='quickActions'){
   if(!Array.isArray(value)||value.some(action=>!object(action)||Object.keys(action).some(field=>!['id','title','command'].includes(field))||['id','title','command'].some(field=>typeof action[field]!=='string')))throw new Error('Profile contains unsupported quick actions. Nothing was loaded.');
  }else if(Array.isArray(current[key])?!Array.isArray(value):typeof value!==typeof current[key]||value===null){
   throw new Error('Profile contains an invalid setting type. Nothing was loaded.');
  }
 }
 const next={...structuredClone(current),...structuredClone(profile.values)};
 if(Object.hasOwn(profile.values,'keybindings'))next.keybindings={...current.keybindings,...profile.values.keybindings};
 return next;
}
function exportSettingsProfile(){const blob=new Blob([JSON.stringify({format:'mydash-settings',version:1,values:settingsDraft},null,2)],{type:'application/json'}),url=URL.createObjectURL(blob),a=el('a');a.href=url;a.download='mydash-settings.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
function renderQuickActions(){let root=$('#custom-quick-actions');if(!root){root=el('div');root.id='custom-quick-actions';root.setAttribute('aria-label','Quick actions');$('.utility-nav').before(root);}
 root.replaceChildren();for(const action of setting('quickActions',[])){const identity={revision:appSettings.revision,contentHash:appSettings.contentHash},button=settingsButton(action.title,()=>chooseQuickAction(action,identity),'nav-item quick-action-button');button.title=action.command;const icon=el('span','sidebar-icon');icon.append(sidebarGlyph('commands'));button.prepend(icon);root.append(button);}}
async function browseReviewSkill(key){
 if(settingsSaving)return;
 const dialog=$('#quick-action-picker'),form=$('#quick-action-picker-form'),settings=$('#settings-dialog'),input=document.getElementById('preference-'+key),draft=settingsDraft,baseline=settingsBaseline,previous=draft[key],trigger=document.activeElement,agent=key==='codexReviewCommand'?'codex':'claude';
 if(!input?.isConnected||!settings.open)return;
 let live=true;for(const owner of [dialog,settings])owner.addEventListener('close',()=>{if(!owner.open)live=false;},{once:true});
 form.replaceChildren(el('h2','','Choose '+(agent==='claude'?'Claude':'Codex')+' review skill'));
 const query=el('input'),results=el('div','skill-picker-results'),status=el('p','form-hint','Loading installed skills…');query.type='search';query.placeholder='Find an installed skill…';query.setAttribute('aria-label','Find review skill');form.append(query,status,results);
 const current=()=>live&&dialog.open&&dialog.isConnected&&settings.open&&settings.isConnected&&form.contains(query)&&document.getElementById('preference-'+key)===input&&settingsDraft===draft&&settingsBaseline===baseline&&draft[key]===previous&&input.value===previous&&!settingsSaving;
 const close=settingsButton('Cancel',()=>dialog.close(),'button');close.type='button';form.append(close);form.onsubmit=e=>e.preventDefault();dialog.showModal();query.focus();
 try{
  const catalog=await api('/api/skills/catalog',{cwd:currentAgentRecord()?.cwd||''});if(!current())return;
  const draw=()=>{
   if(!current())return;results.replaceChildren();const skills=skillPickerMatches(catalog.skills,query.value,agent).filter(s=>s.agents.includes(agent));status.textContent=skills.length?'Choose a skill. Save settings to apply it.':'No matching installed skills. You can enter its command directly.';
   for(const skill of skills){const button=settingsDraftButton('',()=>{
    if(!current()||[...document.querySelectorAll('dialog[open]')].at(-1)!==dialog)return;
    const owned=dialog.contains(document.activeElement),focused=document.activeElement;
    draft[key]=(agent==='codex'?'$':'/')+(skill.commandName||skill.name);input.value=draft[key];updateSettingsDirty();dialog.close();
    if(owned&&settings.open&&settings.isConnected&&document.getElementById('preference-'+key)===input&&![...document.querySelectorAll('dialog[open]')].some(open=>open!==settings)&&(document.activeElement===focused||document.activeElement===document.body||!document.activeElement?.isConnected||document.activeElement===trigger)){
     const target=!input.closest('[hidden]')?input:$('#settings-modified')||$('#settings-search');target?.focus({preventScroll:true});
    }
   },'skill-picker-result');button.type='button';button.append(el('strong','',skill.name),el('span','skill-picker-description',skill.description),el('small','skill-picker-path',skill.path));results.append(button);}
  };query.oninput=draw;draw();
 }catch(error){if(current())status.textContent=error.message;}
}
async function chooseQuickAction(action,identity){const record=currentAgentRecord(),p={revision:identity.revision,contentHash:identity.contentHash,actionId:action.id,recordId:record?.id||'',cwd:record?selectedGitDirectory(record):''};const choices=[];for(const [field,source,prefix]of [['prId','github','PR'],['ticketId','linear','TICKET']])if(new RegExp('\\$\\{?'+prefix+'_').test(action.command)){const items=(record?.items||[]).filter(i=>i.source===source&&(source!=='github'||i.url?.includes('/pull/')));if(!items.length){toast('Select a chat with a linked '+(source==='github'?'PR':'ticket')+'.');return;}if(items.length===1)p[field]=items[0].id;else choices.push({field,items});}
 const start=async()=>{try{const job=await api('/api/commands/invoke',{name:'quick-actions.run',args:p});openActionOutput(job.id);}catch(error){toast(error.message);}};
 if(!choices.length){await start();return;}
 const dialog=$('#quick-action-picker'),form=$('#quick-action-picker-form');form.replaceChildren(el('h2','',action.title));for(const {field,items}of choices){const label=el('label','',field==='prId'?'Pull request':'Ticket'),select=el('select');for(const item of items)select.add(new Option((item.reference||'')+' · '+item.title,item.id));p[field]=select.value;select.onchange=()=>p[field]=select.value;label.append(select);form.append(label);}const controls=el('div','dialog-actions'),cancel=settingsButton('Cancel',()=>dialog.close(),'button');cancel.type='button';const run=el('button','button primary','Run action');run.type='submit';controls.append(cancel,run);form.append(controls);form.onsubmit=e=>{e.preventDefault();dialog.close();void start();};dialog.showModal();}
async function loadQuickActionRuns(){clearTimeout(quickActionTimer);const root=$('#quick-action-runs');if(!root)return;try{const {jobs}=await api('/api/quick-actions'),expanded=new Set([...root.querySelectorAll('details[open]')].map(d=>d.dataset.jobId));root.replaceChildren();for(const job of jobs.sort((a,b)=>b.startedAt-a.startedAt)){const details=el('details','quick-action-output'),summary=el('summary','',job.title+' · '+job.state);details.dataset.jobId=job.id;details.open=expanded.has(job.id)||job.state==='running';details.append(summary,el('pre','',job.output||'No output yet.'));root.append(details);}if(jobs.some(j=>j.state==='running')&&$('#settings-dialog').open)quickActionTimer=setTimeout(loadQuickActionRuns,1500);}catch(error){root.textContent=error.message;}}
window.addEventListener('mydash-configured-command',event=>runConfiguredCommand(event.detail));
openSettings=()=>openPreferences();$('#settings').onclick=openSettings;
const oldDialog=$('#settings-dialog'),legacy=el('div');legacy.id='legacy-connections';while(oldDialog.children.length>2)legacy.append(oldDialog.children[2]);oldDialog.replaceChildren();oldDialog.classList.add('preferences-page');
const heading=el('header','preferences-heading'),title=el('h1','','Settings'),search=el('input');search.id='settings-search';search.type='search';search.placeholder='Search settings…';search.setAttribute('aria-label','Search settings');search.oninput=filterSettings;const close=settingsButton('×',()=>{if(settingsDirty||settingsSaving){$('#settings-error').textContent='Save your changes, or use Discard before closing.';return;}oldDialog.close();},'icon-button');close.setAttribute('aria-label','Close settings');const scope=el('span','settings-scope','User'),context=el('span','settings-context','settings.json'),edit=settingsDraftButton('Edit settings.json',async()=>{try{if(serviceClient.isDesktop)await window.__TAURI__.core.invoke('open_settings_file');else{await serviceClient.copyText(appSettings.files.preferences);$('#settings-status').textContent='Settings path copied. Open it in your editor.';}}catch(error){$('#settings-error').textContent=error.message;}},'button settings-edit-file');heading.append(scope,context,edit,close);
const layout=el('div','preferences-layout'),nav=el('nav','preferences-nav');const navHead=el('div','preferences-nav-head');navHead.append(title,...settingsFilterControls(search));nav.setAttribute('aria-label','Settings categories');for(const [id,label]of settingsCategories()){const button=settingsButton(label,()=>{settingsCategory=id;resetSettingsFilter();filterSettings();$('#settings-body').scrollTop=0;},'settings-category');button.dataset.settingsCategory=id;button.prepend(el('span','settings-chevron','›'));nav.append(button);}const body=el('main','preferences-body');body.id='settings-body';body.append(legacy);layout.append(nav,body);
const footer=el('footer','preferences-footer'),status=el('span');status.id='settings-status';status.setAttribute('role','status');const error=el('span','inline-error');error.id='settings-error';error.setAttribute('role','alert');const discard=settingsDraftButton('Discard & reload files',discardSettingsDraft,'button');const save=settingsButton('Save changes',savePreferences,'button primary');save.id='settings-save';footer.append(status,error,discard,save);const settingsFrame=el('div','preferences-main');settingsFrame.append(heading,body);nav.prepend(navHead);layout.replaceChildren(nav,settingsFrame);oldDialog.append(layout,footer);oldDialog.addEventListener('close',()=>void finishShortcutRecording());oldDialog.addEventListener('cancel',e=>{if(settingsDirty){e.preventDefault();$('#settings-error').textContent='Save your changes, or use Discard before closing.';}});
const file=el('input');file.id='settings-import';file.type='file';file.accept='.json,application/json';file.hidden=true;file.dataset.settingsDraftControl='true';file.onchange=async()=>{const selected=file.files[0];if(!selected)return;if(settingsSaving){file.value='';return;}const current=settingsDraft,stamp=JSON.stringify(current),baseline=settingsBaseline,unchanged=()=>!settingsSaving&&settingsDraft===current&&JSON.stringify(current)===stamp&&settingsBaseline===baseline;try{if(selected.size>65536)throw new Error('Profile exceeds 64 KiB.');const profile=JSON.parse(await selected.text());if(!unchanged())throw new Error('Settings changed while reading the profile. Import it again.');const revision=baseline.revision,contentHash=baseline.contentHash,next=mergeSettingsProfile(current,profile);await api('/api/settings/preview',{revision,contentHash,patch:next});if(!unchanged())throw new Error('Settings changed while validating the profile. Import it again.');settingsDraft=next;paintSettingsBody();updateSettingsDirty();$('#settings-error').textContent='Profile loaded for review. Check Quick actions and review skills before saving; nothing has been applied.';}catch(error){$('#settings-error').textContent=error.message;}finally{file.value='';}};oldDialog.append(file);
const picker=el('dialog');picker.id='quick-action-picker';const pickerForm=el('form');pickerForm.id='quick-action-picker-form';picker.append(pickerForm);document.body.append(picker);
$('#gcp-login').hidden=true;
matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{if(setting('theme','')==='system')applyAppSettings();});
void loadAppSettings();window.addEventListener('mydash-service-restarted',()=>void loadAppSettings());setInterval(()=>{if(!settingsDirty&&!document.hidden)void loadAppSettings();},10000);
