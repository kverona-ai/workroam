# Deployment

**English** · [简体中文](deployment.zh-CN.md)

All of Workroam's logic runs in the client. The server side is just **one S3-compatible bucket** that you own, holding only end-to-end encrypted data. This guide covers every way to provide that storage, the optional `workroam-server`, and how to run them day to day. To install the desktop app or the CLI, see [Install](install.md). Every setting is listed in [Configuration](configuration.md).

## Contents

1. [Choose a deployment](#1-choose-a-deployment)
2. [Use a cloud storage service](#2-use-a-cloud-storage-service)
3. [Self-host with one command](#3-self-host-with-one-command)
4. [Kubernetes (Helm)](#4-kubernetes-helm)
5. [Cloud agent relay (workroam-server relay mode)](#5-cloud-agent-relay-workroam-server-relay-mode)
6. [Full-mode server](#6-full-mode-server)
7. [No public IP: Cloudflare Tunnel](#7-no-public-ip-cloudflare-tunnel)
8. [Git transfer server: Gitea](#8-git-transfer-server-gitea)
9. [Local folder or NAS](#9-local-folder-or-nas)
10. [Set up your computers](#10-set-up-your-computers)
11. [Operations](#11-operations)
12. [Troubleshooting](#12-troubleshooting)

## 1. Choose a deployment

| Your situation | What to deploy | Section |
| --- | --- | --- |
| You already use AWS S3, Cloudflare R2, Alibaba Cloud OSS or another S3 service | **Nothing.** Create a bucket and pick the preset in the setup wizard | [§2](#2-use-a-cloud-storage-service) |
| You have a Linux server with a public IP and a domain | `install.sh`: storage + Caddy (automatic HTTPS) in Docker | [§3](#3-self-host-with-one-command) |
| You run Kubernetes | The Helm chart in `deploy/helm/workroam` | [§4](#4-kubernetes-helm) |
| You want a cloud personal agent (Grok Bot, Muse) to reach your computers | `workroam-server` in relay mode, on the bot's cloud computer or any VPS | [§5](#5-cloud-agent-relay-workroam-server-relay-mode) |
| You want 6-digit pairing codes, phone notifications or the web reader | `install.sh --with-server` (full mode) | [§6](#6-full-mode-server) |
| Your server has no public IP | `install.sh --tunnel` (Cloudflare Tunnel) | [§7](#7-no-public-ip-cloudflare-tunnel) |
| Every computer can see the same NAS or shared folder | Nothing; use the "Local folder" preset | [§9](#9-local-folder-or-nas) |

What the storage needs:

- the S3 API with SigV4 signing (path-style or virtual-hosted, detected during setup);
- **conditional writes** (`If-None-Match` / `If-Match`) for the project lease, which guarantees that only one computer writes at a time. AWS S3, R2 and RustFS support them. Without them (for example Garage) the lease shows as "not supported" and syncing still works, but nothing stops two computers from pushing at once;
- multipart upload for large packs.

The setup wizard runs 16 capability checks (`workroam doctor` runs them again any time), so you find out right away if something is missing.

**Security model.** Data is encrypted on the client before upload, and the storage, Caddy and `workroam-server` only ever see ciphertext. The encryption password and the access keys live in each computer's system keychain, never in the bucket and never on the server. Each computer gets its own key, limited to the workspace bucket, and removing a device rotates it.

## 2. Use a cloud storage service

1. Create an empty, **private** bucket (any name; versioning is not required).
2. Create an access key for Workroam. Either:
   - **let Workroam create a scoped key** (AWS S3 and RustFS): enter an admin / IAM key once and choose "Create a dedicated key for Workroam". The wizard creates a key that can only reach this bucket and drops the admin key; it is never saved;
   - **or create the key yourself** (R2, OSS, others): give it read and write access to this bucket only, then choose "Use this key".
3. In the setup wizard (or `workroam setup`), pick the preset and fill in the endpoint:

| Preset | `--preset` | Endpoint | Region | Workroam can create the key |
| --- | --- | --- | --- | --- |
| AWS S3 | `aws` | `https://s3.<region>.amazonaws.com` | the bucket's region, e.g. `us-east-1` | Yes (IAM) |
| Cloudflare R2 | `r2` | `https://<account-id>.r2.cloudflarestorage.com` | `auto` | No: create an R2 API token with "Object Read & Write" on this bucket |
| Alibaba Cloud OSS | `oss` | `https://oss-<region>.aliyuncs.com`, e.g. `https://oss-cn-hangzhou.aliyuncs.com` | e.g. `cn-hangzhou` | No: create a RAM user with the policy below |
| Other S3-compatible (MinIO, Ceph, Wasabi, …) | `other` | your endpoint | your region (default `us-east-1`) | No |

Command-line equivalent on a headless computer:

```sh
# The secret key is read from stdin (or WORKROAM_SECRET_KEY), never from the command line
workroam setup --new --preset r2 \
  --endpoint https://<account-id>.r2.cloudflarestorage.com \
  --access-key <access key id> --secret-key-stdin --key-mode current --bucket workroam
```

A minimal policy for a key you create yourself (AWS IAM syntax; OSS RAM policies use the same actions with the `oss:` prefix and `acs:oss:*:*:<bucket>` resources):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": ["s3:ListBucket", "s3:GetBucketLocation"], "Resource": ["arn:aws:s3:::<bucket>"] },
    { "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
      "Resource": ["arn:aws:s3:::<bucket>/*"] }
  ]
}
```

A read-only key (for read-only devices and the web reader) only needs `s3:ListBucket`, `s3:GetBucketLocation` and `s3:GetObject`. If the key can see more than the workspace bucket, the wizard warns you; `--accept-broad-key` keeps it anyway.

## 3. Self-host with one command

`install.sh` deploys an S3 storage (RustFS by default, or Garage / SeaweedFS) behind Caddy on a single domain and port 443, with automatic HTTPS from Let's Encrypt.

**Requirements**

- a Linux server (x86_64 or arm64) with a public IP; 1 vCPU and 1 GB of RAM are enough, disk according to your projects;
- Docker Engine with the Compose v2 plugin (`docker compose version` works);
- a domain such as `s3.example.com` whose A/AAAA record points to the server;
- ports 80 and 443 open (80 is only used for the certificate challenge and the redirect to HTTPS).

**Run**

```sh
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install.sh \
  | sh -s -- --domain s3.example.com
```

The copy attached to a Release fetches its Compose files from that release's tag, so the result is reproducible. The script:

1. writes a Compose project to `/opt/workroam` (or `~/workroam` when not run as root);
2. generates the storage admin key and saves it in `.env` there (mode 0600);
3. starts the storage and Caddy; Caddy gets the HTTPS certificate;
4. prints exactly what to enter in the setup wizard, including the equivalent `workroam setup` command.

| Option | Description |
| --- | --- |
| `--domain <domain>` | Required. The host name that points to this server (not a URL) |
| `--storage rustfs\|garage\|seaweedfs` | The storage, default `rustfs` |
| `--with-server` | Also run `workroam-server` in full mode, see [§6](#6-full-mode-server) |
| `--with-gitea` | Also run Gitea at `https://<domain>/git/`, see [§8](#8-git-transfer-server-gitea) |
| `--tunnel <token>` | No public IP: carry the traffic over a Cloudflare Tunnel, see [§7](#7-no-public-ip-cloudflare-tunnel) |
| `--bucket <name>` | The bucket created for Garage / SeaweedFS, default `workroam` |
| `--ntfy <url>` | ntfy topic for phone notifications (with `--with-server`) |
| `--dir <path>` | Deployment directory |
| `--ref <git ref>` | Version of the deploy files to use (default: the release tag for a Release copy, `main` otherwise) |
| `--no-start` | Only write the files; start nothing |

Before running, you can replace any image with an environment variable: `RUSTFS_IMAGE`, `GARAGE_IMAGE`, `SEAWEEDFS_IMAGE`, `CADDY_IMAGE`, `WORKROAM_SERVER_IMAGE`, `GITEA_IMAGE`, `CLOUDFLARED_IMAGE`. For example, pin versions for production: `RUSTFS_IMAGE=rustfs/rustfs:<version> WORKROAM_SERVER_IMAGE=ghcr.io/kverona-ai/workroam-server:0.1.0`.

**What to choose in the wizard**

| Storage | Choose | Key |
| --- | --- | --- |
| RustFS (recommended) | "Self-hosted (install.sh)", endpoint `https://<domain>` | Enter the admin key the script printed and choose "Create a dedicated key for Workroam": the wizard creates the bucket and a key that can only reach it; the admin key is used once and not saved on the computer |
| Garage | "Other S3-compatible", endpoint `https://<domain>`, region `us-east-1` | The script already created the bucket and a key limited to it: enter it, choose "Use this key", bucket `workroam` |
| SeaweedFS | Same as Garage | Same as Garage |

```sh
# RustFS, command-line equivalent (the script prints the full command)
workroam setup --new --preset rustfs --endpoint https://s3.example.com \
  --access-key <admin access key> --secret-key-stdin --key-mode create
```

Garage does not support conditional writes, so the project lease is unavailable there; pick RustFS when you want lease protection.

**Routing on one domain.** All traffic goes to `https://<domain>:443`, and Caddy routes each request:

- requests signed for S3 (`Authorization: AWS4-HMAC-SHA256 …` or a presigned URL) always go to the storage, so a bucket can have any name;
- unsigned `/healthz`, `/relay`, `/mcp`, `/.well-known/oauth-*`, `/oauth/*`, `/pair`, `/devices/*` and `/notify` go to `workroam-server`, and `/web/` is the web reader (with `--with-server`);
- `/git/` goes to Gitea (with `--with-gitea`).

The web reader shares an origin with the storage, so **the bucket needs no CORS** in this setup.

## 4. Kubernetes (Helm)

The chart deploys RustFS (StatefulSet + PVC), the optional `workroam-server`, and one Ingress for the host. It is shipped in the repository and in every source package:

```sh
git clone --depth 1 --branch v0.1.0 https://github.com/kverona-ai/workroam && cd workroam
helm install workroam deploy/helm/workroam -n workroam --create-namespace \
  --set host=s3.example.com \
  --set ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt \
  --set server.enabled=true
helm status workroam -n workroam   # prints how to read the admin key
```

| Value | Default | Description |
| --- | --- | --- |
| `host` | `s3.example.com` | Public host; devices use `https://<host>` as the storage endpoint |
| `storage.image.repository` / `tag` | `rustfs/rustfs` / `latest` | Storage image; pin the tag in production |
| `storage.accessKey` / `secretKey` | empty | Admin key. Empty: generated once and kept in the Secret across upgrades |
| `storage.existingSecret` | empty | Use your own Secret with `access-key` / `secret-key` |
| `storage.persistence.size` / `storageClass` | `100Gi` / default | Data volume |
| `server.enabled` | `false` | Run `workroam-server` in full mode |
| `server.image.tag` | the chart's `appVersion` | Server image tag |
| `server.ntfy` | empty | ntfy topic for phone notifications |
| `server.webHost` | empty | Separate host for the web reader (an Ingress cannot strip a path prefix portably) |
| `server.persistence.size` | `1Gi` | Server state volume |
| `ingress.enabled` / `className` / `annotations` | `true` / empty / empty | Ingress settings |
| `ingress.tls.enabled` / `secretName` | `true` / `workroam-tls` | TLS for the host |

With ingress-nginx, add `nginx.ingress.kubernetes.io/proxy-body-size: "0"` so large packs are not rejected. When `server.webHost` is set, the reader and the storage are on different origins, so configure [CORS](#cors-for-the-web-reader).

## 5. Cloud agent relay (workroam-server relay mode)

A cloud personal agent (Grok Bot, Muse) cannot reach the computers on your desk. `workroam-server` in **relay mode** sits in between: your computers keep an outbound connection to it, and the cloud agent talks to it as a remote MCP connector. Relay mode holds no storage credentials and no encryption keys. It needs no Docker, database or root, and is usually run on the bot's own cloud computer.

**The usual flow (about two minutes)**

1. On your computer, connect the cloud agent from the desktop app's agent list, or run `workroam agent connect <agent>`. It prints deployment instructions with a one-time **bind code** (`wrb1:…`, valid for 30 minutes) and gives you the Workroam Skill for the bot.
2. Paste the instructions to the bot. It runs:

   ```sh
   curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh
   workroam-server --bind '<bind code>' --tunnel
   ```

   `--tunnel` downloads `cloudflared` when it is missing and opens an outbound quick tunnel, then prints a public `https://…trycloudflare.com` address.
3. The bot reports the address back. Paste it into the desktop card, or run `workroam agent connect <agent> --relay <address>`. Your computer signs the bind challenge, and the bind code is spent.
4. Approve the connection card on your computer and choose what the agent may do (read-only or sync) and which projects it may see.

**On your own VPS**, with a fixed address behind your reverse proxy:

```sh
workroam-server --listen 127.0.0.1:8787 --public-url https://relay.example.com --bind '<bind code>'
```

A systemd unit keeps it running:

```ini
# /etc/systemd/system/workroam-server.service
[Unit]
Description=Workroam relay
After=network-online.target
Wants=network-online.target

[Service]
User=workroam
ExecStart=/usr/local/bin/workroam-server --listen 127.0.0.1:8787 --public-url https://relay.example.com
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Run the `--bind` step once by hand as the same user; the binding is kept in the state file (`~/.workroam-server/state.json`). `workroam-server --status` prints the binding, the devices and the agent connections.

Or with the container image:

```sh
docker run -d --name workroam-server --restart unless-stopped -p 8787:8787 \
  -v workroam-server:/home/nonroot/.workroam-server \
  ghcr.io/kverona-ai/workroam-server:0.1.0 --listen 0.0.0.0:8787 --public-url https://relay.example.com
```

Notes:

- a quick tunnel gets a new address after each restart. For a fixed address, create a named tunnel and set `CLOUDFLARED_TOKEN`, with its hostname as `--public-url`;
- other computers join a bound relay from the desktop ("Join this relay"); an already-bound computer confirms on a card;
- one relay can serve several cloud agents, and separate relays are independent, so revoking one does not affect the others;
- the address must be HTTPS, or cloud agents cannot sign in.

## 6. Full-mode server

`install.sh --with-server` (or `server.enabled=true` in Helm) runs `workroam-server --full` next to the storage. On top of relay mode it adds:

- **6-digit pairing codes**: new computers join with `workroam setup --join --server https://<domain>` and a short code, and each one gets its **own** storage key, created with the admin key that only the server container holds;
- **phone notifications**: done, failed and arrival events are forwarded to an ntfy topic (`--ntfy`);
- **web reader** at `https://<domain>/web/`: browse and download project files in the browser. Decryption runs in the browser (WASM) with your password, using a read-only key; the server never sees plaintext.

Bind the server to one of your computers once:

```sh
workroam agent connect <agent>                    # on your computer: prints a bind code wrb1:…
cd /opt/workroam
docker compose run --rm workroam-server --bind '<bind code>' --status
docker compose restart workroam-server
workroam agent connect <agent> --relay https://s3.example.com
```

The admin key reaches the server only through environment variables (`WORKROAM_ADMIN_ACCESS_KEY` / `WORKROAM_ADMIN_SECRET_KEY`, written by `install.sh` from `.env`), never through a flag or the state file. The server never holds the encryption password.

### CORS for the web reader

When the reader and the storage are on different origins (cloud storage such as S3 / R2 / OSS, or Helm with `server.webHost`), allow the reader's origin to read the bucket:

```json
[
  {
    "AllowedOrigins": ["https://web.example.com"],
    "AllowedMethods": ["GET", "HEAD"],
    "AllowedHeaders": ["authorization", "x-amz-date", "x-amz-content-sha256", "range"],
    "ExposeHeaders": ["etag", "content-length", "content-range"],
    "MaxAgeSeconds": 3600
  }
]
```

- AWS S3: `aws s3api put-bucket-cors --bucket <bucket> --cors-configuration '{"CORSRules": <the array above>}'`;
- Cloudflare R2: bucket Settings → CORS Policy, paste the array;
- Alibaba Cloud OSS: bucket → Data Security → CORS, with the same origins, methods and headers.

The reader only uses a read-only key (the pairing code from `workroam device add --read-only`), which cannot write or delete.

## 7. No public IP: Cloudflare Tunnel

1. In Cloudflare Zero Trust → Networks → Tunnels, create a tunnel and copy its token.
2. Run `install.sh --domain s3.example.com --tunnel <token>`. No inbound port is opened, and Caddy serves plain HTTP inside the container network only.
3. In the tunnel's Public Hostname settings, add `s3.example.com` → `http://caddy:80`.

The Cloudflare free plan limits a request body to 100 MB. If your repositories are large and packs may exceed that, use a server with a public IP instead.

## 8. Git transfer server: Gitea

A repository in `remote` mode syncs through a git server that every computer can reach. `--with-gitea` runs Gitea at `https://<domain>/git/` (SQLite, registration and SSH off, HTTPS clones only). The script creates the admin user `workroam` and prints its password. Then set the repository's mode to `remote` and its `via` remote to that server (see [Configuration · workroam.toml](configuration.md#workroamtoml-per-project)).

## 9. Local folder or NAS

When every computer mounts the same folder (a NAS share, an external disk you carry, a synced drive), no S3 service is needed: choose "Local folder" in the wizard, or

```sh
workroam setup --new --preset local --endpoint file:///mnt/nas/workroam
```

The endpoint is `file://` followed by the absolute path of the folder. The lease relies on the file system, so prefer a real network share over a folder that another tool syncs in the background.

## 10. Set up your computers

**The first computer** creates the workspace:

1. Install the desktop app ([Install](install.md)) and open it; or run `workroam setup` in a terminal.
2. "Create a workspace" → connect the storage (above) → the 16 checks run.
3. Set the **encryption password** and save the **recovery key**. Without both, nobody can decrypt the data, including you.
4. Name this computer, choose the default project folder, and list the internal hosts it reaches (for example `git.corp.internal`).
5. Add a project: `cd ~/projects/proj-a && workroam init` (or "Add project" in the app).

**Every other computer** joins:

1. On a computer already in the workspace: Devices → "Add a device", or `workroam device add` (`--read-only` for a read-only device, `--qr` to show a QR code). It prints a pairing code.
2. On the new computer: "Join an existing workspace", enter the pairing code and the encryption password. In a terminal, `workroam setup --join` asks for both; in a script, pipe the code into `workroam setup --join --pairing-code-stdin` and pass the password in `WORKROAM_PASSWORD`. With a full-mode server, enter the 6-digit code and the server address (`--server https://<domain>`).
3. Pick the projects to pull and their folders (`--all-projects` takes all of them at their default folders).

Headless and scripted setups use the same flags with secrets from stdin or the environment (`WORKROAM_SECRET_KEY`, `WORKROAM_PASSWORD`); without a terminal, `--recovery-key-out <file>` is required. Run `workroam doctor` at any time to repeat the checks.

## 11. Operations

**Self-hosted storage (install.sh)**

```sh
cd /opt/workroam
docker compose ps                              # status
docker compose logs -f storage                 # storage logs (also: caddy, workroam-server, gitea)
docker compose pull && docker compose up -d    # update the images
docker compose down                            # stop (data stays in the Docker volumes)
```

To move to the deploy files of a newer release, run that release's `install.sh` again with the same `--dir`; the keys in `.env` are kept.

**Backups.** The data lives in the Docker volume `workroam_storage-data` (Garage also has `workroam_storage-meta`). Everything is end-to-end encrypted, so you can back up the volumes anywhere, or turn on Workroam's off-site backup (`workroam backup add`, Settings → Off-site backup), which mirrors the workspace to a second bucket or folder. Losing `.env` only loses the admin key, not the ability to decrypt. `workroam verify` checks the stored data, and `workroam verify --backup` checks the mirror.

**Retention.** `workroam prune` applies the workspace retention policy and frees unused data: by default every snapshot of the last 30 days is kept, plus the newest snapshot of each week for the 52 weeks before that (see [Configuration](configuration.md#settings-reference)). Archived projects are kept until you delete them.

**Clients.** The desktop app and CLI are upgraded by installing the new release; the install scripts and package managers replace the binaries in place. `workroamd` runs as a per-user service: a LaunchAgent `dev.workroam.workroamd` on macOS, the systemd user unit `workroamd.service` on Linux, and the scheduled task `Workroam\workroamd` on Windows. Its logs are in `~/.workroam/logs/`.

**Removing a device.** `workroam device remove <device>` (Devices → Remove) unregisters the computer and rotates the storage key, so the removed computer can no longer read or write.

**Uninstall.** For a self-hosted stack, `docker compose down -v` in the deployment directory deletes the containers **and the data**. On a computer, `workroam config set autostart false` removes the service; then delete the app and `~/.workroam`. Project folders are never touched.

## 12. Troubleshooting

| Symptom | What to check |
| --- | --- |
| Caddy cannot get a certificate | The domain must resolve to this server and ports 80 and 443 must be reachable from the internet; see `docker compose logs caddy` |
| The wizard fails at "conditional writes" | The storage does not support `If-None-Match` / `If-Match`. Syncing still works without the lease; use RustFS, AWS S3 or R2 for lease protection |
| `SignatureDoesNotMatch` or access denied | Check the region and the addressing (`--addressing path` or `virtual-host`); for AWS and OSS the region must be the bucket's region |
| Large pushes fail behind a proxy | Raise the proxy's body size limit (`proxy-body-size: "0"` on ingress-nginx); a Cloudflare Tunnel on the free plan is limited to 100 MB per request |
| A cloud agent cannot sign in to the relay | The relay address must be HTTPS. A quick tunnel changes address on restart: tell the bot to report the new address, or use a named tunnel |
| The web reader shows CORS errors | Configure [CORS](#cors-for-the-web-reader) for the reader's origin |
| The desktop app or CLI cannot reach `workroamd` | Run `workroam doctor`; check `~/.workroam/logs/workroamd.log`; restart the service or run `workroamd` in a terminal |

**Licenses of the bundled storages.** RustFS and SeaweedFS are Apache-2.0. Garage is AGPL-3.0 and runs only as a separate container, which is aggregation and does not affect Workroam's Apache-2.0 license.
