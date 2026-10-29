//! A same-user Unix socket separates PTY lifetime from the replaceable HTTP/UI process.
use super::*;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
const PROTOCOL: u64 = 1;
const FRAME_LIMIT: usize = 512 * 1024;

pub fn socket_directory(data: &Path) -> Result<PathBuf> {
    let parent = data
        .parent()
        .ok_or(Error::new("Invalid state path."))?
        .canonicalize()?;
    let canonical = parent.join(data.file_name().ok_or(Error::new("Invalid state path."))?);
    let hash = context_kit::sha256(canonical.to_string_lossy().as_bytes());
    let [current, legacy] = ["truffle", "mydash"]
        .map(|name| PathBuf::from(format!("/tmp/{name}-pty-{}-{}", unsafe { libc::geteuid() }, &hash[..16])));
    // A daemon started before the rename keeps its socket until it is replaced.
    if !current.join("terminal.sock").exists() && legacy.join("terminal.sock").exists() {
        return Ok(legacy);
    }
    Ok(current)
}
fn private_directory(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(Error::new(
            "Terminal daemon directory must be private and owned by this user.",
        ));
    }
    Ok(())
}
fn lockfile(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(Error::new("Terminal daemon lock must be private."));
    }
    Ok(file)
}
fn write_frame(stream: &mut UnixStream, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > FRAME_LIMIT {
        return Err(Error::new("Terminal message exceeds 512 KiB."));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
fn read_frame(stream: &mut UnixStream) -> Result<Value> {
    let mut size = [0u8; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > FRAME_LIMIT {
        return Err(Error::new("Invalid terminal message size."));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn call(dir: &Path, payload: Value) -> Result<Value> {
    let mut stream=UnixStream::connect(dir.join("terminal.sock")).map_err(|_|Error::new("Terminal daemon is unavailable. Running agents were not closed; retry after it responds."))?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write_frame(&mut stream, &json!({"protocol":PROTOCOL,"request":payload}))?;
    let reply = read_frame(&mut stream).map_err(|_| {
        Error::new("Terminal daemon did not respond. Retry; do not start a duplicate agent.")
    })?;
    if reply["protocol"] != PROTOCOL {
        return Err(Error::new("Terminal daemon protocol differs. Keep it running until its agents finish before upgrading it."));
    }
    if reply["ok"] != true {
        return Err(Error::new(text(&reply, "error")));
    }
    Ok(reply["value"].clone())
}
// Legacy open has no durable operation acknowledgement. Once any frame bytes
// may be transmitted, rejection, malformed replies and disconnects are unknown.
pub(super) fn open_request(dir:&Path,payload:Value)->Result<super::terminal::OpenOutcome>{
    let frame=serde_json::to_vec(&json!({"protocol":PROTOCOL,"request":payload}))?;
    if frame.len()>FRAME_LIMIT{return Err(Error::new("Terminal message exceeds 512 KiB."));}
    let mut stream=UnixStream::connect(dir.join("terminal.sock")).map_err(|_|Error::new("Terminal daemon is unavailable. Running agents were not closed."))?;
    stream.set_read_timeout(Some(Duration::from_secs(8)))?;stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    if stream.write_all(&(frame.len() as u32).to_be_bytes()).and_then(|_|stream.write_all(&frame)).is_err(){return Ok(super::terminal::OpenOutcome::Unknown);}
    match read_frame(&mut stream){Ok(reply) if reply["protocol"]==PROTOCOL&&reply["ok"]==true=>Ok(super::terminal::OpenOutcome::Accepted),_=>Ok(super::terminal::OpenOutcome::Unknown)}
}
pub enum Host {
    Local(super::embedded::EmbeddedHost),
    Remote(Remote),
}
pub struct Remote {
    root: PathBuf,
    data: PathBuf,
    dir: PathBuf,
    opening: Mutex<()>,
    warning: Mutex<Option<String>>,
}
impl Host {
    pub fn new(root: &Path, data: &Path, demo: bool) -> Result<Self> {
        if demo {
            return Ok(Self::Local(super::embedded::EmbeddedHost::default()));
        }
        let dir = socket_directory(data)?;
        private_directory(&dir)?;
        Ok(Self::Remote(Remote {
            root: root.canonicalize()?,
            data: data.to_owned(),
            dir,
            opening: Mutex::new(()),
            warning: Mutex::new(None),
        }))
    }
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Local(_) => None,
            Self::Remote(r) => r.warning.lock().unwrap().clone(),
        }
    }
    /// Per-call inventory outcome; does not start or repair a daemon.
    pub(super) fn inventory_checked(&self)->Result<Vec<Value>>{
        match self{
            Self::Local(host)=>Ok(host.inventory()),
            Self::Remote(remote)=>remote.request(json!({"op":"inventory"}))?.as_array().cloned().ok_or(Error::new("Terminal inventory response is invalid.")),
        }
    }
    pub fn inventory(&self) -> Vec<Value> {
        if let Self::Remote(remote)=self{if !remote.dir.join("terminal.sock").exists(){return vec![];}}
        match self.inventory_checked(){
            Ok(rows)=>{if let Self::Remote(remote)=self{*remote.warning.lock().unwrap()=None;}rows},
            Err(error)=>{if let Self::Remote(remote)=self{*remote.warning.lock().unwrap()=Some(error.message);}vec![]},
        }
    }
    pub fn open(&self, record: &Value, packet: &str, new: bool, root: &Path) -> Result<super::terminal::OpenOutcome> {
        match self {
            Self::Local(h) => h.open(record, packet, new, root).map(|_|super::terminal::OpenOutcome::Accepted),
            Self::Remote(r) => {
                r.ensure()?;
                let mut launch = json!({});
                for key in ["id", "sessionId", "agent", "title", "cwd"] {
                    launch[key] = record[key].clone();
                }
                open_request(&r.dir,json!({"op":"open","record":launch,"packet":packet,"new":new}))
            }
        }
    }
    pub fn open_shell(&self,record:&Value)->Result<Value>{match self{Self::Local(host)=>host.open_shell(record),Self::Remote(remote)=>{remote.ensure()?;remote.request(json!({"op":"shell","record":record}))}}}
    pub fn open_demo(&self, record: &Value) -> Result<()> {
        match self {
            Self::Local(h) => h.open_demo(record),
            Self::Remote(_) => Err(Error::new("Demo shells require an isolated demo service.")),
        }
    }
    pub fn poll(&self, id: &str, cursor: u64, generation: &str) -> Result<Value> {
        match self {
            Self::Local(h) => h.poll(id, cursor, generation),
            Self::Remote(r) => {
                r.request(json!({"op":"poll","id":id,"cursor":cursor,"generation":generation}))
            }
        }
    }
    pub fn input(&self, id: &str, data: &str) -> Result<()> {
        match self {
            Self::Local(h) => h.input(id, data),
            Self::Remote(r) => r
                .request(json!({"op":"input","id":id,"data":data}))
                .map(|_| ()),
        }
    }
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        match self {
            Self::Local(h) => h.resize(id, cols, rows),
            Self::Remote(r) => r
                .request(json!({"op":"resize","id":id,"cols":cols,"rows":rows}))
                .map(|_| ()),
        }
    }
    pub fn close(&self, id: &str, generation: &str) -> Result<()> {
        match self {
            Self::Local(h) => h.close(id, generation),
            Self::Remote(r) => r
                .request(json!({"op":"close","id":id,"generation":generation}))
                .map(|_| ()),
        }
    }
}
impl Remote {
    fn request(&self, payload: Value) -> Result<Value> {
        call(&self.dir, payload)
    }
    fn ensure(&self) -> Result<()> {
        let _guard = self.opening.lock().unwrap();
        let launch = lockfile(&self.dir.join("launch.lock"))?;
        launch.lock_exclusive()?;
        if self.dir.join("terminal.sock").exists() {
            match self.request(json!({"op":"health"})) {
                Ok(v) => return self.validate(&v),
                Err(e) => {
                    let owner = lockfile(&self.dir.join("daemon.lock"))?;
                    if owner.try_lock_exclusive().is_err() {
                        return Err(e);
                    }
                    // A crashed daemon left a stale socket. Only the new owner may replace it.
                }
            }
        }
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--terminal-daemon")
            .arg("--home")
            .arg(&self.root)
            .arg("--data")
            .arg(&self.data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Detach from service terminal/process-group signals. PTYs belong only to this child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if self.dir.join("terminal.sock").exists() {
                if let Ok(value) = self.request(json!({"op":"health"})) {
                    let result = self.validate(&value);
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return result;
                }
            }
            if child.try_wait()?.is_some() {
                return Err(Error::new("Terminal daemon could not start. Check local permissions; existing terminals were not replaced."));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Err(Error::new(
            "Terminal daemon is still starting. Retry opening this saved session.",
        ))
    }
    fn validate(&self, health: &Value) -> Result<()> {
        if health["root"] != json!(self.root) {
            return Err(Error::new("The terminal daemon belongs to a different truffle installation. Keep it running until its agents finish."));
        }
        Ok(())
    }
}
fn dispatch(
    host: &super::embedded::EmbeddedHost,
    root: &Path,
    data: &Path,
    demo: bool,
    p: &Value,
) -> Result<Value> {
    let id = text(p, "id");
    match text(p, "op") {
        "health" => {
            Ok(json!({"pid":std::process::id(),"root":root,"version":env!("CARGO_PKG_VERSION")}))
        }
        "inventory" => Ok(json!(host.inventory())),
        "open" => {
            sid(text(&p["record"], "id"))?;
            let packet = PathBuf::from(text(p, "packet")).canonicalize()?;
            let contexts = data.parent().unwrap().join("contexts").canonicalize()?;
            if packet.parent() != Some(contexts.as_path())
                || packet.extension().is_none_or(|x| x != "md")
                || packet
                    .file_stem()
                    .is_none_or(|s| s.to_str() != Some(text(&p["record"], "id")))
            {
                return Err(Error::new("Invalid private terminal context path."));
            }
            host.open(
                &p["record"],
                &packet.to_string_lossy(),
                p["new"].as_bool().unwrap_or(false),
                root,
            )
        }
        "shell" => {sid(text(&p["record"],"id"))?;host.open_shell(&p["record"])},
        "demo" if demo => {
            sid(text(&p["record"], "id"))?;
            host.open_demo(&p["record"])?;
            Ok(json!({}))
        }
        "poll" => {
            sid(id)?;
            host.poll(id, p["cursor"].as_u64().unwrap_or(0), text(p, "generation"))
        }
        "input" => {
            sid(id)?;
            host.input(id, text(p, "data"))?;
            Ok(json!({}))
        }
        "resize" => {
            sid(id)?;
            let cols = p["cols"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or(Error::new("Invalid terminal columns."))?;
            let rows = p["rows"]
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or(Error::new("Invalid terminal rows."))?;
            host.resize(id, cols, rows)?;
            Ok(json!({}))
        }
        "close" => {
            sid(id)?;
            host.close(id, text(p, "generation"))?;
            Ok(json!({}))
        }
        _ => Err(Error::new("Unsupported terminal operation.")),
    }
}
pub fn serve(root: PathBuf, data: PathBuf, allow_demo: bool) -> Result<()> {
    let root = root.canonicalize()?;
    let dir = socket_directory(&data)?;
    private_directory(&dir)?;
    let lock = lockfile(&dir.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| Error::new("A terminal daemon already owns this data path."))?;
    let socket = dir.join("terminal.sock");
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let host = Arc::new(super::embedded::EmbeddedHost::default());
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        if active.fetch_add(1, Ordering::Relaxed) >= 64 {
            active.fetch_sub(1, Ordering::Relaxed);
            continue;
        }
        let host = host.clone();
        let active = active.clone();
        let root = root.clone();
        let data = data.clone();
        std::thread::spawn(move || {
            struct Slot(Arc<AtomicUsize>);
            impl Drop for Slot {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::Relaxed);
                }
            }
            let _slot = Slot(active);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
            let result = (|| -> Result<Value> {
                let request = read_frame(&mut stream)?;
                if request["protocol"] != PROTOCOL {
                    return Err(Error::new("Unsupported terminal protocol."));
                }
                dispatch(&host, &root, &data, allow_demo, &request["request"])
            })();
            let reply = match result {
                Ok(v) => json!({"protocol":PROTOCOL,"ok":true,"value":v}),
                Err(e) => json!({"protocol":PROTOCOL,"ok":false,"error":e.message}),
            };
            let _ = write_frame(&mut stream, &reply);
        });
    }
    drop(lock);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_directory_identity_uses_canonical_parent_and_rejects_public_or_symlink_directories() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().canonicalize().unwrap();
        assert_eq!(
            socket_directory(&root.path().join("state.json")).unwrap(),
            socket_directory(&parent.join("state.json")).unwrap()
        );
        let private = root.path().join("private");
        private_directory(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_directory(&private).is_err());
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert!(private_directory(&link).is_err());
    }
    #[test]
    fn socket_directory_keeps_a_live_daemon_from_before_the_rename() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("state.json");
        let current = socket_directory(&data).unwrap();
        let legacy = PathBuf::from(current.to_string_lossy().replace("/tmp/truffle-pty-", "/tmp/mydash-pty-"));
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("terminal.sock"), "").unwrap();
        assert_eq!(socket_directory(&data).unwrap(), legacy);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("terminal.sock"), "").unwrap();
        assert_eq!(socket_directory(&data).unwrap(), current);
        std::fs::remove_dir_all(&legacy).unwrap();
        std::fs::remove_dir_all(&current).unwrap();
    }
    #[test]
    fn framing_and_dispatch_bound_untrusted_messages() {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer
            .write_all(&((FRAME_LIMIT + 1) as u32).to_be_bytes())
            .unwrap();
        assert!(read_frame(&mut reader).is_err());
        let root = tempfile::tempdir().unwrap();
        let host = super::super::embedded::EmbeddedHost::default();
        assert!(dispatch(
            &host,
            root.path(),
            &root.path().join("state.json"),
            false,
            &json!({"op":"demo","record":{"id":uuid()}})
        )
        .is_err());
        assert!(dispatch(
            &host,
            root.path(),
            &root.path().join("state.json"),
            false,
            &json!({"op":"resize","id":uuid(),"cols":65537,"rows":30})
        )
        .is_err());
        assert!(dispatch(
            &host,
            root.path(),
            &root.path().join("state.json"),
            false,
            &json!({"op":"run","command":"anything"})
        )
        .is_err());
    }
}

#[cfg(test)]
mod saved_chat_open_transport_tests {
 use super::*;
 #[test]
 fn saved_chat_open_transport_classifies_all_postsend_reply_failures_as_unknown(){
  for response in [None,Some(json!({"protocol":PROTOCOL,"ok":false,"error":"private daemon detail"})),Some(json!({"protocol":999,"ok":true})),Some(json!({"protocol":PROTOCOL,"ok":true,"value":{}}))]{
   let temp=tempfile::tempdir().unwrap();let listener=UnixListener::bind(temp.path().join("terminal.sock")).unwrap();let accepted=response.as_ref().is_some_and(|r|r["protocol"]==PROTOCOL&&r["ok"]==true);
   let worker=std::thread::spawn(move||{let(mut stream,_)=listener.accept().unwrap();let request=read_frame(&mut stream).unwrap();assert_eq!(request["request"]["op"],"open");if let Some(response)=response{write_frame(&mut stream,&response).unwrap();}});
   let outcome=open_request(temp.path(),json!({"op":"open"})).unwrap();assert_eq!(outcome,if accepted{super::super::terminal::OpenOutcome::Accepted}else{super::super::terminal::OpenOutcome::Unknown});worker.join().unwrap();
  }
  let temp=tempfile::tempdir().unwrap();assert!(open_request(temp.path(),json!({"op":"open"})).is_err());assert!(open_request(temp.path(),json!({"body":"x".repeat(FRAME_LIMIT)})).is_err());
 }
}

#[cfg(test)]
mod checked_inventory_tests{
 use super::*;
 #[test]
 fn runtime_inventory_failure_is_per_call_even_when_shared_warning_is_clear(){let temp=tempfile::tempdir().unwrap();let remote=Remote{root:temp.path().into(),data:temp.path().join("state.json"),dir:temp.path().into(),opening:Mutex::new(()),warning:Mutex::new(None)};let host=Host::Remote(remote);let failure=host.inventory_checked();assert!(failure.is_err());assert!(host.warning().is_none());assert!(host.inventory().is_empty());assert!(failure.is_err());assert!(!temp.path().join("terminal.sock").exists());}
 #[test]
 fn runtime_inventory_checked_rejects_invalid_reply_and_preserves_exact_generation(){for value in [json!({"bad":"shape"}),json!([{"host":"embedded","recordId":uuid(),"agent":"codex","sessionId":null,"generation":uuid(),"live":true}])]{let temp=tempfile::tempdir().unwrap();let listener=UnixListener::bind(temp.path().join("terminal.sock")).unwrap();let expected=value.clone();let server=std::thread::spawn(move||{let (mut stream,_)=listener.accept().unwrap();let request=read_frame(&mut stream).unwrap();assert_eq!(request["request"]["op"],"inventory");write_frame(&mut stream,&json!({"protocol":PROTOCOL,"ok":true,"value":value})).unwrap();});let host=Host::Remote(Remote{root:temp.path().into(),data:temp.path().join("state.json"),dir:temp.path().into(),opening:Mutex::new(()),warning:Mutex::new(None)});let result=host.inventory_checked();server.join().unwrap();if expected.is_array(){assert_eq!(json!(result.unwrap()),expected);}else{assert!(result.is_err());}}}
}
