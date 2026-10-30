//! A small S3 client for operations outside OpenDAL's scope: listing and
//! creating buckets, reading versioning / lifecycle settings, and the probe
//! requests behind the capability checks (architecture §7.5).

use crate::sigv4::{self, uri_encode, SignInput};
use crate::Creds;
use chrono::Utc;
use std::collections::BTreeMap;
use std::time::Duration;
use workroam_core::config::Addressing;
use workroam_core::{Error, Result};

#[derive(Debug, Clone)]
pub struct S3Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl S3Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }
    pub fn error_code(&self) -> Option<String> {
        xml_tag(&self.text(), "Code")
    }
}

#[derive(Clone)]
pub struct S3Client {
    pub endpoint: url::Url,
    pub region: String,
    pub addressing: Addressing,
    pub creds: Creds,
    http: reqwest::Client,
}

impl std::fmt::Debug for S3Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Client")
            .field("endpoint", &self.endpoint.as_str())
            .field("region", &self.region)
            .finish()
    }
}

pub fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(format!("workroam/{}", workroam_core::VERSION))
        .build()
        .expect("HTTP client")
}

impl S3Client {
    pub fn new(
        endpoint: &str,
        region: &str,
        addressing: Addressing,
        creds: Creds,
    ) -> Result<S3Client> {
        let endpoint = url::Url::parse(endpoint)
            .map_err(|e| Error::invalid(format!("invalid endpoint {endpoint}: {e}")))?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(Error::invalid(
                "endpoint must start with https:// (or http:// for local testing)",
            ));
        }
        let region = if region.is_empty() {
            "us-east-1".to_string()
        } else {
            region.to_string()
        };
        Ok(S3Client {
            endpoint,
            region,
            addressing,
            creds,
            http: http_client(Duration::from_secs(60)),
        })
    }

    pub fn with_addressing(&self, addressing: Addressing) -> S3Client {
        S3Client {
            addressing,
            ..self.clone()
        }
    }

    fn host_and_path(&self, bucket: Option<&str>, key: &str) -> (String, String, String) {
        let host = self.endpoint.host_str().unwrap_or("").to_string();
        let port = self
            .endpoint
            .port()
            .map(|p| format!(":{p}"))
            .unwrap_or_default();
        let base_path = self.endpoint.path().trim_end_matches('/').to_string();
        let key_enc = uri_encode(key, true);
        match (bucket, self.addressing) {
            (Some(b), Addressing::VirtualHost) => {
                let h = format!("{b}.{host}{port}");
                let p = format!("{base_path}/{key_enc}");
                (
                    h.clone(),
                    p.clone(),
                    format!("{}://{h}{p}", self.endpoint.scheme()),
                )
            }
            (Some(b), Addressing::Path) => {
                let h = format!("{host}{port}");
                let p = if key.is_empty() {
                    format!("{base_path}/{b}")
                } else {
                    format!("{base_path}/{b}/{key_enc}")
                };
                (
                    h.clone(),
                    p.clone(),
                    format!("{}://{h}{p}", self.endpoint.scheme()),
                )
            }
            (None, _) => {
                let h = format!("{host}{port}");
                let p = if base_path.is_empty() {
                    "/".to_string()
                } else {
                    base_path.clone()
                };
                (
                    h.clone(),
                    p.clone(),
                    format!("{}://{h}{p}", self.endpoint.scheme()),
                )
            }
        }
    }

    pub async fn request(
        &self,
        method: &str,
        bucket: Option<&str>,
        key: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<S3Response> {
        let (host, path, url) = self.host_and_path(bucket, key);
        let path = if path.is_empty() {
            "/".to_string()
        } else {
            path
        };
        let q: BTreeMap<String, String> = query
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let h: BTreeMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let payload = sigv4::sha256_hex(&body);
        let signed = sigv4::sign(&SignInput {
            method,
            host: &host,
            path: &path,
            query: &q,
            headers: &h,
            payload_sha256: &payload,
            region: &self.region,
            service: "s3",
            access_key: &self.creds.access_key,
            secret_key: &self.creds.secret_key,
            now: Utc::now(),
        });
        let qs = q
            .iter()
            .map(|(k, v)| {
                if v.is_empty()
                    && matches!(
                        k.as_str(),
                        "versioning" | "lifecycle" | "uploads" | "location"
                    )
                {
                    uri_encode(k, false)
                } else {
                    format!("{}={}", uri_encode(k, false), uri_encode(v, false))
                }
            })
            .collect::<Vec<_>>()
            .join("&");
        let full = if qs.is_empty() {
            url
        } else {
            format!("{url}?{qs}")
        };
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|e| Error::internal(e.to_string()))?;
        let mut rb = self.http.request(m, &full);
        for (k, v) in h.iter().chain(signed.iter()) {
            rb = rb.header(k.as_str(), v.as_str());
        }
        let resp = rb
            .body(body)
            .send()
            .await
            .map_err(|e| Error::network(format!("{method} {full}: {e}")))?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_lowercase(),
                    v.to_str().unwrap_or("").to_string(),
                )
            })
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| Error::network(e.to_string()))?
            .to_vec();
        Ok(S3Response {
            status,
            headers,
            body,
        })
    }

    pub async fn list_buckets(&self) -> Result<S3Response> {
        self.request("GET", None, "", &[], &[], vec![]).await
    }

    pub fn parse_bucket_names(resp: &S3Response) -> Vec<String> {
        xml_all(&resp.text(), "Bucket")
            .iter()
            .filter_map(|b| xml_tag(b, "Name"))
            .collect()
    }

    pub async fn head_bucket(&self, bucket: &str) -> Result<S3Response> {
        self.request("HEAD", Some(bucket), "", &[], &[], vec![])
            .await
    }

    pub async fn create_bucket(&self, bucket: &str) -> Result<S3Response> {
        let body = if self.region != "us-east-1" && self.region != "auto" && !self.region.is_empty()
        {
            format!("<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>", self.region).into_bytes()
        } else {
            vec![]
        };
        self.request("PUT", Some(bucket), "", &[], &[], body).await
    }

    pub async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        max: usize,
    ) -> Result<(Vec<String>, S3Response)> {
        let max = max.to_string();
        let resp = self
            .request(
                "GET",
                Some(bucket),
                "",
                &[("list-type", "2"), ("max-keys", &max), ("prefix", prefix)],
                &[],
                vec![],
            )
            .await?;
        let keys = xml_all(&resp.text(), "Contents")
            .iter()
            .filter_map(|c| xml_tag(c, "Key"))
            .collect();
        Ok((keys, resp))
    }

    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        headers: &[(&str, &str)],
    ) -> Result<S3Response> {
        self.request("PUT", Some(bucket), key, &[], headers, body)
            .await
    }

    pub async fn get_object(&self, bucket: &str, key: &str) -> Result<S3Response> {
        self.request("GET", Some(bucket), key, &[], &[], vec![])
            .await
    }

    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<S3Response> {
        self.request("DELETE", Some(bucket), key, &[], &[], vec![])
            .await
    }

    pub async fn get_versioning(&self, bucket: &str) -> Result<Option<String>> {
        let r = self
            .request("GET", Some(bucket), "", &[("versioning", "")], &[], vec![])
            .await?;
        if !r.ok() {
            return Ok(None);
        }
        Ok(xml_tag(&r.text(), "Status"))
    }

    /// Lifecycle rules XML, `None` when there is no configuration.
    pub async fn get_lifecycle(&self, bucket: &str) -> Result<Option<String>> {
        let r = self
            .request("GET", Some(bucket), "", &[("lifecycle", "")], &[], vec![])
            .await?;
        if r.ok() {
            Ok(Some(r.text()))
        } else {
            Ok(None)
        }
    }

    /// Multipart upload of `data` split into `part` sized pieces.
    pub async fn multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        data: &[u8],
        part: usize,
    ) -> Result<S3Response> {
        let init = self
            .request("POST", Some(bucket), key, &[("uploads", "")], &[], vec![])
            .await?;
        if !init.ok() {
            return Ok(init);
        }
        let upload_id = xml_tag(&init.text(), "UploadId")
            .ok_or_else(|| Error::network("multipart: no UploadId"))?;
        let mut parts = Vec::new();
        for (i, chunk) in data.chunks(part).enumerate() {
            let n = (i + 1).to_string();
            let r = self
                .request(
                    "PUT",
                    Some(bucket),
                    key,
                    &[("partNumber", &n), ("uploadId", &upload_id)],
                    &[],
                    chunk.to_vec(),
                )
                .await?;
            if !r.ok() {
                let _ = self
                    .request(
                        "DELETE",
                        Some(bucket),
                        key,
                        &[("uploadId", &upload_id)],
                        &[],
                        vec![],
                    )
                    .await;
                return Ok(r);
            }
            parts.push((n, r.header("etag").unwrap_or("").to_string()));
        }
        let body = format!(
            "<CompleteMultipartUpload>{}</CompleteMultipartUpload>",
            parts
                .iter()
                .map(|(n, e)| format!("<Part><PartNumber>{n}</PartNumber><ETag>{e}</ETag></Part>"))
                .collect::<String>()
        );
        self.request(
            "POST",
            Some(bucket),
            key,
            &[("uploadId", &upload_id)],
            &[],
            body.into_bytes(),
        )
        .await
    }
}

/// Text of the first `<tag>…</tag>`.
pub fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml_unescape(&xml[start..end]))
}

/// Inner text of every `<tag>…</tag>`.
pub fn xml_all(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(s) = rest.find(&open) {
        let after = &rest[s + open.len()..];
        match after.find(&close) {
            Some(e) => {
                out.push(after[..e].to_string());
                rest = &after[e + close.len()..];
            }
            None => break,
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bucket_list() {
        let xml = "<ListAllMyBucketsResult><Buckets><Bucket><Name>a</Name></Bucket><Bucket><Name>b&amp;c</Name></Bucket></Buckets></ListAllMyBucketsResult>";
        let r = S3Response {
            status: 200,
            headers: BTreeMap::new(),
            body: xml.as_bytes().to_vec(),
        };
        assert_eq!(S3Client::parse_bucket_names(&r), vec!["a", "b&c"]);
    }

    #[test]
    fn urls() {
        let c = S3Client::new(
            "https://s3.example.com",
            "",
            Addressing::Path,
            Creds {
                access_key: "a".into(),
                secret_key: "b".into(),
            },
        )
        .unwrap();
        assert_eq!(
            c.host_and_path(Some("bk"), "x/y z").2,
            "https://s3.example.com/bk/x/y%20z"
        );
        let v = c.with_addressing(Addressing::VirtualHost);
        assert_eq!(
            v.host_and_path(Some("bk"), "k").2,
            "https://bk.s3.example.com/k"
        );
        let p = S3Client::new(
            "http://127.0.0.1:9000",
            "",
            Addressing::Path,
            Creds {
                access_key: "a".into(),
                secret_key: "b".into(),
            },
        )
        .unwrap();
        assert_eq!(
            p.host_and_path(Some("bk"), "").2,
            "http://127.0.0.1:9000/bk"
        );
    }
}
