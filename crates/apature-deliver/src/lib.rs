//! Delivery: push finished JPEGs to a desk over FTP, FTPS (explicit TLS) or
//! SFTP. One `Target` describes where; the password or key passphrase is
//! handed in at send time and never stored here (the app keeps it in the
//! macOS Keychain). Every file is written to a temporary name and renamed
//! when complete, so a desk's watch folder never picks up a half file.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// A folder on this Mac or a mounted share: a NAS, a desk's SMB share,
    /// or the Google Drive / OneDrive / Dropbox desktop sync folder. `dir`
    /// is the folder path; host, port and user are unused.
    Folder,
    Ftp,
    /// FTP with explicit TLS (AUTH TLS) on the usual port 21.
    Ftps,
    Sftp,
}

/// Where photos go. `dir` is the remote folder, absolute or relative to the
/// login folder; empty means the login folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub kind: Kind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub dir: String,
    /// SFTP only: a private key file; empty = password login.
    #[serde(default)]
    pub key_file: Option<PathBuf>,
}

impl Target {
    pub fn default_port(kind: Kind) -> u16 {
        match kind {
            Kind::Ftp | Kind::Ftps => 21,
            Kind::Sftp => 22,
            Kind::Folder => 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DeliverError {
    #[error("connect to {0}: {1}")]
    Connect(String, String),
    #[error("login as {0} failed: {1}")]
    Login(String, String),
    #[error("remote folder {0}: {1}")]
    Folder(String, String),
    #[error("{0}: {1}")]
    File(PathBuf, String),
}

#[derive(Clone, Debug)]
pub enum Event {
    Started { index: usize },
    Sent { index: usize, bytes: u64 },
    Failed { index: usize, error: String },
    Finished,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub sent: usize,
    pub failed: Vec<(PathBuf, String)>,
    pub bytes: u64,
    pub elapsed_secs: f64,
}

/// The remote path for a local file: `dir` + the file name, with `/` joins.
pub fn remote_path(dir: &str, local: &Path) -> String {
    let name = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = dir.trim().trim_end_matches('/');
    if dir.is_empty() {
        name
    } else {
        format!("{dir}/{name}")
    }
}

trait Remote {
    fn put(&mut self, local: &Path, remote: &str) -> Result<u64, String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;
}

struct Ftp(suppaftp::NativeTlsFtpStream);

impl Remote for Ftp {
    fn put(&mut self, local: &Path, remote: &str) -> Result<u64, String> {
        let mut f = std::fs::File::open(local).map_err(|e| e.to_string())?;
        let n = self.0.put_file(remote, &mut f).map_err(|e| e.to_string())?;
        Ok(n)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.0.rename(from, to).map_err(|e| e.to_string())
    }
}

struct Sftp {
    _session: ssh2::Session,
    sftp: ssh2::Sftp,
}

impl Remote for Sftp {
    fn put(&mut self, local: &Path, remote: &str) -> Result<u64, String> {
        let mut src = std::fs::File::open(local).map_err(|e| e.to_string())?;
        let mut dst = self
            .sftp
            .create(Path::new(remote))
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            let n = src.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
        }
        Ok(total)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.sftp
            .rename(
                Path::new(from),
                Path::new(to),
                Some(ssh2::RenameFlags::OVERWRITE),
            )
            .map_err(|e| e.to_string())
    }
}

struct LocalFolder;

impl Remote for LocalFolder {
    fn put(&mut self, local: &Path, remote: &str) -> Result<u64, String> {
        let dst = Path::new(remote);
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let n = std::fs::copy(local, dst).map_err(|e| e.to_string())?;
        std::fs::File::open(dst)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(n)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }
}

/// Base64 without padding, the way ssh prints SHA256 fingerprints.
fn base64_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| chunk.get(i).copied().unwrap_or(0) as u32;
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// Host-key algorithms for the key types `known_hosts` lists for this
/// host and port (hashed entries too), in OpenSSH's order; empty if none.
fn host_key_algorithms(sess: &ssh2::Session, known_hosts: &str, host: &str, port: u16) -> String {
    let mut types = Vec::new();
    for line in known_hosts.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let Some(ty) = line.split_whitespace().nth(1) else {
            continue;
        };
        let Ok(mut one) = sess.known_hosts() else {
            continue;
        };
        // With a dummy key, a listed host reads as a mismatch, not unknown.
        if one.read_str(line, ssh2::KnownHostFileKind::OpenSSH).is_ok()
            && !matches!(
                one.check_port(host, port, &[0]),
                ssh2::CheckResult::NotFound
            )
        {
            types.push(ty.to_string());
        }
    }
    const ORDER: [(&str, &str); 6] = [
        ("ssh-ed25519", "ssh-ed25519"),
        ("ecdsa-sha2-nistp256", "ecdsa-sha2-nistp256"),
        ("ecdsa-sha2-nistp384", "ecdsa-sha2-nistp384"),
        ("ecdsa-sha2-nistp521", "ecdsa-sha2-nistp521"),
        ("ssh-rsa", "rsa-sha2-512,rsa-sha2-256,ssh-rsa"),
        ("ssh-dss", "ssh-dss"),
    ];
    ORDER
        .iter()
        .filter(|(t, _)| types.iter().any(|x| x == t))
        .map(|(_, a)| *a)
        .collect::<Vec<_>>()
        .join(",")
}

/// Check an SFTP server's host key against `known_hosts` text (the user's
/// `~/.ssh/known_hosts`, the file `ssh` itself uses). Fails closed: a
/// server that isn't listed, or whose key changed, is refused. Hashed
/// entries work: each line is fed to libssh2 on its own, markers and
/// unreadable lines are skipped rather than failing the whole file.
fn check_host_key(
    sess: &ssh2::Session,
    known_hosts: &str,
    host: &str,
    port: u16,
    key: &[u8],
    user: &str,
    fingerprint: &str,
) -> Result<(), DeliverError> {
    let addr = format!("{host}:{port}");
    let err = |m: String| DeliverError::Connect(addr.clone(), m);
    let mut kh = sess.known_hosts().map_err(|e| err(e.to_string()))?;
    for line in known_hosts.lines().map(str::trim) {
        // Markers (@cert-authority, @revoked) and lines libssh2 can't read
        // are skipped rather than failing the whole file.
        if !line.is_empty() && !line.starts_with('#') && !line.starts_with('@') {
            let _ = kh.read_str(line, ssh2::KnownHostFileKind::OpenSSH);
        }
    }
    let ssh_cmd = if port == 22 {
        format!("ssh {user}@{host}")
    } else {
        format!("ssh -p {port} {user}@{host}")
    };
    match kh.check_port(host, port, key) {
        ssh2::CheckResult::Match => Ok(()),
        ssh2::CheckResult::NotFound => Err(err(format!(
            "{host} is not in ~/.ssh/known_hosts (server key {fingerprint}); \
             connect to it once with `{ssh_cmd}` to record the key, then retry"
        ))),
        ssh2::CheckResult::Mismatch => Err(err(format!(
            "host key for {host} does NOT match ~/.ssh/known_hosts (server key {fingerprint}); \
             refusing to connect — if the server was replaced on purpose, update known_hosts first"
        ))),
        ssh2::CheckResult::Failure => Err(err(format!(
            "could not check the host key for {host} against ~/.ssh/known_hosts"
        ))),
    }
}

/// Check the SFTP server's host key against ~/.ssh/known_hosts before any
/// login. A mismatch fails, and so does an unknown host: the first trust has
/// to come from a real ssh client recording the key, not from us accepting
/// whatever a first connection presents.
fn verify_host_key(
    sess: &ssh2::Session,
    target: &Target,
    known_hosts: &str,
) -> Result<(), DeliverError> {
    let (key, _) = sess.host_key().ok_or_else(|| {
        DeliverError::Connect(
            format!("{}:{}", target.host, target.port),
            "server presented no host key".into(),
        )
    })?;
    let fingerprint = sess
        .host_key_hash(ssh2::HashType::Sha256)
        .map(|h| format!("SHA256:{}", base64_nopad(h)))
        .unwrap_or_else(|| "SHA256:unknown".into());
    check_host_key(
        sess,
        known_hosts,
        &target.host,
        target.port,
        key,
        &target.user,
        &fingerprint,
    )
}

fn connect(target: &Target, secret: &str) -> Result<Box<dyn Remote>, DeliverError> {
    let addr = format!("{}:{}", target.host, target.port);
    match target.kind {
        Kind::Folder => {
            let dir = Path::new(target.dir.trim());
            if target.dir.trim().is_empty() {
                return Err(DeliverError::Folder(
                    target.dir.clone(),
                    "no folder set".into(),
                ));
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| DeliverError::Folder(target.dir.clone(), e.to_string()))?;
            // Writable? A read-only share fails here, not mid-send.
            let probe = dir.join(".apature-write-test");
            std::fs::write(&probe, b"")
                .map_err(|e| DeliverError::Folder(target.dir.clone(), e.to_string()))?;
            let _ = std::fs::remove_file(&probe);
            Ok(Box::new(LocalFolder))
        }
        Kind::Ftp | Kind::Ftps => {
            let mut s = suppaftp::NativeTlsFtpStream::connect(&addr)
                .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            if target.kind == Kind::Ftps {
                let tls = suppaftp::native_tls::TlsConnector::new()
                    .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
                s = s
                    .into_secure(suppaftp::NativeTlsConnector::from(tls), &target.host)
                    .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            }
            s.login(&target.user, &secret.to_string())
                .map_err(|e| DeliverError::Login(target.user.clone(), e.to_string()))?;
            let _ = s.transfer_type(suppaftp::types::FileType::Binary);
            if !target.dir.trim().is_empty() {
                // Create the folder if the desk lets us; then it must exist.
                let _ = s.mkdir(target.dir.trim());
                s.cwd(target.dir.trim())
                    .map_err(|e| DeliverError::Folder(target.dir.clone(), e.to_string()))?;
            }
            Ok(Box::new(Ftp(s)))
        }
        Kind::Sftp => {
            let tcp = std::net::TcpStream::connect(&addr)
                .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            let mut sess = ssh2::Session::new()
                .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            sess.set_tcp_stream(tcp);
            let known = std::env::var_os("HOME")
                .map(|h| Path::new(&h).join(".ssh/known_hosts"))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            // Ask the server for the kind of key known_hosts has for it, so
            // the key `ssh` saved is the one we get to compare.
            let algos = host_key_algorithms(&sess, &known, &target.host, target.port);
            if !algos.is_empty() {
                let _ = sess.method_pref(ssh2::MethodType::HostKey, &algos);
            }
            sess.handshake()
                .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            verify_host_key(&sess, target, &known)?;
            match &target.key_file {
                Some(key) => sess
                    .userauth_pubkey_file(
                        &target.user,
                        None,
                        key,
                        Some(secret).filter(|s| !s.is_empty()),
                    )
                    .map_err(|e| DeliverError::Login(target.user.clone(), e.to_string()))?,
                None => sess
                    .userauth_password(&target.user, secret)
                    .map_err(|e| DeliverError::Login(target.user.clone(), e.to_string()))?,
            }
            let sftp = sess
                .sftp()
                .map_err(|e| DeliverError::Connect(addr.clone(), e.to_string()))?;
            if !target.dir.trim().is_empty() {
                let d = Path::new(target.dir.trim());
                if sftp.stat(d).is_err() {
                    sftp.mkdir(d, 0o755)
                        .map_err(|e| DeliverError::Folder(target.dir.clone(), e.to_string()))?;
                }
            }
            Ok(Box::new(Sftp {
                _session: sess,
                sftp,
            }))
        }
    }
}

/// Connect, log in and open the folder; nothing is sent.
pub fn test_connection(target: &Target, secret: &str) -> Result<(), DeliverError> {
    connect(target, secret).map(|_| ())
}

/// Send every file; each lands as `.name.part` and is renamed on completion.
pub fn send(
    target: &Target,
    secret: &str,
    files: &[PathBuf],
    mut on_event: impl FnMut(Event),
) -> Result<Report, DeliverError> {
    let t0 = Instant::now();
    let mut remote = connect(target, secret)?;
    // After cwd on FTP the folder is current, so names are relative there;
    // SFTP has no cwd, so names carry the folder.
    let base = match target.kind {
        Kind::Sftp | Kind::Folder => target.dir.clone(),
        _ => String::new(),
    };
    let mut report = Report::default();
    for (index, f) in files.iter().enumerate() {
        on_event(Event::Started { index });
        let final_name = remote_path(&base, f);
        let part = {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            remote_path(&base, Path::new(&format!(".{name}.part")))
        };
        // Three tries per file; a failed try reconnects first, since a
        // dropped FTP data connection usually takes the control one with it.
        let mut result: Result<u64, String> = Err(String::new());
        for attempt in 1..=3u32 {
            if attempt > 1 {
                std::thread::sleep(std::time::Duration::from_secs(attempt as u64));
                match connect(target, secret) {
                    Ok(r) => remote = r,
                    Err(e) => {
                        result = Err(format!("reconnect: {e}"));
                        continue;
                    }
                }
            }
            result = remote
                .put(f, &part)
                .and_then(|n| remote.rename(&part, &final_name).map(|_| n));
            match &result {
                Ok(_) => break,
                Err(e) => log::warn!(
                    "delivery try {attempt}/3 failed {} → {}: {e}",
                    f.display(),
                    target.host
                ),
            }
        }
        match result {
            Ok(n) => {
                report.sent += 1;
                report.bytes += n;
                log::info!("delivered {} → {}:{}", f.display(), target.host, final_name);
                on_event(Event::Sent { index, bytes: n });
            }
            Err(e) => {
                log::warn!("delivery failed {} → {}: {e}", f.display(), target.host);
                report.failed.push((f.clone(), e.clone()));
                on_event(Event::Failed { index, error: e });
            }
        }
    }
    report.elapsed_secs = t0.elapsed().as_secs_f64();
    on_event(Event::Finished);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sftp_host_keys_must_be_known_and_match() {
        // ssh-ed25519 blobs with 32 bytes of 0x01 and of 0x02.
        let blob = |b: u8| {
            let mut v = vec![0, 0, 0, 11];
            v.extend_from_slice(b"ssh-ed25519");
            v.extend_from_slice(&[0, 0, 0, 32]);
            v.extend_from_slice(&[b; 32]);
            v
        };
        let known = "# comment\n\
            @cert-authority *.example ssh-ed25519 AAAA\n\
            desk.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB\n\
            [alt.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB\n\
            garbage line\n";
        let sess = ssh2::Session::new().unwrap();
        let check = |host: &str, port: u16, key: &[u8]| {
            check_host_key(&sess, known, host, port, key, "me", "SHA256:test")
        };
        assert!(check("desk.example", 22, &blob(1)).is_ok());
        assert!(check("alt.example", 2222, &blob(1)).is_ok());
        let changed = check("desk.example", 22, &blob(2)).unwrap_err().to_string();
        assert!(changed.contains("does NOT match"), "{changed}");
        let unknown = check("other.example", 22, &blob(1))
            .unwrap_err()
            .to_string();
        assert!(unknown.contains("ssh me@other.example"), "{unknown}");
        // The port matters: alt.example on 22 isn't the listed server.
        assert!(check("alt.example", 22, &blob(1)).is_err());
        // The algorithms asked for follow what is listed for that host.
        assert_eq!(
            host_key_algorithms(&sess, known, "desk.example", 22),
            "ssh-ed25519"
        );
        assert_eq!(host_key_algorithms(&sess, known, "other.example", 22), "");
        let rsa = "desk.example ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQC7\n";
        let both = format!("{known}{rsa}");
        let a = host_key_algorithms(&sess, &both, "desk.example", 22);
        assert!(
            a == "ssh-ed25519" || a.starts_with("ssh-ed25519,rsa-sha2-512"),
            "{a}"
        );
        // No known_hosts at all: refused.
        assert!(
            check_host_key(&sess, "", "desk.example", 22, &blob(1), "me", "SHA256:test").is_err()
        );
    }

    #[test]
    fn remote_paths_join_with_slashes() {
        assert_eq!(remote_path("", Path::new("/a/b/X.JPG")), "X.JPG");
        assert_eq!(
            remote_path("/incoming/", Path::new("/a/X.JPG")),
            "/incoming/X.JPG"
        );
        assert_eq!(remote_path("photos", Path::new("X.JPG")), "photos/X.JPG");
    }

    #[test]
    fn folder_send_lands_files_under_final_names() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let a = src.path().join("IMG_0001.JPG");
        let b = src.path().join("IMG_0002.JPG");
        std::fs::write(&a, vec![3u8; 10_000]).unwrap();
        std::fs::write(&b, vec![9u8; 2_500]).unwrap();
        let target = Target {
            name: "folder test".into(),
            kind: Kind::Folder,
            host: String::new(),
            port: Target::default_port(Kind::Folder),
            user: String::new(),
            dir: out.path().to_string_lossy().into_owned(),
            key_file: None,
        };
        let rep = send(&target, "", &[a, b], |_| {}).unwrap();
        assert_eq!(rep.sent, 2);
        assert!(rep.failed.is_empty(), "failures: {:?}", rep.failed);
        assert_eq!(rep.bytes, 12_500);
        assert_eq!(
            std::fs::read(out.path().join("IMG_0001.JPG")).unwrap(),
            vec![3u8; 10_000]
        );
        assert_eq!(
            std::fs::read(out.path().join("IMG_0002.JPG")).unwrap(),
            vec![9u8; 2_500]
        );
        // Only the two final names: no `.part` file is left behind.
        let mut names: Vec<String> = std::fs::read_dir(out.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["IMG_0001.JPG", "IMG_0002.JPG"]);
    }

    /// Runs only with a local FTP server: `APATURE_FTP_TEST=host:port:user:pass:dir`.
    #[test]
    fn ftp_round_trip_when_a_server_is_given() {
        let Ok(spec) = std::env::var("APATURE_FTP_TEST") else {
            return;
        };
        let p: Vec<&str> = spec.splitn(5, ':').collect();
        let target = Target {
            name: "test".into(),
            kind: Kind::Ftp,
            host: p[0].into(),
            port: p[1].parse().unwrap(),
            user: p[2].into(),
            dir: p[4].into(),
            key_file: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("TEST-001.JPG");
        std::fs::write(&f, vec![7u8; 100_000]).unwrap();
        let mut events = 0;
        let rep = send(&target, p[3], &[f], |_| events += 1).unwrap();
        assert_eq!(rep.sent, 1);
        assert_eq!(rep.bytes, 100_000);
        assert!(events >= 3);
    }
}
