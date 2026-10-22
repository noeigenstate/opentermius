//! SFTP operations exercised through a real, in-process SSH/SFTP server.
use opentermius::{
    ssh::{self, Command, Event},
    store::Profile,
};
use russh::{
    Channel, ChannelId,
    server::{self, Auth, Server as _},
};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

#[derive(Clone, Debug)]
enum Node {
    Dir,
    File(Vec<u8>),
}
impl Node {
    fn attrs(&self) -> FileAttributes {
        FileAttributes {
            size: Some(match self {
                Self::Dir => 0,
                Self::File(data) => data.len() as u64,
            }),
            permissions: Some(match self {
                Self::Dir => 0o40755,
                Self::File(_) => 0o100644,
            }),
            mtime: Some(1700000000),
            ..Default::default()
        }
    }
}
#[derive(Clone)]
struct State {
    nodes: Arc<Mutex<HashMap<String, Node>>>,
    slow: Arc<AtomicBool>,
    bad_name: Arc<Mutex<Option<String>>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            nodes: Arc::new(Mutex::new(HashMap::from([("/".into(), Node::Dir)]))),
            slow: Arc::new(AtomicBool::new(false)),
            bad_name: Arc::new(Mutex::new(None)),
        }
    }
}
#[derive(Clone)]
struct Server(State);
impl server::Server for Server {
    type Handler = Peer;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Peer {
        Peer {
            state: self.0.clone(),
            channels: HashMap::new(),
        }
    }
}
struct Peer {
    state: State,
    channels: HashMap<ChannelId, Channel<server::Msg>>,
}
impl server::Handler for Peer {
    type Error = anyhow::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "tester" && password == "test-password" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)?;
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.channels.contains_key(&id) {
            session.data(id, data.to_vec())?;
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            session.channel_success(id)?;
            let channel = self.channels.remove(&id).unwrap();
            let state = self.state.clone();
            tokio::spawn(async move {
                russh_sftp::server::run(
                    channel.into_stream(),
                    FileServer {
                        state,
                        listed: HashSet::new(),
                    },
                )
                .await;
            });
        } else {
            session.channel_failure(id)?;
        }
        Ok(())
    }
}
struct FileServer {
    state: State,
    listed: HashSet<String>,
}
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}
fn normalized(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap()
}
impl russh_sftp::server::Handler for FileServer {
    type Error = StatusCode;
    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let path = normalized(&path);
        let nodes = self.state.nodes.lock().unwrap();
        Ok(Name {
            id,
            files: vec![File::new(
                path.clone(),
                nodes.get(&path).ok_or(StatusCode::NoSuchFile)?.attrs(),
            )],
        })
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        if !matches!(self.state.nodes.lock().unwrap().get(&path), Some(Node::Dir)) {
            return Err(StatusCode::NoSuchFile);
        }
        self.listed.remove(&path);
        Ok(Handle { id, handle: path })
    }
    async fn readdir(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        if !self.listed.insert(path.clone()) {
            return Err(StatusCode::Eof);
        }
        let nodes = self.state.nodes.lock().unwrap();
        let mut files = nodes
            .iter()
            .filter(|(name, _)| name.as_str() != "/" && parent(name) == path)
            .map(|(name, node)| File::new(name.rsplit('/').next().unwrap(), node.attrs()))
            .collect::<Vec<_>>();
        files.push(File::new(".", Node::Dir.attrs()));
        files.push(File::new("..", Node::Dir.attrs()));
        if let Some(name) = self.state.bad_name.lock().unwrap().as_ref() {
            files.push(File::new(name, Node::File(vec![]).attrs()));
        }
        Ok(Name { id, files })
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        Ok(Attrs {
            id,
            attrs: self
                .state
                .nodes
                .lock()
                .unwrap()
                .get(&path)
                .ok_or(StatusCode::NoSuchFile)?
                .attrs(),
        })
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.stat(id, path).await
    }
    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _: FileAttributes,
    ) -> Result<Status, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        if nodes.contains_key(&path) || !matches!(nodes.get(parent(&path)), Some(Node::Dir)) {
            return Err(StatusCode::Failure);
        }
        nodes.insert(path, Node::Dir);
        Ok(ok(id))
    }
    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        if nodes.contains_key(&to) || !nodes.contains_key(&from) {
            return Err(StatusCode::Failure);
        }
        let paths = nodes
            .keys()
            .filter(|path| *path == &from || path.starts_with(&format!("{from}/")))
            .cloned()
            .collect::<Vec<_>>();
        for path in paths {
            let node = nodes.remove(&path).unwrap();
            nodes.insert(format!("{to}{}", &path[from.len()..]), node);
        }
        Ok(ok(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        if !matches!(nodes.get(&path), Some(Node::File(_))) {
            return Err(StatusCode::Failure);
        }
        nodes.remove(&path);
        Ok(ok(id))
    }
    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        if !matches!(nodes.get(&path), Some(Node::Dir))
            || nodes
                .keys()
                .any(|name| name.starts_with(&format!("{path}/")))
        {
            return Err(StatusCode::Failure);
        }
        nodes.remove(&path);
        Ok(ok(id))
    }
    async fn open(
        &mut self,
        id: u32,
        path: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        if flags.contains(OpenFlags::EXCLUDE) && nodes.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        if flags.contains(OpenFlags::CREATE) {
            if !matches!(nodes.get(parent(&path)), Some(Node::Dir)) {
                return Err(StatusCode::NoSuchFile);
            }
            nodes.entry(path.clone()).or_insert(Node::File(vec![]));
        }
        let Some(Node::File(data)) = nodes.get_mut(&path) else {
            return Err(StatusCode::NoSuchFile);
        };
        if flags.contains(OpenFlags::TRUNCATE) {
            data.clear();
        }
        Ok(Handle { id, handle: path })
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }
    async fn read(
        &mut self,
        id: u32,
        path: String,
        offset: u64,
        length: u32,
    ) -> Result<Data, StatusCode> {
        if self.state.slow.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let nodes = self.state.nodes.lock().unwrap();
        let Some(Node::File(data)) = nodes.get(&path) else {
            return Err(StatusCode::NoSuchFile);
        };
        let offset = offset as usize;
        if offset >= data.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data {
            id,
            data: data[offset..(offset + length as usize).min(data.len())].to_vec(),
        })
    }
    async fn write(
        &mut self,
        id: u32,
        path: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let mut nodes = self.state.nodes.lock().unwrap();
        let Some(Node::File(data)) = nodes.get_mut(&path) else {
            return Err(StatusCode::NoSuchFile);
        };
        let offset = offset as usize;
        data.resize(data.len().max(offset + bytes.len()), 0);
        data[offset..offset + bytes.len()].copy_from_slice(&bytes);
        Ok(ok(id))
    }
}
async fn fixture() -> (u16, String, State, tokio::task::JoinHandle<()>) {
    let key = russh::keys::PrivateKey::random(
        &mut russh::keys::key::safe_rng(),
        russh::keys::Algorithm::Ed25519,
    )
    .unwrap();
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let state = State::default();
    let mut server = Server(state.clone());
    let task = tokio::spawn(async move {
        server.run_on_socket(config, &socket).await.unwrap();
    });
    (port, fingerprint, state, task)
}
fn profile(port: u16) -> Profile {
    Profile {
        id: "test".into(),
        name: "loopback".into(),
        host: "127.0.0.1".into(),
        port,
        username: "tester".into(),
        auth_type: "password".into(),
        private_key_path: String::new(),
        key_id: String::new(),
        group: String::new(),
        encrypted_secret: None,
        tags: Vec::new(),
        last_connected: None,
        created_at: None,
    }
}
async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("transport timed out")
        .expect("event queue closed")
}
async fn connected(rx: &mut mpsc::Receiver<Event>) {
    loop {
        match next(rx).await {
            Event::Status {
                connected: true, ..
            } => break,
            Event::Status {
                error: true,
                message,
                ..
            } => panic!("{message}"),
            _ => {}
        }
    }
}

struct Fixture {
    session: ssh::Session,
    rx: mpsc::Receiver<Event>,
    state: State,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let (port, key, state, server) = fixture().await;
        let (tx, mut rx) = mpsc::channel(128);
        let session = ssh::spawn(
            &tokio::runtime::Handle::current(),
            "test".into(),
            profile(port),
            "test-password".into(),
            Some(key),
            tx,
        );
        connected(&mut rx).await;
        Self {
            session,
            rx,
            state,
            server,
        }
    }
    async fn send(&self, command: Command) {
        self.session.commands.send(command).await.unwrap();
    }
    async fn message(&mut self, error: bool) {
        loop {
            match next(&mut self.rx).await {
                Event::Transfer {
                    error: actual,
                    message,
                    ..
                } => {
                    assert_eq!(actual, error, "{message}");
                    return;
                }
                _ => {}
            }
        }
    }
    async fn finished(&mut self, transfer: u64, error: bool, cancelled: bool) -> String {
        loop {
            match next(&mut self.rx).await {
                Event::TransferFinished {
                    transfer: actual,
                    error: actual_error,
                    cancelled: actual_cancelled,
                    message,
                    ..
                } if transfer == actual => {
                    assert_eq!(
                        (actual_error, actual_cancelled),
                        (error, cancelled),
                        "{message}"
                    );
                    return message;
                }
                Event::Transfer {
                    error: true,
                    message,
                    ..
                } => panic!("unexpected untracked error: {message}"),
                _ => {}
            }
        }
    }
    async fn close(self) {
        self.send(Command::Disconnect).await;
        timeout(Duration::from_secs(5), self.session.task)
            .await
            .unwrap()
            .unwrap();
        self.server.abort();
    }
}

struct DirectoryLink(std::path::PathBuf);
impl DirectoryLink {
    fn new(link: std::path::PathBuf, target: &std::path::Path) -> Self {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &link).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(target)
                .output()
                .unwrap();
            assert!(result.status.success(), "failed to create test junction");
        }
        Self(link)
    }
}
impl Drop for DirectoryLink {
    fn drop(&mut self) {
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&self.0);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chosen_root_through_linked_ancestor_supports_all_transfer_entries() {
    let mut f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("chosen/tree/nested/empty")).unwrap();
    std::fs::write(real.join("chosen/file.txt"), b"single").unwrap();
    std::fs::write(real.join("chosen/tree/nested/data.bin"), b"recursive").unwrap();
    let alias = DirectoryLink::new(dir.path().join("alias"), &real);
    let root = alias.0.join("chosen");
    assert_eq!(
        ssh::canonical_local_root(&root).unwrap(),
        real.join("chosen").canonicalize().unwrap()
    );
    assert_eq!(ssh::read_local_dir(&root).unwrap().len(), 2);
    for legacy in [true, false] {
        let upload = if legacy {
            Command::Upload {
                local_root: root.clone(),
                directory: "/".into(),
                local: root.join("file.txt"),
                overwrite: true,
            }
        } else {
            Command::UploadFile {
                local_root: root.clone(),
                transfer: 1,
                directory: "/".into(),
                local: root.join("file.txt"),
                overwrite: true,
            }
        };
        f.send(upload).await;
        if legacy {
            f.message(false).await;
        } else {
            f.finished(1, false, false).await;
        }
        let name = if legacy { "legacy.txt" } else { "saved.txt" };
        let download = if legacy {
            Command::Download {
                local_root: root.clone(),
                remote: "/file.txt".into(),
                local: root.join(name),
            }
        } else {
            Command::DownloadFile {
                local_root: root.clone(),
                transfer: 2,
                remote: "/file.txt".into(),
                local: root.join(name),
            }
        };
        f.send(download).await;
        if legacy {
            f.message(false).await;
        } else {
            f.finished(2, false, false).await;
        }
        assert_eq!(
            std::fs::read(real.join("chosen").join(name)).unwrap(),
            b"single"
        );
    }
    f.send(Command::UploadTree {
        local_root: root.clone(),
        transfer: 3,
        directory: "/".into(),
        local: root.join("tree"),
        overwrite: false,
    })
    .await;
    f.finished(3, false, false).await;
    std::fs::create_dir(root.join("downloads")).unwrap();
    f.send(Command::DownloadTree {
        local_root: root.clone(),
        transfer: 4,
        remote: "/tree".into(),
        local_dir: root.join("downloads"),
    })
    .await;
    f.finished(4, false, false).await;
    assert_eq!(
        std::fs::read(root.join("downloads/tree/nested/data.bin")).unwrap(),
        b"recursive"
    );
    assert!(root.join("downloads/tree/nested/empty").is_dir());
    // A directly chosen alias is also a valid boundary.
    f.send(Command::DownloadFile {
        local_root: alias.0.clone(),
        transfer: 5,
        remote: "/file.txt".into(),
        local: alias.0.join("direct.txt"),
    })
    .await;
    f.finished(5, false, false).await;
    assert_eq!(std::fs::read(real.join("direct.txt")).unwrap(), b"single");
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn links_below_chosen_root_remain_blocked_for_uploads_and_downloads() {
    let mut f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"outside").unwrap();
    let link = DirectoryLink::new(dir.path().join("link"), outside.path());
    for (index, command) in [
        Command::UploadFile {
            local_root: dir.path().into(),
            transfer: 1,
            directory: "/".into(),
            local: link.0.join("secret.txt"),
            overwrite: true,
        },
        Command::UploadTree {
            local_root: dir.path().into(),
            transfer: 2,
            directory: "/".into(),
            local: link.0.clone(),
            overwrite: true,
        },
    ]
    .into_iter()
    .enumerate()
    {
        f.send(command).await;
        assert!(
            f.finished(index as u64 + 1, true, false)
                .await
                .contains("links cannot be followed")
        );
    }
    assert_eq!(
        f.state.nodes.lock().unwrap().len(),
        1,
        "rejected uploads created remote entries"
    );
    {
        let mut nodes = f.state.nodes.lock().unwrap();
        nodes.insert("/tree".into(), Node::Dir);
        nodes.insert("/tree/file.txt".into(), Node::File(b"download".to_vec()));
    }
    f.send(Command::DownloadFile {
        local_root: dir.path().into(),
        transfer: 3,
        remote: "/tree/file.txt".into(),
        local: link.0.join("saved.txt"),
    })
    .await;
    assert!(f.finished(3, true, false).await.contains("links cannot be followed"));
    let tree_link = DirectoryLink::new(dir.path().join("tree"), outside.path());
    f.send(Command::DownloadTree {
        local_root: dir.path().into(),
        transfer: 4,
        remote: "/tree".into(),
        local_dir: dir.path().into(),
    })
    .await;
    assert!(f.finished(4, true, false).await.contains("links cannot be followed"));
    assert!(!tree_link.0.join("file.txt").exists());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside"
    );
    std::fs::write(dir.path().join("safe.txt"), b"inside").unwrap();
    f.send(Command::UploadTree {
        local_root: dir.path().into(),
        transfer: 5,
        directory: "/".into(),
        local: dir.path().into(),
        overwrite: false,
    })
    .await;
    f.finished(5, false, false).await;
    {
        let nodes = f.state.nodes.lock().unwrap();
        let prefix = format!("/{}", dir.path().file_name().unwrap().to_str().unwrap());
        assert!(nodes.contains_key(&format!("{prefix}/safe.txt")));
        assert!(
            !nodes
                .keys()
                .any(|path| path.starts_with(&format!("{prefix}/link"))
                    || path.starts_with(&format!("{prefix}/tree")))
        );
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mkdir_rename_remove_and_confirmed_recursive_rmdir() {
    let mut f = Fixture::new().await;
    f.send(Command::CreateDir {
        directory: "/".into(),
        name: "folder".into(),
    })
    .await;
    f.message(false).await;
    assert!(matches!(
        f.state.nodes.lock().unwrap().get("/folder"),
        Some(Node::Dir)
    ));
    f.send(Command::Rename {
        directory: "/".into(),
        from: "folder".into(),
        to: "renamed".into(),
    })
    .await;
    f.message(false).await;
    {
        let mut nodes = f.state.nodes.lock().unwrap();
        nodes.insert("/renamed/nested".into(), Node::Dir);
        nodes.insert(
            "/renamed/nested/file.txt".into(),
            Node::File(b"hello".to_vec()),
        );
    }
    f.send(Command::Delete {
        directory: "/".into(),
        name: "renamed".into(),
        confirmed: false,
    })
    .await;
    loop {
        if let Event::DeleteRequired {
            files, dirs, bytes, ..
        } = next(&mut f.rx).await
        {
            assert_eq!((files, dirs, bytes), (1, 1, 5));
            break;
        }
    }
    assert!(
        f.state
            .nodes
            .lock()
            .unwrap()
            .contains_key("/renamed/nested/file.txt"),
        "summary must not delete"
    );
    f.send(Command::Delete {
        directory: "/".into(),
        name: "renamed".into(),
        confirmed: true,
    })
    .await;
    f.message(false).await;
    assert_eq!(f.state.nodes.lock().unwrap().len(), 1);
    f.state
        .nodes
        .lock()
        .unwrap()
        .insert("/before.txt".into(), Node::File(b"one".to_vec()));
    f.send(Command::Rename {
        directory: "/".into(),
        from: "before.txt".into(),
        to: "after.txt".into(),
    })
    .await;
    f.message(false).await;
    assert!(f.state.nodes.lock().unwrap().contains_key("/after.txt"));
    f.send(Command::Delete {
        directory: "/".into(),
        name: "after.txt".into(),
        confirmed: true,
    })
    .await;
    f.message(false).await;
    f.send(Command::CreateDir {
        directory: "/".into(),
        name: "../escape".into(),
    })
    .await;
    f.message(true).await;
    assert_eq!(f.state.nodes.lock().unwrap().len(), 1);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recursive_round_trip_overwrite_prompt_and_download_no_clobber() {
    let mut f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tree");
    std::fs::create_dir_all(source.join("nested/empty")).unwrap();
    let data = (0..200000).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    std::fs::write(source.join("nested/data.bin"), &data).unwrap();
    std::fs::write(source.join(".hidden"), b"hidden").unwrap();
    f.send(Command::UploadTree {
        local_root: dir.path().to_path_buf(),
        transfer: 1,
        directory: "/".into(),
        local: source.clone(),
        overwrite: false,
    })
    .await;
    let mut progress = false;
    loop {
        match next(&mut f.rx).await {
            Event::TransferProgress {
                transfer: 1,
                transferred,
                total,
                ..
            } => {
                assert!(transferred <= total);
                if transferred == total {
                    assert_eq!(total, data.len() as u64 + 6);
                    progress = true;
                }
            }
            Event::TransferFinished {
                transfer: 1,
                error,
                message,
                ..
            } => {
                assert!(!error, "{message}");
                break;
            }
            _ => {}
        }
    }
    assert!(progress, "must report final byte progress");
    assert!(
        f.state
            .nodes
            .lock()
            .unwrap()
            .contains_key("/tree/nested/empty")
    );
    f.send(Command::UploadTree {
        local_root: dir.path().to_path_buf(),
        transfer: 2,
        directory: "/".into(),
        local: source.clone(),
        overwrite: false,
    })
    .await;
    loop {
        if let Event::OverwriteRequired {
            transfer, folder, ..
        } = next(&mut f.rx).await
        {
            assert_eq!(transfer, 2);
            assert!(folder);
            break;
        }
    }
    std::fs::write(source.join(".hidden"), b"changed").unwrap();
    f.send(Command::UploadTree {
        local_root: dir.path().to_path_buf(),
        transfer: 2,
        directory: "/".into(),
        local: source,
        overwrite: true,
    })
    .await;
    f.finished(2, false, false).await;
    let download = dir.path().join("downloads");
    std::fs::create_dir(&download).unwrap();
    f.send(Command::DownloadTree {
        local_root: dir.path().to_path_buf(),
        transfer: 3,
        remote: "/tree".into(),
        local_dir: download.clone(),
    })
    .await;
    f.finished(3, false, false).await;
    assert_eq!(
        std::fs::read(download.join("tree/nested/data.bin")).unwrap(),
        data
    );
    assert_eq!(
        std::fs::read(download.join("tree/.hidden")).unwrap(),
        b"changed"
    );
    assert!(download.join("tree/nested/empty").is_dir());
    std::fs::write(download.join("tree/.hidden"), b"preserve").unwrap();
    f.send(Command::DownloadTree {
        local_root: dir.path().to_path_buf(),
        transfer: 4,
        remote: "/tree".into(),
        local_dir: download.clone(),
    })
    .await;
    f.finished(4, true, false).await;
    assert_eq!(
        std::fs::read(download.join("tree/.hidden")).unwrap(),
        b"preserve"
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_file_transfers_ask_before_overwrite_and_download_never_replaces() {
    let mut f = Fixture::new().await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("upload.bin");
    std::fs::write(&source, b"first version").unwrap();
    f.send(Command::UploadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 1,
        directory: "/".into(),
        local: source.clone(),
        overwrite: false,
    })
    .await;
    f.finished(1, false, false).await;
    std::fs::write(&source, b"second version").unwrap();
    f.send(Command::UploadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 2,
        directory: "/".into(),
        local: source.clone(),
        overwrite: false,
    })
    .await;
    loop {
        if let Event::OverwriteRequired {
            transfer, folder, ..
        } = next(&mut f.rx).await
        {
            assert_eq!(transfer, 2);
            assert!(!folder);
            break;
        }
    }
    {
        let nodes = f.state.nodes.lock().unwrap();
        let Some(Node::File(data)) = nodes.get("/upload.bin") else {
            panic!("missing upload");
        };
        assert_eq!(data, b"first version");
    }
    f.send(Command::UploadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 2,
        directory: "/".into(),
        local: source,
        overwrite: true,
    })
    .await;
    f.finished(2, false, false).await;
    let destination = dir.path().join("saved.bin");
    f.send(Command::DownloadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 3,
        remote: "/upload.bin".into(),
        local: destination.clone(),
    })
    .await;
    f.finished(3, false, false).await;
    assert_eq!(std::fs::read(&destination).unwrap(), b"second version");
    std::fs::write(&destination, b"keep local").unwrap();
    f.send(Command::DownloadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 4,
        remote: "/upload.bin".into(),
        local: destination.clone(),
    })
    .await;
    f.finished(4, true, false).await;
    assert_eq!(std::fs::read(destination).unwrap(), b"keep local");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_active_download_removes_partial_and_cancel_queued_upload_has_no_effect() {
    let mut f = Fixture::new().await;
    f.state.slow.store(true, Ordering::SeqCst);
    f.state
        .nodes
        .lock()
        .unwrap()
        .insert("/large.bin".into(), Node::File(vec![7; 8 * 1024 * 1024]));
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("download.bin");
    let upload = dir.path().join("queued.txt");
    std::fs::write(&upload, b"never upload").unwrap();
    f.send(Command::DownloadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 1,
        remote: "/large.bin".into(),
        local: local.clone(),
    })
    .await;
    f.send(Command::UploadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 2,
        directory: "/".into(),
        local: upload,
        overwrite: false,
    })
    .await;
    f.send(Command::CancelTransfer { transfer: 2 }).await;
    loop {
        if let Event::TransferProgress {
            transfer: 1,
            transferred,
            total,
            ..
        } = next(&mut f.rx).await
        {
            assert!(transferred > 0 && transferred < total);
            break;
        }
    }
    assert!(
        std::fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part"))
    );
    f.send(Command::CancelTransfer { transfer: 1 }).await;
    f.finished(1, false, true).await;
    f.finished(2, false, true).await;
    assert!(!local.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "only original queued.txt remains"
    );
    assert!(!f.state.nodes.lock().unwrap().contains_key("/queued.txt"));
    f.send(Command::List("/".into())).await;
    loop {
        if let Event::Files { entries, .. } = next(&mut f.rx).await {
            assert_eq!(entries.len(), 1);
            break;
        }
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsafe_server_names_never_create_local_files() {
    let mut f = Fixture::new().await;
    f.state
        .nodes
        .lock()
        .unwrap()
        .insert("/unsafe".into(), Node::Dir);
    let dir = tempfile::tempdir().unwrap();
    for (index, name) in [
        "../escape",
        "/absolute",
        "sub/file",
        r"sub\file",
        "C:stream",
        "trailing.",
        "NUL.txt",
    ]
    .into_iter()
    .enumerate()
    {
        *f.state.bad_name.lock().unwrap() = Some(name.to_string());
        f.send(Command::DownloadTree {
            local_root: dir.path().to_path_buf(),
            transfer: index as u64 + 1,
            remote: "/unsafe".into(),
            local_dir: dir.path().into(),
        })
        .await;
        let message = f.finished(index as u64 + 1, true, false).await;
        assert!(
            message.contains("file name"),
            "unexpected rejection: {message}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "unsafe listing {name:?} made a local write"
        );
    }
    f.close().await;
}

#[test]
fn component_validation_sorting_hidden_permissions_and_local_summary() {
    for name in [
        "", ".", "..", "/tmp", r"C:\temp", "a/b", r"a\b", "CON", "NUL.txt", "a:", "a.", "a ", "a?",
        "a\n",
    ] {
        assert!(!ssh::safe_local_component(name), "{name:?}");
    }
    for name in ["hello.txt", ".env", "folder name", "file-1"] {
        assert!(ssh::safe_local_component(name));
    }
    assert_eq!(ssh::format_permissions(0o100755), "rwxr-xr-x");
    assert_eq!(ssh::format_permissions(0o100640), "rw-r-----");
    assert!(ssh::is_hidden_name(".env"));
    assert!(!ssh::is_hidden_name("visible"));
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("folder/empty")).unwrap();
    std::fs::write(dir.path().join("a.txt"), b"12345").unwrap();
    std::fs::write(dir.path().join("z.bin"), b"12").unwrap();
    let summary = ssh::local_tree_summary(dir.path()).unwrap();
    assert_eq!((summary.files, summary.dirs, summary.bytes), (2, 2, 7));
    let mut entries = ssh::read_local_dir(dir.path()).unwrap();
    for column in [
        ssh::SortColumn::Name,
        ssh::SortColumn::Date,
        ssh::SortColumn::Size,
        ssh::SortColumn::Kind,
    ] {
        for ascending in [true, false] {
            ssh::sort_local_files(&mut entries, column, ascending);
            assert!(entries[0].is_dir);
        }
    }
    ssh::sort_local_files(&mut entries, ssh::SortColumn::Size, true);
    assert_eq!(entries[1].name, "z.bin");
    ssh::sort_local_files(&mut entries, ssh::SortColumn::Kind, true);
    assert_eq!(entries[1].name, "z.bin");
    ssh::delete_local_tree(&dir.path().join("folder")).unwrap();
    assert!(!dir.path().join("folder").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_queue_reports_rejections_and_all_accepted_jobs_can_be_cancelled() {
    let mut f = Fixture::new().await;
    f.state.slow.store(true, Ordering::SeqCst);
    f.state
        .nodes
        .lock()
        .unwrap()
        .insert("/large.bin".into(), Node::File(vec![7; 8 * 1024 * 1024]));
    let dir = tempfile::tempdir().unwrap();
    f.send(Command::DownloadFile {
        local_root: dir.path().to_path_buf(),
        transfer: 1,
        remote: "/large.bin".into(),
        local: dir.path().join("1.bin"),
    })
    .await;
    loop {
        if let Event::TransferProgress { transfer: 1, .. } = next(&mut f.rx).await {
            break;
        }
    }
    for transfer in 2..=12 {
        f.send(Command::DownloadFile {
            local_root: dir.path().to_path_buf(),
            transfer,
            remote: "/large.bin".into(),
            local: dir.path().join(format!("{transfer}.bin")),
        })
        .await;
    }
    for transfer in 1..=12 {
        f.send(Command::CancelTransfer { transfer }).await;
    }
    let mut finished = HashSet::new();
    let mut rejected = 0;
    while finished.len() < 12 {
        if let Event::TransferFinished {
            transfer,
            error,
            cancelled,
            message,
            ..
        } = next(&mut f.rx).await
        {
            assert!(finished.insert(transfer), "duplicate terminal event");
            if error {
                rejected += 1;
                assert!(message.contains("queue"));
            } else {
                assert!(cancelled);
            }
        }
    }
    assert!(rejected >= 3, "queue must be bounded at 8 pending jobs");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    f.close().await;
}

#[test]
fn remote_sort_uses_numeric_dates_and_sizes_and_keeps_folders_first() {
    let file = |name: &str, dir, size, modified: &str| ssh::RemoteFile {
        name: name.into(),
        path: format!("/{name}"),
        is_dir: dir,
        size,
        modified: modified.into(),
        permissions: None,
    };
    let mut entries = vec![
        file("B.txt", false, 100, "9"),
        file("a.bin", false, 2, "10"),
        file("Z", true, 0, "0"),
    ];
    for (column, ascending, expected) in [
        (ssh::SortColumn::Name, true, "a.bin"),
        (ssh::SortColumn::Name, false, "B.txt"),
        (ssh::SortColumn::Date, true, "B.txt"),
        (ssh::SortColumn::Date, false, "a.bin"),
        (ssh::SortColumn::Size, true, "a.bin"),
        (ssh::SortColumn::Size, false, "B.txt"),
        (ssh::SortColumn::Kind, true, "a.bin"),
        (ssh::SortColumn::Kind, false, "B.txt"),
    ] {
        ssh::sort_remote_files(&mut entries, column, ascending);
        assert_eq!(entries[0].name, "Z");
        assert_eq!(entries[1].name, expected);
    }
}

