//! Run the relay binary and play both sides: a device on `/relay` and a
//! cloud agent doing OAuth (DCR + PKCE) and MCP calls.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use workroam_ipc::relay::{self as proto, BindCode, FromDevice, ToDevice};

struct Server {
    child: std::process::Child,
    base: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start(state: &std::path::Path, bind: Option<&str>) -> Server {
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_workroam-server"));
    c.args([
        "--listen",
        &format!("127.0.0.1:{port}"),
        "--public-url",
        &base,
        "--state",
    ])
    .arg(state)
    .stdout(std::process::Stdio::null());
    if let Some(b) = bind {
        c.args(["--bind", b]);
    }
    let child = c.spawn().unwrap();
    let s = Server { child, base };
    for _ in 0..100 {
        if reqwest::get(format!("{}/healthz", s.base)).await.is_ok() {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("server did not start");
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn recv(ws: &mut Ws) -> ToDevice {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("frame in time")
            .unwrap()
            .unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

async fn send(ws: &mut Ws, m: &FromDevice) {
    ws.send(Message::Text(serde_json::to_string(m).unwrap().into()))
        .await
        .unwrap();
}

async fn device(base: &str, key: &SigningKey, id: &str) -> (Ws, ToDevice) {
    let url = format!("{}/relay", base.replace("http://", "ws://"));
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let ToDevice::Challenge { relay_id, nonce } = recv(&mut ws).await else {
        panic!("challenge first")
    };
    let sig = key.sign(&proto::auth_message(&relay_id, id, &nonce));
    send(
        &mut ws,
        &FromDevice::Auth {
            device_id: id.into(),
            device_name: "office".into(),
            pubkey: hex::encode(key.verifying_key().to_bytes()),
            signature: hex::encode(sig.to_bytes()),
            bind_code: None,
        },
    )
    .await;
    let w = recv(&mut ws).await;
    (ws, w)
}

fn pkce() -> (String, String) {
    let v = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 40]);
    let c = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(<sha2::Sha256 as sha2::Digest>::digest(v.as_bytes()));
    (v, c)
}

async fn mcp(http: &reqwest::Client, base: &str, token: &str, body: Value) -> reqwest::Response {
    http.post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn bind_authorize_call_revoke() {
    let td = tempfile::tempdir().unwrap();
    let state = td.path().join("state.json");
    let key = SigningKey::from_bytes(&[3u8; 32]);
    let code = BindCode {
        relay_id: "r_test".into(),
        device_id: "d_1".into(),
        device_name: "office".into(),
        pubkey: hex::encode(key.verifying_key().to_bytes()),
        expires: chrono::Utc::now().timestamp() + 1800,
    }
    .encode();
    let srv = start(&state, Some(&code)).await;
    let http = reqwest::Client::new();

    // A stranger cannot connect; the bound device can.
    let other = SigningKey::from_bytes(&[9u8; 32]);
    let (_, w) = device(&srv.base, &other, "d_x").await;
    assert!(matches!(w, ToDevice::Error { .. }), "{w:?}");
    let (mut ws, w) = device(&srv.base, &key, "d_1").await;
    let ToDevice::Welcome { relay_id, .. } = w else {
        panic!("{w:?}")
    };
    assert_eq!(relay_id, "r_test");
    let st = std::fs::read_to_string(&state).unwrap();
    assert!(
        st.contains("\"d_1\"") && !st.contains("\"bind\": {"),
        "bound and the bind code is used: {st}"
    );

    // OAuth metadata and dynamic client registration.
    let meta: Value = http
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            srv.base
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["code_challenge_methods_supported"][0], "S256");
    let r = mcp(
        &http,
        &srv.base,
        "nope",
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(r.status(), 401);
    assert!(r.headers()["www-authenticate"]
        .to_str()
        .unwrap()
        .contains("resource_metadata"));
    let reg: Value = http
        .post(format!("{}/oauth/register", srv.base))
        .json(&json!({"client_name": "Grok", "redirect_uris": ["http://127.0.0.1:9/cb"]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let client_id = reg["client_id"].as_str().unwrap().to_string();

    // Authorize: the page shows the code, the device gets the same code.
    let (verifier, challenge) = pkce();
    let page = http
        .get(format!("{}/oauth/authorize", srv.base))
        .query(&[
            ("response_type", "code"),
            ("client_id", &client_id),
            ("redirect_uri", "http://127.0.0.1:9/cb"),
            ("state", "xyz"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let ToDevice::Authorize {
        request_id,
        connection_id,
        agent,
        code,
        ..
    } = recv(&mut ws).await
    else {
        panic!()
    };
    assert_eq!(agent, "grok-bot");
    assert!(page.contains(&code), "the page shows the confirmation code");
    let projects = vec!["demo".to_string()];
    // A forged grant (wrong key) is ignored.
    let bad = other.sign(&proto::grant_message(
        "r_test",
        &connection_id,
        "sync",
        &projects,
    ));
    send(
        &mut ws,
        &FromDevice::Grant {
            request_id: request_id.clone(),
            connection_id: connection_id.clone(),
            access: "sync".into(),
            projects: projects.clone(),
            signature: hex::encode(bad.to_bytes()),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let s: Value = http
        .get(format!("{}/oauth/authorize/status", srv.base))
        .query(&[("id", &request_id)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["status"], "pending");
    let sig = key.sign(&proto::grant_message(
        "r_test",
        &connection_id,
        "sync",
        &projects,
    ));
    send(
        &mut ws,
        &FromDevice::Grant {
            request_id: request_id.clone(),
            connection_id: connection_id.clone(),
            access: "sync".into(),
            projects,
            signature: hex::encode(sig.to_bytes()),
        },
    )
    .await;
    let mut redirect = String::new();
    for _ in 0..50 {
        let s: Value = http
            .get(format!("{}/oauth/authorize/status", srv.base))
            .query(&[("id", &request_id)])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if s["status"] == "granted" {
            redirect = s["redirect"].as_str().unwrap().to_string();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let u = url::Url::parse(&redirect).unwrap();
    let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert_eq!(q["state"], "xyz");

    // Token exchange checks PKCE.
    let tok = |verifier: &str| {
        http.post(format!("{}/oauth/token", srv.base)).form(&[
            ("grant_type", "authorization_code"),
            ("code", q["code"].as_str()),
            ("client_id", client_id.as_str()),
            ("redirect_uri", "http://127.0.0.1:9/cb"),
            ("code_verifier", verifier),
        ])
    };
    let wrong = tok(&"x".repeat(43)).send().await.unwrap();
    assert_eq!(wrong.status(), 400);
    // The code was consumed by the failed attempt: authorization codes are single use.
    let again = tok(&verifier).send().await.unwrap();
    assert_eq!(again.status(), 400);
    // Second round, done right.
    let page = http
        .get(format!("{}/oauth/authorize", srv.base))
        .query(&[
            ("response_type", "code"),
            ("client_id", &client_id),
            ("redirect_uri", "http://127.0.0.1:9/cb"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await
        .unwrap();
    assert!(page.status().is_success());
    let ToDevice::Authorize {
        request_id,
        connection_id: c2,
        ..
    } = recv(&mut ws).await
    else {
        panic!()
    };
    let projects = vec!["demo".to_string()];
    let sig = key.sign(&proto::grant_message("r_test", &c2, "sync", &projects));
    send(
        &mut ws,
        &FromDevice::Grant {
            request_id: request_id.clone(),
            connection_id: c2.clone(),
            access: "sync".into(),
            projects,
            signature: hex::encode(sig.to_bytes()),
        },
    )
    .await;
    let mut code2 = String::new();
    for _ in 0..50 {
        let s: Value = http
            .get(format!("{}/oauth/authorize/status", srv.base))
            .query(&[("id", &request_id)])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(r) = s["redirect"].as_str() {
            code2 = url::Url::parse(r)
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "code")
                .unwrap()
                .1
                .into_owned();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let t: Value = http
        .post(format!("{}/oauth/token", srv.base))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code2),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = t["access_token"].as_str().unwrap().to_string();
    let refresh = t["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(t["expires_in"], 3600);
    assert!(
        !std::fs::read_to_string(&state).unwrap().contains(&access),
        "tokens are stored hashed"
    );

    // MCP: initialize, tools, a call answered by the device.
    let init: Value = mcp(&http, &srv.base, &access, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).await.json().await.unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "workroam");
    let tools: Value = mcp(
        &http,
        &srv.base,
        &access,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
    let note = mcp(
        &http,
        &srv.base,
        &access,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(note.status(), 202);

    let call = {
        let http = http.clone();
        let base = srv.base.clone();
        let access = access.clone();
        tokio::spawn(async move {
            mcp(&http, &base, &access, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "status", "arguments": {"project": "demo"}}}))
                .await
                .json::<Value>()
                .await
                .unwrap()
        })
    };
    let ToDevice::Call {
        call_id,
        tool,
        connection_id: cid,
        args,
        ..
    } = recv(&mut ws).await
    else {
        panic!()
    };
    assert_eq!(
        (tool.as_str(), cid.as_str(), args["project"].as_str()),
        ("status", c2.as_str(), Some("demo"))
    );
    send(
        &mut ws,
        &FromDevice::Result {
            call_id,
            ok: true,
            result: json!({"project": "demo", "lease": null}),
            error: None,
        },
    )
    .await;
    let r = call.await.unwrap();
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(r["result"]["structuredContent"]["project"], "demo");

    // Out of scope: refused at the relay, never forwarded.
    let r: Value = mcp(&http, &srv.base, &access, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "history", "arguments": {"project": "secret"}}})).await.json().await.unwrap();
    assert_eq!(r["result"]["isError"], true);

    // Refresh rotates the refresh token.
    let t2: Value = http
        .post(format!("{}/oauth/token", srv.base))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(t2["access_token"].is_string());
    let reuse = http
        .post(format!("{}/oauth/token", srv.base))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
        .send()
        .await
        .unwrap();
    assert_eq!(reuse.status(), 400);

    // Offline device: reads say so, pushes are queued and delivered on reconnect.
    drop(ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a2 = t2["access_token"].as_str().unwrap();
    let r: Value = mcp(&http, &srv.base, a2, json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "request_push", "arguments": {"project": "demo", "release": true}}})).await.json().await.unwrap();
    assert_eq!(r["result"]["structuredContent"]["queued"], true, "{r}");
    let (mut ws, _) = device(&srv.base, &key, "d_1").await;
    let m = recv(&mut ws).await;
    assert!(
        matches!(m, ToDevice::Call { ref tool, .. } if tool == "request_push"),
        "{m:?}"
    );

    // Revoking on the device kills the tokens.
    send(
        &mut ws,
        &FromDevice::Revoke {
            connection_id: c2.clone(),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let r = mcp(
        &http,
        &srv.base,
        a2,
        json!({"jsonrpc": "2.0", "id": 6, "method": "tools/list"}),
    )
    .await;
    assert_eq!(r.status(), 401);
    let _ = connection_id;
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_public_address_refuses_oauth() {
    let td = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_workroam-server"))
        .args([
            "--listen",
            &format!("127.0.0.1:{port}"),
            "--public-url",
            "http://relay.example.com",
            "--state",
        ])
        .arg(td.path().join("s.json"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let mut ok = false;
    for _ in 0..100 {
        if reqwest::get(format!("{base}/healthz")).await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok);
    let r = reqwest::Client::new()
        .post(format!("{base}/oauth/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", "x")])
        .send()
        .await
        .unwrap();
    let body: Value = r.json().await.unwrap();
    assert!(body["error_description"]
        .as_str()
        .unwrap()
        .contains("HTTPS"));
    let _ = child.kill();
    let _ = child.wait();
}
