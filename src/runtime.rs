// Runtime detection. We pick a single socket per process and stick with it.
// Override via KOBURI_SOCKET; otherwise try docker, rootful podman,
// rootless podman. We do NOT keep the connection open; main.rs opens it
// per request (AGENTS.md rule 3).

use std::env;
use std::io;
use std::path::PathBuf;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Docker,
    Podman,
    Pm2,
}

#[derive(Debug, Clone)]
pub struct SocketResolution {
    pub runtime: Runtime,
    pub path: PathBuf,
}

#[cfg(unix)]
pub fn resolve() -> io::Result<Vec<SocketResolution>> {
    let mut resolutions = Vec::new();

    let mut resolved_any_env = false;

    if let Some(path_str) = env::var_os("SOCKET_DOCKER") {
        resolved_any_env = true;
        let path = PathBuf::from(path_str);
        if let Err(e) = probe(&path) {
            eprintln!("koburi: docker socket {} is unreachable: {e}", path.display());
        } else {
            resolutions.push(SocketResolution { runtime: Runtime::Docker, path });
        }
    }

    if let Some(path_str) = env::var_os("SOCKET_PODMAN") {
        resolved_any_env = true;
        let path = PathBuf::from(path_str);
        if let Err(e) = probe(&path) {
            eprintln!("koburi: podman socket {} is unreachable: {e}", path.display());
        } else {
            resolutions.push(SocketResolution { runtime: Runtime::Podman, path });
        }
    }

    if let Some(path_str) = env::var_os("SOCKET_PM2") {
        resolved_any_env = true;
        let path = PathBuf::from(path_str);
        if let Err(e) = probe(&path) {
            eprintln!("koburi: pm2 socket {} is unreachable: {e}", path.display());
        } else {
            resolutions.push(SocketResolution { runtime: Runtime::Pm2, path });
        }
    }

    if resolved_any_env {
        if resolutions.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "all configured environment sockets are unreachable",
            ));
        }
        return Ok(resolutions);
    }

    let candidates: &[(&str, &str, Runtime)] = &[
        ("/var/run/docker.sock", "docker", Runtime::Docker),
        ("/run/podman/podman.sock", "podman-rootful", Runtime::Podman),
    ];

    for &(path, label, runtime) in candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            match probe(&p) {
                Ok(()) => {
                    eprintln!("koburi: using {label} socket at {path}");
                    resolutions.push(SocketResolution {
                        runtime,
                        path: p,
                    });
                }
                Err(e) => {
                    eprintln!("koburi: {label} socket present but unreachable: {e}");
                }
            }
        }
    }

    // Rootless podman: /run/user/$UID/podman/podman.sock
    if let Some(uid) = env::var("UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .or_else(users_uid)
    {
        let p = PathBuf::from(format!("/run/user/{uid}/podman/podman.sock"));
        if p.exists() {
            if let Err(e) = probe(&p) {
                eprintln!("koburi: podman-rootless socket present but unreachable: {e}");
            } else {
                eprintln!("koburi: using podman-rootless socket at {}", p.display());
                resolutions.push(SocketResolution {
                    runtime: Runtime::Podman,
                    path: p,
                });
            }
        }
    }

    if resolutions.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no docker or podman socket found; set KOBURI_SOCKET",
        ));
    }

    Ok(resolutions)
}

#[cfg(not(unix))]
pub fn resolve() -> io::Result<Vec<SocketResolution>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "koburi runs on unix only",
    ))
}

#[cfg(unix)]
fn probe(path: &std::path::Path) -> io::Result<()> {
    let s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    s.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;
    Ok(())
}

// Best-effort UID lookup without depending on the `users` crate. Read
// /proc/self/status, look for "Uid:". This is only invoked when the env
// var UID is not set, which on a properly-launched daemon is rare.
#[cfg(unix)]
fn users_uid() -> Option<u32> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let mut it = rest.split_whitespace();
            let real = it.next()?;
            return real.parse().ok();
        }
    }
    None
}
