//! Check registry (architecture §7.5).

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use workroam_core::config::{Addressing, Capabilities, WorkspaceLocation};
use workroam_core::text::tr;
use workroam_store::{Creds, S3Client};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Info,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub group: String,
    pub label: String,
    pub status: CheckStatus,
    pub message: String,
    pub hint: Option<String>,
    pub duration_ms: u64,
}

impl CheckResult {
    fn new(id: &str, group: &str, status: CheckStatus, message: impl Into<String>) -> Self {
        CheckResult {
            id: id.into(),
            group: group.into(),
            label: tr(&format!("check.{id}")).to_string(),
            status,
            message: message.into(),
            hint: None,
            duration_ms: 0,
        }
    }
    fn hint(mut self, key: &str) -> Self {
        self.hint = Some(tr(key).to_string());
        self
    }
}

/// What the storage checks learned (written into machine.toml on commit).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Findings {
    pub addressing: Option<Addressing>,
    pub capabilities: Capabilities,
    pub broad_key: bool,
    pub bucket_exists: bool,
}

pub struct StorageTarget<'a> {
    pub loc: &'a WorkspaceLocation,
    pub creds: Option<&'a Creds>,
    /// The bucket may not exist yet (setup "create"): bucket-level probes are skipped.
    pub bucket_may_be_missing: bool,
}

fn timed(start: Instant, mut r: CheckResult) -> CheckResult {
    r.duration_ms = start.elapsed().as_millis() as u64;
    r
}

pub fn all_ok(results: &[CheckResult]) -> bool {
    !results.iter().any(|r| r.status == CheckStatus::Fail)
}

/// Run every storage check, calling `each` as soon as one finishes.
pub async fn storage_checks(
    t: &StorageTarget<'_>,
    each: &mut (dyn FnMut(&CheckResult) + Send),
) -> (Vec<CheckResult>, Findings) {
    let mut out = Vec::new();
    let mut f = Findings::default();
    let mut push = |r: CheckResult, out: &mut Vec<CheckResult>| {
        each(&r);
        out.push(r);
    };
    if t.loc.is_local() {
        let start = Instant::now();
        let root = t.loc.local_root().unwrap_or_default();
        let ok = std::fs::create_dir_all(&root).is_ok() && {
            let p = root.join(".workroam-probe");
            let w = std::fs::write(&p, b"x").is_ok();
            let _ = std::fs::remove_file(&p);
            w
        };
        push(
            timed(
                start,
                if ok {
                    CheckResult::new(
                        "rwd",
                        "storage",
                        CheckStatus::Pass,
                        root.display().to_string(),
                    )
                } else {
                    CheckResult::new(
                        "rwd",
                        "storage",
                        CheckStatus::Fail,
                        format!("{} is not writable", root.display()),
                    )
                    .hint("hint.rwd")
                },
            ),
            &mut out,
        );
        f.capabilities = Capabilities {
            conditional_create: true,
            conditional_overwrite: true,
            multipart: true,
        };
        f.bucket_exists = true;
        f.addressing = Some(Addressing::Path);
        return (out, f);
    }
    let Some(creds) = t.creds else {
        push(
            CheckResult::new("credentials", "storage", CheckStatus::Fail, "no access key")
                .hint("hint.credentials"),
            &mut out,
        );
        return (out, f);
    };
    let url = match url::Url::parse(&t.loc.endpoint) {
        Ok(u) => u,
        Err(e) => {
            push(
                CheckResult::new("dns", "storage", CheckStatus::Fail, e.to_string())
                    .hint("hint.dns"),
                &mut out,
            );
            return (out, f);
        }
    };
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(443);

    // DNS
    let start = Instant::now();
    match tokio::net::lookup_host((host.as_str(), port)).await {
        Ok(mut addrs) => {
            let a = addrs.next().map(|a| a.ip().to_string()).unwrap_or_default();
            push(
                timed(
                    start,
                    CheckResult::new("dns", "storage", CheckStatus::Pass, format!("{host} → {a}")),
                ),
                &mut out,
            );
        }
        Err(e) => {
            push(
                timed(
                    start,
                    CheckResult::new("dns", "storage", CheckStatus::Fail, format!("{host}: {e}"))
                        .hint("hint.dns"),
                ),
                &mut out,
            );
            return (out, f);
        }
    }

    // TLS + clock (one plain request).
    let start = Instant::now();
    let client = workroam_store::s3raw::http_client(Duration::from_secs(15));
    let resp = client.get(url.as_str()).send().await;
    match &resp {
        Ok(r) => {
            let tls = if url.scheme() == "https" {
                CheckResult::new(
                    "tls",
                    "storage",
                    CheckStatus::Pass,
                    "certificate chain verified",
                )
            } else {
                CheckResult::new(
                    "tls",
                    "storage",
                    CheckStatus::Warn,
                    "plain http: only use this for local testing",
                )
                .hint("hint.tls")
            };
            push(timed(start, tls), &mut out);
            let skew = r
                .headers()
                .get("date")
                .and_then(|d| d.to_str().ok())
                .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
                .map(|d| {
                    (chrono::Utc::now() - d.with_timezone(&chrono::Utc))
                        .num_seconds()
                        .abs()
                });
            let clock = match skew {
                Some(s) if s > 300 => CheckResult::new(
                    "clock",
                    "storage",
                    CheckStatus::Fail,
                    format!("{s} s difference"),
                )
                .hint("hint.clock"),
                Some(s) => CheckResult::new(
                    "clock",
                    "storage",
                    CheckStatus::Pass,
                    format!("{s} s difference"),
                ),
                None => CheckResult::new(
                    "clock",
                    "storage",
                    CheckStatus::Info,
                    "server sent no Date header",
                ),
            };
            push(clock, &mut out);
        }
        Err(e) => {
            let msg = format!("{e:?}");
            let cert =
                msg.to_lowercase().contains("certificate") || msg.to_lowercase().contains("tls");
            push(
                timed(
                    start,
                    CheckResult::new(
                        "tls",
                        "storage",
                        CheckStatus::Fail,
                        if cert {
                            "certificate rejected".to_string()
                        } else {
                            e.to_string()
                        },
                    )
                    .hint(if cert { "hint.tls" } else { "hint.dns" }),
                ),
                &mut out,
            );
            return (out, f);
        }
    }

    // Addressing: virtual host first, then path (§7.5).
    let start = Instant::now();
    let region = if t.loc.region.is_empty() {
        "us-east-1"
    } else {
        t.loc.region.as_str()
    };
    let mut chosen = None;
    let order = match t.loc.addressing {
        Addressing::Path => [Addressing::Path, Addressing::VirtualHost],
        Addressing::VirtualHost => [Addressing::VirtualHost, Addressing::Path],
    };
    let mut last_err = String::new();
    let mut cred_fail = false;
    for a in order {
        let Ok(c) = S3Client::new(&t.loc.endpoint, region, a, creds.clone()) else {
            continue;
        };
        match c.head_bucket(&t.loc.bucket).await {
            Ok(r) if r.ok() => {
                chosen = Some((a, true));
                break;
            }
            Ok(r) if r.status == 404 && t.bucket_may_be_missing => {
                // The bucket does not exist yet; listing tells whether the style works.
                if let Ok(l) = c.list_buckets().await {
                    if l.ok() {
                        chosen = Some((a, false));
                        break;
                    }
                    if l.status == 403 {
                        cred_fail = true;
                    }
                }
            }
            Ok(r) => {
                if r.status == 403
                    && matches!(
                        r.error_code().as_deref(),
                        Some("InvalidAccessKeyId") | Some("SignatureDoesNotMatch")
                    )
                {
                    cred_fail = true;
                }
                last_err = format!("HTTP {} {}", r.status, r.error_code().unwrap_or_default());
            }
            Err(e) => last_err = e.message,
        }
    }
    let creds_check = if cred_fail && chosen.is_none() {
        CheckResult::new(
            "credentials",
            "storage",
            CheckStatus::Fail,
            "the storage rejected the access key",
        )
        .hint("hint.credentials")
    } else {
        CheckResult::new(
            "credentials",
            "storage",
            CheckStatus::Pass,
            format!(
                "access key {}…",
                creds.access_key.chars().take(6).collect::<String>()
            ),
        )
    };
    let creds_failed = creds_check.status == CheckStatus::Fail;
    push(creds_check, &mut out);
    if creds_failed {
        return (out, f);
    }
    let Some((addr, exists)) = chosen else {
        push(
            timed(
                start,
                CheckResult::new("addressing", "storage", CheckStatus::Fail, last_err.clone())
                    .hint("hint.addressing"),
            ),
            &mut out,
        );
        push(
            CheckResult::new(
                "bucket",
                "storage",
                CheckStatus::Fail,
                format!("{}: {last_err}", t.loc.bucket),
            )
            .hint("hint.bucket"),
            &mut out,
        );
        return (out, f);
    };
    f.addressing = Some(addr);
    f.bucket_exists = exists;
    push(
        timed(
            start,
            CheckResult::new("addressing", "storage", CheckStatus::Pass, addr.as_str()),
        ),
        &mut out,
    );
    let c = S3Client::new(&t.loc.endpoint, region, addr, creds.clone()).expect("validated");
    if !exists {
        push(
            CheckResult::new(
                "bucket",
                "storage",
                CheckStatus::Info,
                format!("{} will be created", t.loc.bucket),
            ),
            &mut out,
        );
        // Probes below need the bucket; they run again after it is created.
        f.capabilities = Capabilities {
            conditional_create: true,
            conditional_overwrite: true,
            multipart: true,
        };
        return (out, f);
    }
    push(
        CheckResult::new("bucket", "storage", CheckStatus::Pass, t.loc.bucket.clone()),
        &mut out,
    );

    let probe = format!("{}control/.probe-{}", t.loc.key_prefix(), hex_id());
    // Read / write / delete
    let start = Instant::now();
    let rwd = async {
        let w = c
            .put_object(&t.loc.bucket, &probe, b"workroam".to_vec(), &[])
            .await?;
        if !w.ok() {
            return Ok::<_, workroam_core::Error>(format!("write: HTTP {}", w.status));
        }
        let r = c.get_object(&t.loc.bucket, &probe).await?;
        if !r.ok() || r.body != b"workroam" {
            return Ok(format!("read: HTTP {}", r.status));
        }
        let d = c.delete_object(&t.loc.bucket, &probe).await?;
        if !d.ok() {
            return Ok(format!("delete: HTTP {}", d.status));
        }
        Ok(String::new())
    }
    .await;
    match rwd {
        Ok(e) if e.is_empty() => push(
            timed(
                start,
                CheckResult::new("rwd", "storage", CheckStatus::Pass, "ok"),
            ),
            &mut out,
        ),
        Ok(e) => {
            push(
                timed(
                    start,
                    CheckResult::new("rwd", "storage", CheckStatus::Fail, e).hint("hint.rwd"),
                ),
                &mut out,
            );
            return (out, f);
        }
        Err(e) => {
            push(
                timed(
                    start,
                    CheckResult::new("rwd", "storage", CheckStatus::Fail, e.message)
                        .hint("hint.rwd"),
                ),
                &mut out,
            );
            return (out, f);
        }
    }

    // Conditional create
    let start = Instant::now();
    let k = format!("{probe}-c");
    let first = c
        .put_object(&t.loc.bucket, &k, b"1".to_vec(), &[("if-none-match", "*")])
        .await;
    let second = c
        .put_object(&t.loc.bucket, &k, b"2".to_vec(), &[("if-none-match", "*")])
        .await;
    let cc = matches!((&first, &second), (Ok(a), Ok(b)) if a.ok() && (b.status == 412 || b.status == 409));
    f.capabilities.conditional_create = cc;
    push(
        timed(
            start,
            if cc {
                CheckResult::new(
                    "cond_create",
                    "storage",
                    CheckStatus::Pass,
                    "If-None-Match honoured",
                )
            } else {
                CheckResult::new(
                    "cond_create",
                    "storage",
                    CheckStatus::Warn,
                    "If-None-Match is ignored",
                )
                .hint("hint.conditional")
            },
        ),
        &mut out,
    );
    // Conditional overwrite with a stale ETag
    let start = Instant::now();
    let co = match c
        .put_object(
            &t.loc.bucket,
            &k,
            b"3".to_vec(),
            &[("if-match", "\"00000000000000000000000000000000\"")],
        )
        .await
    {
        Ok(r) => r.status == 412,
        Err(_) => false,
    };
    f.capabilities.conditional_overwrite = co;
    let _ = c.delete_object(&t.loc.bucket, &k).await;
    push(
        timed(
            start,
            if co {
                CheckResult::new(
                    "cond_overwrite",
                    "storage",
                    CheckStatus::Pass,
                    "If-Match honoured",
                )
            } else {
                CheckResult::new(
                    "cond_overwrite",
                    "storage",
                    CheckStatus::Warn,
                    "If-Match is ignored",
                )
                .hint("hint.conditional")
            },
        ),
        &mut out,
    );
    // Multipart
    let start = Instant::now();
    let data = vec![7u8; 5 * 1024 * 1024 + 1024];
    let km = format!("{probe}-m");
    let mp = matches!(c.multipart_upload(&t.loc.bucket, &km, &data, 5 * 1024 * 1024).await, Ok(r) if r.ok());
    let _ = c.delete_object(&t.loc.bucket, &km).await;
    f.capabilities.multipart = mp;
    push(
        timed(
            start,
            if mp {
                CheckResult::new("multipart", "storage", CheckStatus::Pass, "ok")
            } else {
                CheckResult::new(
                    "multipart",
                    "storage",
                    CheckStatus::Warn,
                    "multipart upload failed",
                )
                .hint("hint.multipart")
            },
        ),
        &mut out,
    );

    // Key scope
    let start = Instant::now();
    let listed = c
        .list_buckets()
        .await
        .ok()
        .filter(|r| r.ok())
        .map(|r| S3Client::parse_bucket_names(&r))
        .unwrap_or_default();
    let others: Vec<&String> = listed.iter().filter(|b| **b != t.loc.bucket).collect();
    let mut broad = false;
    for b in others.iter().take(3) {
        if let Ok((_, r)) = c.list_objects(b, "", 1).await {
            if r.ok() {
                broad = true;
                break;
            }
        }
    }
    f.broad_key = broad;
    push(
        timed(
            start,
            if broad {
                CheckResult::new(
                    "key_scope",
                    "storage",
                    CheckStatus::Warn,
                    format!("the key can read {} other bucket(s)", others.len()),
                )
                .hint("hint.key_scope")
            } else {
                CheckResult::new(
                    "key_scope",
                    "storage",
                    CheckStatus::Pass,
                    "limited to the workspace",
                )
            },
        ),
        &mut out,
    );
    // Versioning and lifecycle
    let start = Instant::now();
    let v = c.get_versioning(&t.loc.bucket).await.ok().flatten();
    let l = c.get_lifecycle(&t.loc.bucket).await.ok().flatten();
    let has_rules = l.as_deref().map(|x| x.contains("<Rule>")).unwrap_or(false);
    push(
        timed(
            start,
            if has_rules {
                CheckResult::new(
                    "versioning",
                    "storage",
                    CheckStatus::Warn,
                    "the bucket has lifecycle rules",
                )
                .hint("hint.versioning")
            } else {
                CheckResult::new(
                    "versioning",
                    "storage",
                    CheckStatus::Pass,
                    format!("versioning: {}", v.unwrap_or_else(|| "off".into())),
                )
            },
        ),
        &mut out,
    );
    // Latency and bandwidth (8 MB)
    let start = Instant::now();
    let ks = format!("{probe}-s");
    let t0 = Instant::now();
    let _ = c.head_bucket(&t.loc.bucket).await;
    let rtt = t0.elapsed().as_millis();
    let big = vec![1u8; 8 * 1024 * 1024];
    let t1 = Instant::now();
    let up = c
        .put_object(&t.loc.bucket, &ks, big, &[])
        .await
        .map(|r| r.ok())
        .unwrap_or(false);
    let up_s = t1.elapsed().as_secs_f64().max(0.001);
    let t2 = Instant::now();
    let down = c
        .get_object(&t.loc.bucket, &ks)
        .await
        .map(|r| r.ok())
        .unwrap_or(false);
    let down_s = t2.elapsed().as_secs_f64().max(0.001);
    let _ = c.delete_object(&t.loc.bucket, &ks).await;
    let msg = if up && down {
        format!(
            "{rtt} ms · ↑ {:.1} MB/s · ↓ {:.1} MB/s",
            8.0 / up_s,
            8.0 / down_s
        )
    } else {
        format!("{rtt} ms")
    };
    push(
        timed(
            start,
            CheckResult::new("speed", "storage", CheckStatus::Info, msg),
        ),
        &mut out,
    );
    (out, f)
}

fn hex_id() -> String {
    workroam_core::crypto::random_id("p")
}

/// Checks of this computer: git, long paths, agents.
pub fn local_checks() -> Vec<CheckResult> {
    let mut out = Vec::new();
    let start = Instant::now();
    out.push(timed(
        start,
        match workroam_git::cmd::version() {
            Some((a, b, c)) if (a, b) >= (2, 30) => CheckResult::new(
                "git",
                "local",
                CheckStatus::Pass,
                format!("git {a}.{b}.{c}"),
            ),
            Some((a, b, c)) => CheckResult::new(
                "git",
                "local",
                CheckStatus::Fail,
                format!("git {a}.{b}.{c} (needs 2.30+)"),
            )
            .hint("hint.git"),
            None => CheckResult::new("git", "local", CheckStatus::Fail, "git not found")
                .hint("hint.git"),
        },
    ));
    if cfg!(windows) {
        out.push(if workroam_platform::dircheck::long_paths_enabled() {
            CheckResult::new(
                "longpaths",
                "local",
                CheckStatus::Pass,
                "LongPathsEnabled = 1",
            )
        } else {
            CheckResult::new(
                "longpaths",
                "local",
                CheckStatus::Warn,
                "LongPathsEnabled is off",
            )
            .hint("hint.longpaths")
        });
    } else {
        out.push(CheckResult::new(
            "longpaths",
            "local",
            CheckStatus::Skip,
            "not Windows",
        ));
    }
    let start = Instant::now();
    let detected: Vec<workroam_adapters::Detected> = workroam_adapters::registry::local()
        .iter()
        .map(workroam_adapters::detect)
        .collect();
    let installed: Vec<String> = detected
        .iter()
        .filter(|d| d.installed)
        .map(|d| {
            format!(
                "{} {}{}",
                d.name,
                d.version.clone().unwrap_or_default(),
                if d.supported == Some(false) {
                    " (!)"
                } else {
                    ""
                }
            )
        })
        .collect();
    let unsupported = detected.iter().any(|d| d.supported == Some(false));
    let mut r = CheckResult::new(
        "agents",
        "local",
        if unsupported {
            CheckStatus::Warn
        } else {
            CheckStatus::Info
        },
        if installed.is_empty() {
            "none detected".to_string()
        } else {
            installed.join(", ")
        },
    );
    if unsupported {
        r = r.hint("hint.agents");
    }
    out.push(timed(start, r));
    out
}

/// Off-site backup target reachable and recent (when configured).
pub async fn backup_check(
    last_ok: Option<chrono::DateTime<chrono::Utc>>,
    reachable: Result<(), String>,
) -> CheckResult {
    match reachable {
        Err(e) => CheckResult::new("backup", "backup", CheckStatus::Warn, e),
        Ok(()) => match last_ok {
            Some(t) if chrono::Utc::now() - t > chrono::Duration::days(3) => CheckResult::new(
                "backup",
                "backup",
                CheckStatus::Warn,
                format!("last mirror {}", t.format("%Y-%m-%d")),
            ),
            Some(t) => CheckResult::new(
                "backup",
                "backup",
                CheckStatus::Pass,
                format!("last mirror {}", t.format("%Y-%m-%d %H:%M")),
            ),
            None => CheckResult::new("backup", "backup", CheckStatus::Warn, "never mirrored yet"),
        },
    }
}
