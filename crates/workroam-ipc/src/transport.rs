//! Local transport: Unix domain socket (0600) or a Windows named pipe that
//! refuses remote clients. Both carry newline-delimited JSON.

use std::io;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncWrite};

pub trait Duplex: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Duplex for T {}

pub type Stream = Box<dyn Duplex>;

#[cfg(unix)]
pub struct Listener {
    inner: tokio::net::UnixListener,
}

#[cfg(windows)]
pub struct Listener {
    name: String,
    next: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl Listener {
    /// Listen on `path`. Fails with `AddrInUse` when another daemon answers.
    #[cfg(unix)]
    pub async fn bind(path: &Path) -> io::Result<Listener> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
            }
        }
        if path.exists() {
            if tokio::net::UnixStream::connect(path).await.is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "workroamd is already running",
                ));
            }
            let _ = std::fs::remove_file(path);
        }
        let inner = tokio::net::UnixListener::bind(path)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener { inner })
    }

    #[cfg(windows)]
    pub async fn bind(path: &Path) -> io::Result<Listener> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = path.to_string_lossy().to_string();
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    io::Error::new(io::ErrorKind::AddrInUse, "workroamd is already running")
                } else {
                    e
                }
            })?;
        Ok(Listener {
            name,
            next: Some(first),
        })
    }

    #[cfg(unix)]
    pub async fn accept(&mut self) -> io::Result<Stream> {
        let (s, _) = self.inner.accept().await?;
        Ok(Box::new(s))
    }

    #[cfg(windows)]
    pub async fn accept(&mut self) -> io::Result<Stream> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let server = match self.next.take() {
            Some(s) => s,
            None => ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?,
        };
        server.connect().await?;
        self.next = Some(
            ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?,
        );
        Ok(Box::new(server))
    }
}

#[cfg(unix)]
pub async fn connect(path: &Path) -> io::Result<Stream> {
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

#[cfg(windows)]
pub async fn connect(path: &Path) -> io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let name = path.to_string_lossy().to_string();
    for _ in 0..50 {
        match ClientOptions::new().open(&name) {
            Ok(c) => return Ok(Box::new(c)),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "workroamd pipe is busy",
    ))
}
