//! `workroamd`: run the Workroam background service.

use std::process::ExitCode;

fn usage() -> &'static str {
    "workroamd — Workroam background service\n\n\
     Usage: workroamd [--socket <path>] [--log-stderr]\n\
     \x20      workroamd --version\n\n\
     Started by the system at login (workroam config set autostart true),\n\
     by the desktop app, or by hand. Clients: the Workroam app and `workroam`."
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    let mut log_stderr = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--version" | "-V" => {
                println!("workroamd {}", workroam_core::VERSION);
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--log-stderr" => log_stderr = true,
            other => {
                eprintln!("unknown argument {other}\n\n{}", usage());
                return ExitCode::from(2);
            }
        }
    }
    let _guard = init_logging(log_stderr);
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot start: {e}");
            return ExitCode::from(1);
        }
    };
    rt.block_on(async move {
        let path = socket.unwrap_or_else(workroam_core::paths::socket_path);
        let d = workroamd::Daemon::new(true);
        workroamd::triggers::spawn(d.clone());
        workroamd::relay::ensure_running(&d);
        on_signal(d.clone());
        match workroamd::server::listen(d, &path).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!("workroamd is already running ({})", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("workroamd: cannot listen on {}: {e}", path.display());
                ExitCode::from(30)
            }
        }
    })
}

/// `SIGTERM` (logout / shutdown on macOS and Linux) and the Windows
/// shutdown and logoff events stop the service in order; Ctrl-C, the
/// console closing and `daemon.shutdown` stop it without the pre-shutdown
/// push.
fn on_signal(d: std::sync::Arc<workroamd::Daemon>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        let system = {
            use tokio::signal::unix::{signal, SignalKind};
            let (Ok(mut term), Ok(mut int)) = (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) else {
                return;
            };
            tokio::select! {
                _ = term.recv() => true,
                _ = int.recv() => false,
                _ = d.stop.notified() => false,
            }
        };
        #[cfg(windows)]
        let system = {
            use tokio::signal::windows::{ctrl_c, ctrl_close, ctrl_logoff, ctrl_shutdown};
            let (Ok(mut sd), Ok(mut lo), Ok(mut c), Ok(mut cl)) =
                (ctrl_shutdown(), ctrl_logoff(), ctrl_c(), ctrl_close())
            else {
                return;
            };
            tokio::select! {
                _ = sd.recv() => true,
                _ = lo.recv() => true,
                _ = c.recv() => false,
                _ = cl.recv() => false,
                _ = d.stop.notified() => false,
            }
        };
        #[cfg(not(any(unix, windows)))]
        let system = {
            d.stop.notified().await;
            false
        };
        // Let the reply to `daemon.shutdown` go out first.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        d.shutdown(system).await;
    });
}

fn init_logging(stderr: bool) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = tracing_subscriber::EnvFilter::try_from_env("WORKROAM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if stderr {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
        return None;
    }
    let dir = workroam_core::paths::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let appender = tracing_appender::rolling::daily(dir, "workroamd.log");
    let (w, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(w)
        .init();
    Some(guard)
}
