//! Terminal presentation port. Local CLI utilities and the optional Codex desktop opener.
use super::*;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::Duration,
};
use wait_timeout::ChildExt;
pub fn run(program: &str, args: &[String], input: Option<&str>, seconds: u64) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_bytes(program,args,input,seconds)?).into())
}
pub fn run_bytes(program: &str, args: &[String], input: Option<&str>, seconds: u64) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            Error::new(format!(
                "Cannot start {program}. Check that it is installed."
            ))
        })?;
    if let Some(data) = input {
        child.stdin.take().unwrap().write_all(data.as_bytes())?;
    }
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut b = Vec::new();
        out.by_ref().take(16 * 1024 * 1024).read_to_end(&mut b).ok();
        b
    });
    let errors = std::thread::spawn(move || {
        let mut b = Vec::new();
        err.by_ref().take(1024 * 1024).read_to_end(&mut b).ok();
        b
    });
    let status = child.wait_timeout(Duration::from_secs(seconds))?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        let mut e = Error::new(format!(
            "{program} timed out. Cached data remains available."
        ));
        e.retry = 300.0;
        e.data = json!({"kind":"timeout","transport":"local-cli","timeoutSeconds":seconds});
        return Err(e);
    }
    let stdout = output.join().unwrap_or_default();
    let _stderr = errors.join().unwrap_or_default();
    if !status.unwrap().success() {
        return Err(Error::new(
            "Local command failed. Check account access or the selected command.",
        ));
    }
    Ok(stdout)
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn executable(name: &str) -> String {
    for root in std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .chain(["/opt/homebrew/bin", "/usr/local/bin"])
    {
        let p = PathBuf::from(root).join(name);
        if p.is_file() {
            return p.to_string_lossy().into();
        }
    }
    let p = home().join(".local/bin").join(name);
    if p.is_file() {
        return p.to_string_lossy().into();
    }
    name.into()
}
/// Observations do not launch, focus, or write to a terminal.
#[derive(Debug,PartialEq,Eq)]
pub(super) enum RuntimeObservation{Observed(String),Missing,Unavailable}
pub(super) fn existing_codex_runtime(record:&Value,inventory:&[Value],unavailable:bool)->RuntimeObservation{
    if unavailable{return RuntimeObservation::Unavailable;}
    if record["agent"]!="codex"||!record["sessionId"].is_null()&&record["sessionId"]!=""{return RuntimeObservation::Missing;}
    let matching=inventory.iter().filter(|row|row["recordId"]==record["id"]&&row["live"]==true).collect::<Vec<_>>();
    if matching.is_empty(){return RuntimeObservation::Missing;}if matching.len()!=1{return RuntimeObservation::Unavailable;}
    let row=matching[0];let generation=text(row,"generation");
    if row["host"]!="embedded"||row["agent"]!=record["agent"]||(!row["sessionId"].is_null()&&row["sessionId"]!="")||sid(generation).ok().as_deref()!=Some(generation){return RuntimeObservation::Unavailable;}
    RuntimeObservation::Observed(generation.to_owned())
}
#[derive(Debug,PartialEq,Eq)]
pub enum OpenOutcome { Accepted, Preview, Unknown }
fn desktop_command_outcome(mut command:Command,timeout:Duration)->Result<OpenOutcome>{
    let mut child=command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|Error::new("The desktop opener could not start."))?;
    match child.wait_timeout(timeout){Ok(Some(status)) if status.success()=>Ok(OpenOutcome::Accepted),_=>{let _=child.kill();let _=child.wait();Ok(OpenOutcome::Unknown)}}
}
pub fn open_desktop(s: &Value, packet: &str) -> Result<OpenOutcome> {
    let id = text(s, "sessionId");
    let agent = text(s, "agent");
    let cwd = directory(text(s, "cwd"))?;
    if s["destination"] == "desktop" && agent == "codex" {
        let url = if !id.is_empty() {
            format!("codex://threads/{}", sid(id)?)
        } else {
            let mut u = reqwest::Url::parse("codex://new").unwrap();
            u.query_pairs_mut().append_pair("path", &cwd).append_pair(
                "prompt",
                &format!("Read the work context at {packet}. Follow the assigned instruction."),
            );
            u.to_string()
        };
        let mut command=Command::new("/usr/bin/open");command.arg(url);
        return desktop_command_outcome(command,Duration::from_secs(5));
    }
    Err(Error::new("Choose the embedded terminal or Codex desktop."))
}

#[cfg(test)]
mod open_outcome_tests{
 use super::*;
 #[test]fn saved_chat_desktop_open_classifies_only_prespawn_failures_as_rejected(){assert!(desktop_command_outcome(Command::new("/nonexistent/desktop-opener"),Duration::from_millis(50)).is_err());assert_eq!(desktop_command_outcome(Command::new("/usr/bin/true"),Duration::from_secs(1)).unwrap(),OpenOutcome::Accepted);assert_eq!(desktop_command_outcome(Command::new("/usr/bin/false"),Duration::from_secs(1)).unwrap(),OpenOutcome::Unknown);let mut sleep=Command::new("/bin/sleep");sleep.arg("1");assert_eq!(desktop_command_outcome(sleep,Duration::from_millis(1)).unwrap(),OpenOutcome::Unknown);}
}

#[cfg(test)]
mod runtime_observation_tests{
 use super::*;
 #[test]
 fn null_id_runtime_observation_requires_exact_live_identity_and_canonical_generation(){let id=uuid();let generation=uuid();let record=json!({"id":id,"agent":"codex","sessionId":null});let live=json!({"host":"embedded","recordId":id,"agent":"codex","sessionId":null,"live":true,"generation":generation});assert_eq!(existing_codex_runtime(&record,&[live.clone()],false),RuntimeObservation::Observed(generation));assert_eq!(existing_codex_runtime(&record,&[live.clone()],true),RuntimeObservation::Unavailable);
 for (key,value) in [("recordId",json!(uuid())),("live",json!(false))]{let mut row=live.clone();row[key]=value;assert_eq!(existing_codex_runtime(&record,&[row],false),RuntimeObservation::Missing);}
 for (key,value) in [("agent",json!("claude")),("host",json!("other")),("sessionId",json!(uuid())),("generation",json!("stale-not-a-generation"))]{let mut row=live.clone();row[key]=value;assert_eq!(existing_codex_runtime(&record,&[row],false),RuntimeObservation::Unavailable);}
 assert_eq!(existing_codex_runtime(&record,&[live.clone(),live.clone()],false),RuntimeObservation::Unavailable);let next=uuid();let mut replaced=live;replaced["generation"]=json!(next);assert_eq!(existing_codex_runtime(&record,&[replaced],false),RuntimeObservation::Observed(next));}
}
