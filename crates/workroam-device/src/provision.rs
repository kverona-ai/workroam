//! Creating and revoking scoped access keys through each storage's admin
//! interface (architecture §7.3 `KeyProvisioner`). Storages without one use
//! the manual flow: the user creates the key in the console and pastes it.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use workroam_core::config::WorkspaceLocation;
use workroam_core::model::Access;
use workroam_core::{Code, Error, Result};
use workroam_store::s3raw::{http_client, xml_tag};
use workroam_store::sigv4::{self, SignInput};
use workroam_store::Creds;

/// Administrator credentials, used once and then dropped (never stored).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdminCreds {
    pub access_key: String,
    pub secret_key: String,
    /// Garage: admin API endpoint (e.g. `http://host:3903`).
    #[serde(default)]
    pub admin_endpoint: Option<String>,
    /// Garage: admin bearer token.
    #[serde(default)]
    pub admin_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProvisionedKey {
    pub access_key: String,
    pub secret_key: String,
    pub scope: Access,
    /// Provider handle needed to revoke (IAM user name, Garage key id…).
    pub provider_ref: String,
    pub provider: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyProvisioner {
    AwsIam,
    /// MinIO-compatible admin API (MinIO, RustFS).
    MinioAdmin,
    Garage,
}

/// The provisioner for a storage preset (`None`: manual flow only).
pub fn provisioner_for(preset: &str, endpoint: &str) -> Option<KeyProvisioner> {
    match preset {
        "aws" => Some(KeyProvisioner::AwsIam),
        "rustfs" | "minio" => Some(KeyProvisioner::MinioAdmin),
        "garage" => Some(KeyProvisioner::Garage),
        "other" if endpoint.contains("amazonaws.com") => Some(KeyProvisioner::AwsIam),
        _ => None,
    }
}

/// S3 policy limited to `<bucket>/<prefix>`.
pub fn bucket_policy(loc: &WorkspaceLocation, scope: Access) -> Value {
    let b = &loc.bucket;
    let p = loc.prefix.trim_matches('/');
    let objects = if p.is_empty() {
        format!("arn:aws:s3:::{b}/*")
    } else {
        format!("arn:aws:s3:::{b}/{p}/*")
    };
    let mut list = json!({
        "Effect": "Allow",
        "Action": ["s3:ListBucket", "s3:GetBucketLocation"],
        "Resource": [format!("arn:aws:s3:::{b}")]
    });
    if !p.is_empty() {
        list["Condition"] =
            json!({ "StringLike": { "s3:prefix": [format!("{p}/*"), p.to_string()] } });
    }
    let actions: Vec<&str> = match scope {
        Access::ReadOnly => vec!["s3:GetObject"],
        Access::ReadWrite => vec![
            "s3:GetObject",
            "s3:PutObject",
            "s3:DeleteObject",
            "s3:AbortMultipartUpload",
            "s3:ListMultipartUploadParts",
        ],
    };
    json!({
        "Version": "2012-10-17",
        "Statement": [list, { "Effect": "Allow", "Action": actions, "Resource": [objects] }]
    })
}

/// A SigV4-signed request to any AWS-style endpoint.
pub async fn signed_request(
    method: &str,
    url: &str,
    service: &str,
    region: &str,
    creds: &Creds,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<(u16, Vec<u8>)> {
    let u = url::Url::parse(url).map_err(|e| Error::invalid(format!("invalid URL {url}: {e}")))?;
    let host = match u.port() {
        Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()),
        None => u.host_str().unwrap_or_default().to_string(),
    };
    let query: BTreeMap<String, String> = u
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let h: BTreeMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let payload = sigv4::sha256_hex(&body);
    let path = if u.path().is_empty() {
        "/".to_string()
    } else {
        u.path().to_string()
    };
    let signed = sigv4::sign(&SignInput {
        method,
        host: &host,
        path: &path,
        query: &query,
        headers: &h,
        payload_sha256: &payload,
        region,
        service,
        access_key: &creds.access_key,
        secret_key: &creds.secret_key,
        now: Utc::now(),
    });
    let client = http_client(Duration::from_secs(30));
    let mut req = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
        u.as_str(),
    );
    for (k, v) in h.iter().chain(signed.iter()) {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req
        .body(body)
        .send()
        .await
        .map_err(|e| Error::network(e.to_string()))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::network(e.to_string()))?;
    Ok((status, bytes.to_vec()))
}

fn admin_error(provider: &str, status: u16, body: &[u8]) -> Error {
    let text = String::from_utf8_lossy(body);
    let detail = xml_tag(&text, "Message").unwrap_or_else(|| text.chars().take(300).collect());
    let code = if status == 401 || status == 403 {
        Code::Denied
    } else {
        Code::Network
    };
    Error::new(
        code,
        format!("{provider} admin call failed ({status}): {detail}"),
    )
    .with_hint("check that the administrator credentials are correct and allowed to manage keys")
}

fn key_name(loc: &WorkspaceLocation, scope: Access) -> String {
    let ws: String = loc
        .workspace_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let suffix = hex::encode(workroam_core::crypto::random_bytes::<3>());
    format!(
        "workroam-{ws}-{}-{suffix}",
        if scope == Access::ReadOnly {
            "ro"
        } else {
            "rw"
        }
    )
}

impl KeyProvisioner {
    pub fn id(self) -> &'static str {
        match self {
            KeyProvisioner::AwsIam => "aws-iam",
            KeyProvisioner::MinioAdmin => "minio-admin",
            KeyProvisioner::Garage => "garage",
        }
    }

    /// Whether the admin interface needs a separate endpoint and token.
    pub fn needs_admin_token(self) -> bool {
        self == KeyProvisioner::Garage
    }

    pub async fn create_key(
        self,
        loc: &WorkspaceLocation,
        admin: &AdminCreds,
        scope: Access,
    ) -> Result<ProvisionedKey> {
        let name = key_name(loc, scope);
        let key = match self {
            KeyProvisioner::AwsIam => aws_create(loc, admin, scope, &name).await?,
            KeyProvisioner::MinioAdmin => minio_create(loc, admin, scope, &name).await?,
            KeyProvisioner::Garage => garage_create(loc, admin, scope, &name).await?,
        };
        Ok(key)
    }

    pub async fn revoke_key(
        self,
        loc: &WorkspaceLocation,
        admin: &AdminCreds,
        key: &ProvisionedKey,
    ) -> Result<()> {
        match self {
            KeyProvisioner::AwsIam => aws_revoke(admin, key).await,
            KeyProvisioner::MinioAdmin => minio_revoke(loc, admin, key).await,
            KeyProvisioner::Garage => garage_revoke(admin, key).await,
        }
    }
}

// ---- AWS IAM -------------------------------------------------------------

const IAM: &str = "https://iam.amazonaws.com/";

async fn iam_call(admin: &AdminCreds, params: &[(&str, &str)]) -> Result<String> {
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    body.append_pair("Version", "2010-05-08");
    for (k, v) in params {
        body.append_pair(k, v);
    }
    let creds = Creds {
        access_key: admin.access_key.clone(),
        secret_key: admin.secret_key.clone(),
    };
    let (status, resp) = signed_request(
        "POST",
        IAM,
        "iam",
        "us-east-1",
        &creds,
        &[(
            "content-type",
            "application/x-www-form-urlencoded; charset=utf-8",
        )],
        body.finish().into_bytes(),
    )
    .await?;
    if status >= 300 {
        return Err(admin_error("IAM", status, &resp));
    }
    Ok(String::from_utf8_lossy(&resp).into_owned())
}

async fn aws_create(
    loc: &WorkspaceLocation,
    admin: &AdminCreds,
    scope: Access,
    name: &str,
) -> Result<ProvisionedKey> {
    iam_call(
        admin,
        &[
            ("Action", "CreateUser"),
            ("UserName", name),
            ("Path", "/workroam/"),
        ],
    )
    .await?;
    let policy = bucket_policy(loc, scope).to_string();
    iam_call(
        admin,
        &[
            ("Action", "PutUserPolicy"),
            ("UserName", name),
            ("PolicyName", "workroam"),
            ("PolicyDocument", &policy),
        ],
    )
    .await?;
    let xml = iam_call(admin, &[("Action", "CreateAccessKey"), ("UserName", name)]).await?;
    Ok(ProvisionedKey {
        access_key: xml_tag(&xml, "AccessKeyId")
            .ok_or_else(|| Error::internal("IAM returned no AccessKeyId"))?,
        secret_key: xml_tag(&xml, "SecretAccessKey")
            .ok_or_else(|| Error::internal("IAM returned no SecretAccessKey"))?,
        scope,
        provider_ref: name.to_string(),
        provider: "aws-iam".into(),
    })
}

async fn aws_revoke(admin: &AdminCreds, key: &ProvisionedKey) -> Result<()> {
    let user = key.provider_ref.as_str();
    iam_call(
        admin,
        &[
            ("Action", "DeleteAccessKey"),
            ("UserName", user),
            ("AccessKeyId", &key.access_key),
        ],
    )
    .await?;
    let _ = iam_call(
        admin,
        &[
            ("Action", "DeleteUserPolicy"),
            ("UserName", user),
            ("PolicyName", "workroam"),
        ],
    )
    .await;
    let _ = iam_call(admin, &[("Action", "DeleteUser"), ("UserName", user)]).await;
    Ok(())
}

// ---- MinIO-compatible admin API (MinIO, RustFS) --------------------------

fn admin_base(loc: &WorkspaceLocation) -> String {
    loc.endpoint.trim_end_matches('/').to_string()
}

async fn minio_call(
    loc: &WorkspaceLocation,
    admin: &AdminCreds,
    method: &str,
    path_query: &str,
    body: Vec<u8>,
) -> Result<(u16, Vec<u8>)> {
    let creds = Creds {
        access_key: admin.access_key.clone(),
        secret_key: admin.secret_key.clone(),
    };
    let region = if loc.region.is_empty() {
        "us-east-1"
    } else {
        &loc.region
    };
    let mut last = (404u16, Vec::new());
    for base in ["/minio/admin/v3", "/rustfs/admin/v3"] {
        let url = format!("{}{base}{path_query}", admin_base(loc));
        let r = signed_request(
            method,
            &url,
            "s3",
            region,
            &creds,
            &[("content-type", "application/octet-stream")],
            body.clone(),
        )
        .await?;
        if r.0 != 404 && r.0 != 405 {
            return Ok(r);
        }
        last = r;
    }
    Ok(last)
}

async fn minio_create(
    loc: &WorkspaceLocation,
    admin: &AdminCreds,
    scope: Access,
    name: &str,
) -> Result<ProvisionedKey> {
    let req = json!({
        "policy": bucket_policy(loc, scope),
        "accessKey": "",
        "secretKey": "",
        "name": name.chars().take(32).collect::<String>(),
        "description": "Workroam workspace key",
    });
    let enc = crate::madmin::encrypt(&admin.secret_key, req.to_string().as_bytes())?;
    let (status, body) = minio_call(loc, admin, "PUT", "/add-service-account", enc).await?;
    if status >= 300 {
        return Err(admin_error("Storage", status, &body));
    }
    let plain = crate::madmin::decrypt(&admin.secret_key, &body)
        .or_else(|_| Ok::<_, Error>(body.clone()))?;
    let v: Value =
        serde_json::from_slice(&plain).map_err(|_| Error::internal("unexpected admin response"))?;
    let c = v.get("credentials").unwrap_or(&v);
    let ak = c
        .get("accessKey")
        .and_then(|x| x.as_str())
        .ok_or_else(|| Error::internal("admin response has no accessKey"))?;
    let sk = c
        .get("secretKey")
        .and_then(|x| x.as_str())
        .ok_or_else(|| Error::internal("admin response has no secretKey"))?;
    Ok(ProvisionedKey {
        access_key: ak.into(),
        secret_key: sk.into(),
        scope,
        provider_ref: ak.into(),
        provider: "minio-admin".into(),
    })
}

async fn minio_revoke(
    loc: &WorkspaceLocation,
    admin: &AdminCreds,
    key: &ProvisionedKey,
) -> Result<()> {
    let q = format!(
        "/delete-service-account?accessKey={}",
        sigv4::uri_encode(&key.access_key, false)
    );
    let (status, body) = minio_call(loc, admin, "DELETE", &q, Vec::new()).await?;
    if status >= 300 && status != 404 {
        return Err(admin_error("Storage", status, &body));
    }
    Ok(())
}

// ---- Garage admin API v2 --------------------------------------------------

async fn garage_call(
    admin: &AdminCreds,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let base = admin.admin_endpoint.as_deref().ok_or_else(|| {
        Error::invalid("Garage needs the admin API endpoint (for example http://host:3903)")
    })?;
    let token = admin
        .admin_token
        .as_deref()
        .ok_or_else(|| Error::invalid("Garage needs the admin token"))?;
    let client = http_client(Duration::from_secs(30));
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let mut req = client
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
            url,
        )
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| Error::network(e.to_string()))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::network(e.to_string()))?;
    if status >= 300 {
        return Err(admin_error("Garage", status, &bytes));
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn garage_create(
    loc: &WorkspaceLocation,
    admin: &AdminCreds,
    scope: Access,
    name: &str,
) -> Result<ProvisionedKey> {
    let key = garage_call(
        admin,
        "POST",
        "/v2/CreateKey",
        Some(json!({ "name": name })),
    )
    .await?;
    let ak = key["accessKeyId"]
        .as_str()
        .ok_or_else(|| Error::internal("Garage returned no accessKeyId"))?
        .to_string();
    let sk = key["secretAccessKey"]
        .as_str()
        .ok_or_else(|| Error::internal("Garage returned no secretAccessKey"))?
        .to_string();
    let bucket = garage_call(
        admin,
        "GET",
        &format!(
            "/v2/GetBucketInfo?globalAlias={}",
            sigv4::uri_encode(&loc.bucket, false)
        ),
        None,
    )
    .await?;
    let bid = bucket["id"]
        .as_str()
        .ok_or_else(|| Error::internal("Garage returned no bucket id"))?;
    garage_call(
        admin,
        "POST",
        "/v2/AllowBucketKey",
        Some(json!({ "bucketId": bid, "accessKeyId": ak, "permissions": { "read": true, "write": scope == Access::ReadWrite, "owner": false } })),
    )
    .await?;
    Ok(ProvisionedKey {
        access_key: ak.clone(),
        secret_key: sk,
        scope,
        provider_ref: ak,
        provider: "garage".into(),
    })
}

async fn garage_revoke(admin: &AdminCreds, key: &ProvisionedKey) -> Result<()> {
    garage_call(
        admin,
        "POST",
        &format!(
            "/v2/DeleteKey?id={}",
            sigv4::uri_encode(&key.provider_ref, false)
        ),
        None,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_scopes_prefix() {
        let loc = WorkspaceLocation {
            bucket: "b".into(),
            prefix: "team/ws".into(),
            ..Default::default()
        };
        let p = bucket_policy(&loc, Access::ReadOnly);
        assert_eq!(p["Statement"][1]["Resource"][0], "arn:aws:s3:::b/team/ws/*");
        assert_eq!(p["Statement"][1]["Action"].as_array().unwrap().len(), 1);
        assert_eq!(
            p["Statement"][0]["Condition"]["StringLike"]["s3:prefix"][0],
            "team/ws/*"
        );
        assert_eq!(
            provisioner_for("rustfs", ""),
            Some(KeyProvisioner::MinioAdmin)
        );
        assert_eq!(provisioner_for("r2", ""), None);
    }
}
