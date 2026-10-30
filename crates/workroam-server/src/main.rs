//! `workroam-server` (architecture §18): the optional server. Relay mode
//! (default) connects cloud agents to devices and holds no storage
//! credentials or keys; full mode (`--full`, cargo feature `full`) adds
//! pairing codes with per-device keys, ntfy forwarding and the web reader.

mod app;
#[cfg(feature = "full")]
mod full;
mod mcp;
mod oauth;
mod relay;
mod state;
mod tunnel;

use clap::Parser;
use std::time::Duration;
use workroam_ipc::relay::BindCode;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(
    name = "workroam-server",
    version = VERSION,
    about = "Optional Workroam server: remote MCP connector and device relay",
    after_help = "Docs: https://github.com/kverona-ai/workroam/blob/main/docs/architecture.md#18-轻量服务端可选"
)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8787", env = "WORKROAM_SERVER_LISTEN")]
    listen: String,
    /// The public https:// address that reaches this port.
    #[arg(long, env = "WORKROAM_SERVER_PUBLIC_URL")]
    public_url: Option<String>,
    /// Open an outbound cloudflared tunnel for a public HTTPS address.
    #[arg(long)]
    tunnel: bool,
    /// Bind to a device with the one-time code from `workroam agent connect`.
    #[arg(long, value_name = "CODE")]
    bind: Option<String>,
    /// State file (default ~/.workroam-server/state.json).
    #[arg(long, env = "WORKROAM_SERVER_STATE")]
    state: Option<std::path::PathBuf>,
    /// Full mode: pairing codes, per-device keys, ntfy, web reader.
    #[arg(long)]
    full: bool,
    #[cfg(feature = "full")]
    #[command(flatten)]
    full_args: full::FullArgs,
    /// Print the state summary and exit.
    #[arg(long)]
    status: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("WORKROAM_LOG")
                .unwrap_or_else(|_| "workroam_server=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    if let Err(e) = run(args).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), String> {
    let state_path = args
        .state
        .clone()
        .unwrap_or_else(state::Persist::default_path);
    let mut persist =
        state::Persist::load(&state_path).map_err(|e| format!("{}: {e}", state_path.display()))?;
    if let Some(code) = &args.bind {
        let b = BindCode::decode(code)
            .ok_or("this is not a Workroam bind code (it starts with wrb1:)")?;
        if b.expires < chrono::Utc::now().timestamp() {
            return Err(
                "the bind code expired; create a new one with `workroam agent connect`".into(),
            );
        }
        if persist.relay_id.as_deref().is_some_and(|r| r != b.relay_id) {
            // A new workspace or a fresh bind: start over.
            persist = state::Persist::default();
        }
        persist.relay_id = Some(b.relay_id.clone());
        if !persist
            .devices
            .iter()
            .any(|d| d.id == b.device_id && d.pubkey == b.pubkey)
        {
            persist.bind = Some(b);
        }
    }
    persist.gc();
    persist
        .save(&state_path)
        .map_err(|e| format!("{}: {e}", state_path.display()))?;
    if args.status {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "relay_id": persist.relay_id,
                "waiting_for_bind": persist.bind.as_ref().map(|b| &b.device_name),
                "devices": persist.devices.iter().map(|d| &d.name).collect::<Vec<_>>(),
                "connections": persist.connections.iter().filter(|c| !c.revoked).map(|c| serde_json::json!({"agent": c.agent_name, "access": c.access, "projects": c.projects})).collect::<Vec<_>>(),
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    if args.full && !cfg!(feature = "full") {
        return Err(
            "this build has no full mode; use the container image or build with --features full"
                .into(),
        );
    }
    let public = args
        .public_url
        .clone()
        .map(|u| u.trim_end_matches('/').to_string());
    if let Some(u) = &public {
        if !u.starts_with("https://") {
            eprintln!("warning: {u} is not https://; cloud agents cannot sign in until the address is HTTPS");
        }
    }
    let cfg = app::Config {
        listen: args.listen.clone(),
        public_url: std::sync::Mutex::new(public.clone()),
        state_path: state_path.clone(),
        #[cfg(feature = "full")]
        full: if args.full {
            Some(full::FullConfig::from_args(&args.full_args)?)
        } else {
            None
        },
    };
    let bound_to = persist.bind.as_ref().map(|b| b.device_name.clone());
    let devices: Vec<String> = persist.devices.iter().map(|d| d.name.clone()).collect();
    let app = app::App::new(cfg, persist);
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", args.listen))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(8787);
    let mode = if args.full { "full mode" } else { "relay mode" };
    println!("workroam-server {VERSION} ({mode})");
    println!("listening on {}", args.listen);
    if let Some(u) = &public {
        println!("public address: {u}");
        println!("MCP connector:  {u}/mcp");
    }
    match (&bound_to, devices.is_empty()) {
        (Some(n), _) => {
            println!("waiting for device {n} to connect (bind code valid for 30 minutes)")
        }
        (None, false) => println!("bound devices: {}", devices.join(", ")),
        (None, true) => println!(
            "not bound yet: run `workroam agent connect <agent>` on your computer for a bind code"
        ),
    }
    if args.tunnel {
        let dir = state_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| ".".into());
        println!("starting the tunnel…");
        tunnel::spawn(app.clone(), dir, port);
    }
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(300));
            loop {
                t.tick().await;
                mcp::sweep(&app);
                app.update(|_| ());
            }
        });
    }
    let router = app::router(app);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|e| e.to_string())
}
