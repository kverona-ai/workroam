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
