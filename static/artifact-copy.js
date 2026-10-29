'use strict';
function artifactCSV(rows){return rows.map(row=>row.map(value=>{const text=value==null?'':String(value);return /[",\r\n]/.test(text)?'"'+text.replace(/"/g,'""')+'"':text;}).join(',')).join('\r\n')+'\r\n';}
function artifactMarkdown(rows){
  if(!rows.length)return '';
  const columns=Math.max(...rows.map(row=>row.length));
  const cell=value=>String(value??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/\\/g,'\\\\').replace(/\|/g,'\\|').replace(/\r\n|\r|\n/g,'<br>');
  const line=row=>'| '+Array.from({length:columns},(_,i)=>cell(row[i])).join(' | ')+' |';
  return [line(rows[0]),line(Array(columns).fill('---')),...rows.slice(1).map(line)].join('\n')+'\n';
}
async function clipboardPNG(blob){
  if(blob.type==='image/png')return blob;
  const bitmap=await createImageBitmap(blob);
  try{
    if(!bitmap.width||!bitmap.height||bitmap.width*bitmap.height>32000000)throw new Error('Image is too large to copy.');
    const canvas=document.createElement('canvas');canvas.width=bitmap.width;canvas.height=bitmap.height;
    const context=canvas.getContext('2d');if(!context)throw new Error('Image copying is unavailable.');context.drawImage(bitmap,0,0);
    return await new Promise((resolve,reject)=>canvas.toBlob(value=>value?resolve(value):reject(new Error('Could not encode clipboard image.')),'image/png'));
  }finally{bitmap.close();}
}
if(typeof module!=='undefined')module.exports={artifactCSV,artifactMarkdown};
