//! Workroam IPC (architecture §15): JSON-RPC 2.0, one JSON object per
//! line, over a Unix domain socket (macOS / Linux) or a named pipe
//! (Windows). The desktop app, the CLI, `workroam mcp` and the device relay
//! are all clients of `workroamd`.

pub mod client;
pub mod methods;
pub mod parity;
pub mod protocol;
pub mod relay;
pub mod transport;

pub use client::Client;
pub use methods::{is_known, Method, EVENTS, METHODS};
pub use protocol::{
    ClientKind, Hello, HelloReply, Message, Notification, Origin, Request, Response, RpcError,
    PROTOCOL, SCHEMA_VERSION,
};
