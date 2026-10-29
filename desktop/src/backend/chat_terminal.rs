use super::*;
pub fn open(repo:&Repository,p:&Value)->Result<Value>{
 let chat=sid(text(p,"recordId"))?;
 let saved=repo.transaction(|s|{
  let record=arr(&s["sessions"]).into_iter().find(|r|r["id"]==chat).ok_or(Error::new("Select an existing chat."))?;
  if !s["chatTerminals"].is_object(){s["chatTerminals"]=json!({"version":1,"byChat":{}});}
  if !s["chatTerminals"]["byChat"][&chat].is_object(){s["chatTerminals"]["byChat"][&chat]=json!({"id":uuid(),"chatId":chat,"sessionId":"","agent":"shell","title":"Terminal","cwd":directory(text(&record,"cwd"))?});}
  Ok(s["chatTerminals"]["byChat"][&chat].clone())
 })?;
 repo.shells.open_shell(&saved)?;repo.transaction(|s|{super::guidance::complete(s,"chat.shell");Ok(())})?;Ok(saved)
}
pub fn action(repo:&Repository,path:&str,p:&Value)->Result<Value>{
 if path.ends_with("/open"){return open(repo,p);}
 let id=sid(text(p,"id"))?;
 {let s=repo.state.lock().map_err(|_|Error::io())?;if !s["chatTerminals"]["byChat"].as_object().is_some_and(|m|m.values().any(|r|r["id"]==id)){return Err(Error::new("Chat terminal unavailable."));}}
 match path.rsplit('/').next().unwrap_or(""){
 "poll"=>repo.shells.poll(&id,p["cursor"].as_u64().unwrap_or(0),text(p,"generation")),
 "input"=>{let data=p["data"].as_str().ok_or(Error::new("Expected terminal input."))?;repo.shells.input(&id,data)?;Ok(json!({}))},
 "resize"=>{let cols=p["cols"].as_u64().filter(|n|(10..=500).contains(n)).ok_or(Error::new("Invalid columns."))?;let rows=p["rows"].as_u64().filter(|n|(2..=200).contains(n)).ok_or(Error::new("Invalid rows."))?;repo.shells.resize(&id,cols as u16,rows as u16)?;Ok(json!({}))},
 "close"=>{repo.shells.close(&id,text(p,"generation"))?;Ok(json!({}))},
 _=>Err(Error::new("Unknown terminal command."))}
}
#[cfg(test)]mod tests{use super::*;#[test]fn shell_identity_and_working_directory_survive_reopen(){let root=tempfile::tempdir().unwrap();let repo=Repository::open(root.path().into(),None,true).unwrap();let chat=uuid();repo.transaction(|s|{s["sessions"]=json!([{ "id":chat,"cwd":root.path(),"title":"Chat"}]);Ok(())}).unwrap();let first=open(&repo,&json!({"recordId":chat})).unwrap();repo.shells.input(text(&first,"id"),"export TRUFFLE_SHELL_TEST=retained\nprintf 'shell-check:%s\\n' \"$TRUFFLE_SHELL_TEST\"\n").unwrap();let second=open(&repo,&json!({"recordId":chat})).unwrap();assert_eq!(first["id"],second["id"]);assert_eq!(PathBuf::from(text(&first,"cwd")),root.path().canonicalize().unwrap());let mut output=vec![];let mut cursor=0;let mut generation=String::new();for _ in 0..100{std::thread::sleep(std::time::Duration::from_millis(30));let v=repo.shells.poll(text(&first,"id"),cursor,&generation).unwrap();generation=text(&v,"generation").into();cursor=v["cursor"].as_u64().unwrap();output.extend(arr(&v["bytes"]).iter().map(|n|n.as_u64().unwrap()as u8));if String::from_utf8_lossy(&output).contains("shell-check:retained"){break;}}assert!(String::from_utf8_lossy(&output).contains("shell-check:retained"));repo.shells.close(text(&first,"id"),&generation).unwrap();assert!(open(&repo,&json!({"recordId":uuid()})).is_err());}}
