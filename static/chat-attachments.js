'use strict';
(()=>{
 const form=$('.app-chat-form');if(!form)return;const input=$('textarea',form);if(!input)return;
 let images=[];
 const strip=el('div','chat-attachment-strip'),status=el('p','form-hint');status.setAttribute('role','status');status.hidden=true;strip.hidden=true;form.prepend(strip,status);
 const pick=el('input');pick.type='file';pick.accept='image/*';pick.multiple=true;pick.hidden=true;form.append(pick);
 const button=featureButton('Attach',()=>pick.click(),'text-button');button.title='Attach a screenshot · or paste an image';button.type='button';$('.app-chat-footer',form).prepend(button);
 function draw(){strip.replaceChildren();strip.hidden=!images.length;for(const [index,image]of images.entries()){const card=el('div','chat-attachment'),preview=el('img');preview.src='data:'+image.mediaType+';base64,'+image.data;preview.alt='Attached screenshot '+(index+1);const remove=featureButton('×',()=>{images.splice(index,1);draw();},'text-button');remove.type='button';remove.setAttribute('aria-label','Remove screenshot '+(index+1));card.append(preview,remove);strip.append(card);}if(images.length)strip.append(el('small','form-hint','Sent with your next message. Screenshots are not saved in chat history.'));input.dispatchEvent(new Event('input'));}
 async function encode(file){
  if(file.size>20*1024*1024)throw new Error('Choose an image under 20 MiB.');
  const bitmap=await createImageBitmap(file);try{const canvas=document.createElement('canvas'),scale=Math.min(1,1920/Math.max(bitmap.width,bitmap.height));canvas.width=Math.max(1,Math.round(bitmap.width*scale));canvas.height=Math.max(1,Math.round(bitmap.height*scale));const context=canvas.getContext('2d');context.fillStyle='#fff';context.fillRect(0,0,canvas.width,canvas.height);context.drawImage(bitmap,0,0,canvas.width,canvas.height);let url='';for(const quality of [.9,.75,.6,.45]){url=canvas.toDataURL('image/jpeg',quality);if(url.length<790000)return {mediaType:'image/jpeg',data:url.split(',')[1]};}throw new Error('This screenshot is too large. Crop it before attaching.');}finally{bitmap.close();}
 }
 async function add(files){if($('.app-chat-messages')?.getAttribute('aria-busy')==='true')return;status.hidden=true;try{for(const file of files){if(images.length>=2)throw new Error('Attach at most two screenshots per message.');if(!file.type.startsWith('image/'))continue;images.push(await encode(file));}draw();}catch(error){draw();status.hidden=false;status.textContent=error.message;}}
 input.addEventListener('paste',event=>{const files=[...(event.clipboardData?.items||[])].filter(item=>item.kind==='file'&&item.type.startsWith('image/')).map(item=>item.getAsFile()).filter(Boolean);if(files.length){event.preventDefault();void add(files);}});
 pick.onchange=()=>{void add([...pick.files]);pick.value='';};
 window.truffleChatImages={count:()=>images.length,payload:()=>images.map(image=>({...image})),clear:()=>{images=[];status.hidden=true;draw();}};
 new MutationObserver(()=>{button.disabled=$('.app-chat-messages')?.getAttribute('aria-busy')==='true';}).observe($('.app-chat-messages'),{attributes:true,attributeFilter:['aria-busy']});
})();
