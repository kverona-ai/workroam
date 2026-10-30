# 部署指南

[English](deployment.md) · **简体中文**

Workroam 的全部逻辑都在客户端里，服务器只需要**一个属于你自己的 S3 兼容存储桶**，里面只有端到端加密后的数据。本文介绍提供这个存储的所有方式、可选的 `workroam-server`，以及日常运维。桌面应用和命令行的安装见 [安装](install.zh-CN.md)，全部设置项见 [配置说明](configuration.zh-CN.md)。

## 目录

1. [选择部署方式](#1-选择部署方式)
2. [使用云存储服务](#2-使用云存储服务)
3. [一条命令自建存储](#3-一条命令自建存储)
4. [Kubernetes（Helm）](#4-kuberneteshelm)
5. [云端 Agent 中继（workroam-server 中继模式）](#5-云端-agent-中继workroam-server-中继模式)
6. [完整模式服务端](#6-完整模式服务端)
7. [没有公网 IP：Cloudflare Tunnel](#7-没有公网-ipcloudflare-tunnel)
8. [代码仓库中转：Gitea](#8-代码仓库中转gitea)
9. [本地目录或 NAS](#9-本地目录或-nas)
10. [初始化各台电脑](#10-初始化各台电脑)
11. [运维](#11-运维)
12. [常见问题](#12-常见问题)

## 1. 选择部署方式

| 你的情况 | 需要部署什么 | 章节 |
| --- | --- | --- |
| 已经在用 AWS S3、Cloudflare R2、阿里云 OSS 或其他 S3 服务 | **什么都不用部署**：建一个桶，在初始化向导里选对应预设 | [§2](#2-使用云存储服务) |
| 有一台带公网 IP 的 Linux 服务器和一个域名 | `install.sh`：用 Docker 部署存储 + Caddy（自动 HTTPS） | [§3](#3-一条命令自建存储) |
| 使用 Kubernetes | `deploy/helm/workroam` 中的 Helm Chart | [§4](#4-kuberneteshelm) |
| 想让云端个人 Agent（Grok Bot、Muse）连到你的电脑 | 中继模式的 `workroam-server`，运行在 bot 的云端电脑或任意 VPS 上 | [§5](#5-云端-agent-中继workroam-server-中继模式) |
| 想要 6 位配对码、手机通知或 Web 查看器 | `install.sh --with-server`（完整模式） | [§6](#6-完整模式服务端) |
| 服务器没有公网 IP | `install.sh --tunnel`（Cloudflare Tunnel） | [§7](#7-没有公网-ipcloudflare-tunnel) |
| 每台电脑都能访问同一个 NAS 或共享目录 | 什么都不用部署，选“本地目录”预设 | [§9](#9-本地目录或-nas) |

对存储的要求：

- 支持 S3 API 与 SigV4 签名（路径风格或虚拟主机风格，初始化时自动探测）；
- 支持**条件写入**（`If-None-Match` / `If-Match`），用于项目租约，保证同一时间只有一台电脑写入。AWS S3、R2、RustFS 都支持。不支持时（例如 Garage）租约显示为“不支持”，同步仍然可用，但无法阻止两台电脑同时推送；
- 支持分段上传，用于大文件包。

初始化向导会运行 16 项能力检查（之后随时可以用 `workroam doctor` 重新检查），缺什么当场就能发现。

**安全模型。** 数据在客户端加密后才上传，存储、Caddy 和 `workroam-server` 只能看到密文。加密密码和访问密钥保存在每台电脑的系统密钥库里，不会进入存储桶，也不会到服务器上。每台电脑使用自己的、只能访问工作区存储桶的密钥，移除设备时会轮换密钥。

## 2. 使用云存储服务

1. 新建一个空的**私有**存储桶（名称随意，不需要开启版本控制）。
2. 为 Workroam 准备访问密钥，两种方式任选：
   - **由 Workroam 创建专用密钥**（AWS S3、RustFS）：填一次管理员 / IAM 密钥，选择“由 Workroam 创建专用密钥”。向导会创建一个只能访问这个桶的密钥，管理员密钥用完即弃，不会保存；
   - **自己在控制台创建**（R2、OSS 及其他）：只授予这个桶的读写权限，然后选择“使用当前密钥”。
3. 在初始化向导（或 `workroam setup`）中选择预设并填写端点：

| 预设 | `--preset` | 端点 | 区域 | 能否由 Workroam 创建密钥 |
| --- | --- | --- | --- | --- |
| AWS S3 | `aws` | `https://s3.<区域>.amazonaws.com` | 桶所在区域，如 `us-east-1` | 能（IAM） |
| Cloudflare R2 | `r2` | `https://<账户 ID>.r2.cloudflarestorage.com` | `auto` | 不能：创建对这个桶有“对象读和写”权限的 R2 API 令牌 |
| 阿里云 OSS | `oss` | `https://oss-<区域>.aliyuncs.com`，如 `https://oss-cn-hangzhou.aliyuncs.com` | 如 `cn-hangzhou` | 不能：按下面的策略创建 RAM 用户 |
| 其他 S3 兼容（MinIO、Ceph、Wasabi 等） | `other` | 你的端点 | 你的区域（默认 `us-east-1`） | 不能 |

无图形界面的电脑上，等价的命令：

```sh
# Secret Key 从标准输入读取（或环境变量 WORKROAM_SECRET_KEY），不要写在命令行里
workroam setup --new --preset r2 \
  --endpoint https://<账户 ID>.r2.cloudflarestorage.com \
  --access-key <Access Key ID> --secret-key-stdin --key-mode current --bucket workroam
```

自己创建密钥时的最小权限策略（AWS IAM 写法；阿里云 RAM 策略使用相同的动作，前缀改为 `oss:`，资源写成 `acs:oss:*:*:<桶>`）：

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": ["s3:ListBucket", "s3:GetBucketLocation"], "Resource": ["arn:aws:s3:::<桶>"] },
    { "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:AbortMultipartUpload", "s3:ListMultipartUploadParts"],
      "Resource": ["arn:aws:s3:::<桶>/*"] }
  ]
}
```

只读密钥（用于只读设备和 Web 查看器）只需要 `s3:ListBucket`、`s3:GetBucketLocation` 和 `s3:GetObject`。密钥能访问的范围超出工作区存储桶时，向导会提示；加上 `--accept-broad-key` 可以继续使用。

## 3. 一条命令自建存储

`install.sh` 在一个域名、一个 443 端口上部署 S3 存储（默认 RustFS，也可选 Garage / SeaweedFS）和 Caddy，由 Let's Encrypt 自动签发 HTTPS 证书。

**准备**

- 一台有公网 IP 的 Linux 服务器（x86_64 或 arm64），1 核 1 GB 内存即可，磁盘按项目大小准备；
- Docker Engine 及 Compose v2 插件（能运行 `docker compose version`）；
- 一个域名（如 `s3.example.com`），A/AAAA 记录指向这台服务器；
- 开放 80 与 443 端口（80 只用于证书验证和跳转到 HTTPS）。

**运行**

```sh
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install.sh \
  | sh -s -- --domain s3.example.com
```

Release 附带的 `install.sh` 会从对应版本的 tag 拉取 Compose 文件，部署结果可以复现。脚本会：

1. 在 `/opt/workroam`（非 root 运行时为 `~/workroam`）生成 Compose 项目；
2. 生成存储管理员密钥，保存在该目录的 `.env`（权限 0600）；
3. 启动存储和 Caddy，Caddy 自动申请 HTTPS 证书；
4. 打印向导里需要填写的内容，以及等价的 `workroam setup` 命令。

| 选项 | 说明 |
| --- | --- |
| `--domain <域名>` | 必填，指向这台服务器的主机名（不是 URL） |
| `--storage rustfs\|garage\|seaweedfs` | 存储，默认 `rustfs` |
| `--with-server` | 同时运行完整模式的 `workroam-server`，见 [§6](#6-完整模式服务端) |
| `--with-gitea` | 同时在 `https://<域名>/git/` 运行 Gitea，见 [§8](#8-代码仓库中转gitea) |
| `--tunnel <令牌>` | 没有公网 IP：经 Cloudflare Tunnel 接入，见 [§7](#7-没有公网-ipcloudflare-tunnel) |
| `--bucket <名称>` | 为 Garage / SeaweedFS 预先创建的桶，默认 `workroam` |
| `--ntfy <地址>` | 手机通知的 ntfy 主题（配合 `--with-server`） |
| `--dir <目录>` | 部署目录 |
| `--ref <git 引用>` | 使用哪个版本的部署文件（Release 附带的脚本默认为该版本的 tag，其他情况默认 `main`） |
| `--no-start` | 只生成文件，不启动 |

运行前可以用环境变量替换任意镜像：`RUSTFS_IMAGE`、`GARAGE_IMAGE`、`SEAWEEDFS_IMAGE`、`CADDY_IMAGE`、`WORKROAM_SERVER_IMAGE`、`GITEA_IMAGE`、`CLOUDFLARED_IMAGE`。生产环境建议固定版本，例如 `RUSTFS_IMAGE=rustfs/rustfs:<版本> WORKROAM_SERVER_IMAGE=ghcr.io/kverona-ai/workroam-server:0.1.0`。

**在向导中填写**

| 存储 | 向导里选择 | 密钥 |
| --- | --- | --- |
| RustFS（推荐） | “自建（install.sh 部署）”，端点 `https://<域名>` | 填脚本打印的管理员密钥，选“由 Workroam 创建专用密钥”：向导建桶并创建只能访问这个桶的密钥，管理员密钥用完即弃，不保存在电脑上 |
| Garage | “其他 S3 兼容”，端点 `https://<域名>`，区域 `us-east-1` | 脚本已创建桶和只能访问这个桶的密钥，填它并选“使用当前密钥”，桶选 `workroam` |
| SeaweedFS | 同 Garage | 同 Garage |

```sh
# RustFS，命令行等价写法（脚本会打印完整命令）
workroam setup --new --preset rustfs --endpoint https://s3.example.com \
  --access-key <管理员 Access Key> --secret-key-stdin --key-mode create
```

Garage 不支持条件写入，所以没有项目租约保护；需要租约时选 RustFS。

**同一个域名上的路由。** 全部流量都走 `https://<域名>:443`，由 Caddy 按请求区分：

- 带 S3 签名的请求（`Authorization: AWS4-HMAC-SHA256 …` 或预签名 URL）一律转给存储，所以桶名不受限制；
- 未签名的 `/healthz`、`/relay`、`/mcp`、`/.well-known/oauth-*`、`/oauth/*`、`/pair`、`/devices/*`、`/notify` 转给 `workroam-server`，`/web/` 是 Web 查看器（`--with-server` 时）；
- `/git/` 转给 Gitea（`--with-gitea` 时）。

这种部署下 Web 查看器与存储同源，**不需要给桶配置 CORS**。

## 4. Kubernetes（Helm）

Chart 部署 RustFS（StatefulSet + PVC）、可选的 `workroam-server`，以及这个主机名的 Ingress。Chart 在仓库和每个源码包中：

```sh
git clone --depth 1 --branch v0.1.0 https://github.com/kverona-ai/workroam && cd workroam
helm install workroam deploy/helm/workroam -n workroam --create-namespace \
  --set host=s3.example.com \
  --set ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt \
  --set server.enabled=true
helm status workroam -n workroam   # 打印读取管理员密钥的命令
```

| 参数 | 默认值 | 说明 |
| --- | --- | --- |
| `host` | `s3.example.com` | 公网主机名，设备用 `https://<host>` 作为存储端点 |
| `storage.image.repository` / `tag` | `rustfs/rustfs` / `latest` | 存储镜像，生产环境请固定 tag |
| `storage.accessKey` / `secretKey` | 空 | 管理员密钥；留空时自动生成一次，升级后保持不变（保存在 Secret 里） |
| `storage.existingSecret` | 空 | 使用自己的 Secret（键为 `access-key` / `secret-key`） |
| `storage.persistence.size` / `storageClass` | `100Gi` / 默认 | 数据卷 |
| `server.enabled` | `false` | 运行完整模式的 `workroam-server` |
| `server.image.tag` | Chart 的 `appVersion` | 服务端镜像 tag |
| `server.ntfy` | 空 | 手机通知的 ntfy 主题 |
| `server.webHost` | 空 | Web 查看器单独的主机名（Ingress 不能可靠地去掉路径前缀） |
| `server.persistence.size` | `1Gi` | 服务端状态卷 |
| `ingress.enabled` / `className` / `annotations` | `true` / 空 / 空 | Ingress 设置 |
| `ingress.tls.enabled` / `secretName` | `true` / `workroam-tls` | 主机名的 TLS |

使用 ingress-nginx 时，加上 `nginx.ingress.kubernetes.io/proxy-body-size: "0"`，否则大文件包会被拒绝。设置了 `server.webHost` 时，查看器与存储跨域，需要配置 [CORS](#web-查看器的-cors)。

## 5. 云端 Agent 中继（workroam-server 中继模式）

云端个人 Agent（Grok Bot、Muse）连不到你桌上的电脑。**中继模式**的 `workroam-server` 在中间搭桥：你的电脑主动连向它，云端 Agent 把它当作远程 MCP 连接器。中继模式不持有存储凭证，也不持有加密密钥；不需要 Docker、数据库或 root，通常直接运行在 bot 自己的云端电脑上。

**常规流程（约 2 分钟）**

1. 在电脑上：在桌面应用的 Agent 列表里连接云端 Agent，或运行 `workroam agent connect <agent>`。它会给出一段部署说明，含一次性**绑定码**（`wrb1:…`，30 分钟内有效），并提供给 bot 用的 Workroam Skill。
2. 把部署说明发给 bot，它会执行：

   ```sh
   curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh
   workroam-server --bind '<绑定码>' --tunnel
   ```

   `--tunnel` 会在缺少 `cloudflared` 时自动下载，建立出站的临时隧道，并打印一个 `https://…trycloudflare.com` 公网地址。
3. bot 把地址告诉你。把地址粘贴到桌面卡片里，或运行 `workroam agent connect <agent> --relay <地址>`。电脑对绑定挑战签名，绑定码随即作废。
4. 在电脑上确认连接卡片，选择 Agent 的权限（只读或同步）以及它能看到哪些项目。

**在自己的 VPS 上**，放在反向代理后面、使用固定地址：

```sh
workroam-server --listen 127.0.0.1:8787 --public-url https://relay.example.com --bind '<绑定码>'
```

用 systemd 保持运行：

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

`--bind` 只需要以同一个用户手动执行一次，绑定结果保存在状态文件（`~/.workroam-server/state.json`）里。`workroam-server --status` 会打印绑定、设备和 Agent 连接情况。

也可以用容器镜像：

```sh
docker run -d --name workroam-server --restart unless-stopped -p 8787:8787 \
  -v workroam-server:/home/nonroot/.workroam-server \
  ghcr.io/kverona-ai/workroam-server:0.1.0 --listen 0.0.0.0:8787 --public-url https://relay.example.com
```

注意：

- 临时隧道每次重启都会换地址。需要固定地址时，创建命名隧道并设置 `CLOUDFLARED_TOKEN`，把它的主机名作为 `--public-url`；
- 其他电脑在桌面上点“加入这个中继”即可加入已绑定的中继，由已绑定的电脑在卡片上确认；
- 一个中继可以服务多个云端 Agent；多个中继互不相干，撤销一个不影响其他；
- 地址必须是 HTTPS，否则云端 Agent 无法登录。

## 6. 完整模式服务端

`install.sh --with-server`（或 Helm 的 `server.enabled=true`）会在存储旁边运行 `workroam-server --full`。在中继模式之外，它还提供：

- **6 位配对码**：新电脑用 `workroam setup --join --server https://<域名>` 加短码加入，每台电脑获得**独立**的存储密钥，由只放在服务端容器里的管理员密钥创建；
- **手机通知**：把完成、失败、到达事件转发到 ntfy 主题（`--ntfy`）；
- **Web 查看器** `https://<域名>/web/`：在浏览器里浏览和下载项目文件。解密在浏览器中（WASM）用你的密码完成，使用只读密钥，服务端永远看不到明文。

先把服务端绑定到一台自己的电脑：

```sh
workroam agent connect <agent>                    # 在电脑上：输出绑定码 wrb1:…
cd /opt/workroam
docker compose run --rm workroam-server --bind '<绑定码>' --status
docker compose restart workroam-server
workroam agent connect <agent> --relay https://s3.example.com
```

管理员密钥只经环境变量传给服务端（`WORKROAM_ADMIN_ACCESS_KEY` / `WORKROAM_ADMIN_SECRET_KEY`，由 `install.sh` 从 `.env` 写入），不会出现在命令行参数或状态文件里。服务端从不持有加密密码。

### Web 查看器的 CORS

查看器与存储不在同一个域名时（使用 S3 / R2 / OSS 等云存储，或 Helm 设置了 `server.webHost`），需要允许查看器的来源读取存储桶：

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

- AWS S3：`aws s3api put-bucket-cors --bucket <桶> --cors-configuration '{"CORSRules": <上面的数组>}'`；
- Cloudflare R2：桶设置 → CORS Policy，粘贴上面的数组；
- 阿里云 OSS：桶 → 数据安全 → 跨域设置，来源、方法、允许 Headers 同上。

查看器只使用只读密钥（`workroam device add --read-only` 生成的配对码），不能写入或删除。

## 7. 没有公网 IP：Cloudflare Tunnel

1. 在 Cloudflare Zero Trust → Networks → Tunnels 创建隧道，复制令牌；
2. 运行 `install.sh --domain s3.example.com --tunnel <令牌>`：不开放任何入站端口，Caddy 只在容器网络内提供 HTTP；
3. 在隧道的 Public Hostname 里添加 `s3.example.com` → `http://caddy:80`。

Cloudflare 免费套餐限制单个请求体 100 MB。仓库很大、文件包可能超过这个大小时，建议改用有公网 IP 的服务器。

## 8. 代码仓库中转：Gitea

`remote` 模式的代码仓库需要一个每台电脑都能访问的 git 服务器。`--with-gitea` 在 `https://<域名>/git/` 运行 Gitea（SQLite，关闭注册与 SSH，只用 HTTPS 克隆），脚本会创建管理员 `workroam` 并打印密码。之后把仓库的模式设为 `remote`，`via` 指向这台服务器上的远程仓库（见 [配置说明 · workroam.toml](configuration.zh-CN.md#workroamtoml每个项目一份)）。

## 9. 本地目录或 NAS

如果每台电脑都挂载同一个目录（NAS 共享、随身携带的移动硬盘、同步盘），就不需要 S3 服务：在向导里选“本地目录”，或

```sh
workroam setup --new --preset local --endpoint file:///mnt/nas/workroam
```

端点为 `file://` 加目录的绝对路径。租约依赖文件系统，建议使用真正的网络共享，而不是由其他工具在后台同步的目录。

## 10. 初始化各台电脑

**第一台电脑**创建工作区：

1. 安装并打开桌面应用（见 [安装](install.zh-CN.md)），或在终端运行 `workroam setup`；
2. 选择“创建工作区” → 连接存储（见上文） → 运行 16 项检查；
3. 设置**加密密码**并保存**恢复密钥**。两者都丢失时，任何人（包括你）都无法解密数据；
4. 给这台电脑起名，选择默认项目目录，填写它能访问的内网主机（例如 `git.corp.internal`）；
5. 添加项目：`cd ~/projects/proj-a && workroam init`（或在应用里点“添加项目”）。

**其他电脑**加入：

1. 在已加入工作区的电脑上：设备 → “添加设备”，或运行 `workroam device add`（只读设备加 `--read-only`，`--qr` 显示二维码），得到配对码；
2. 在新电脑上：选择“加入已有工作区”，输入配对码和加密密码。在终端里 `workroam setup --join` 会依次询问；脚本中把配对码通过管道传给 `workroam setup --join --pairing-code-stdin`，密码放在 `WORKROAM_PASSWORD`。使用完整模式服务端时，输入 6 位码和服务端地址（`--server https://<域名>`）；
3. 选择要拉取的项目和目录（`--all-projects` 表示全部拉取到默认目录）。

无图形界面或脚本化初始化使用同样的参数，密钥从标准输入或环境变量读取（`WORKROAM_SECRET_KEY`、`WORKROAM_PASSWORD`）；没有终端时必须加 `--recovery-key-out <文件>`。随时可以运行 `workroam doctor` 重新检查。

## 11. 运维

**自建存储（install.sh）**

```sh
cd /opt/workroam
docker compose ps                              # 状态
docker compose logs -f storage                 # 存储日志（还有 caddy、workroam-server、gitea）
docker compose pull && docker compose up -d    # 更新镜像
docker compose down                            # 停止（数据保留在 Docker 卷里）
```

要换用新版本的部署文件，用同样的 `--dir` 再运行一次新版本的 `install.sh`，`.env` 里的密钥保持不变。

**备份。** 数据在 Docker 卷 `workroam_storage-data`（Garage 另有 `workroam_storage-meta`）。所有数据都是端到端加密的，备份这些卷到任何地方都不会暴露明文；也可以开启 Workroam 的异地备份（`workroam backup add`，或设置 → 异地备份），把工作区镜像到另一个存储桶或目录。丢失 `.env` 只会丢失管理员密钥，不影响解密。`workroam verify` 检查存储中的数据，`workroam verify --backup` 检查异地镜像。

**保留策略。** `workroam prune` 按工作区的保留策略清理并释放空间：默认保留最近 30 天的全部快照，再往前 52 周每周保留最新的一个（见 [配置说明](configuration.zh-CN.md#设置项一览)）。归档的项目一直保留，直到你删除它。

**客户端。** 桌面应用和命令行通过安装新版本升级，安装脚本和包管理器会原地替换二进制。`workroamd` 以当前用户的服务运行：macOS 上是 LaunchAgent `dev.workroam.workroamd`，Linux 上是 systemd 用户服务 `workroamd.service`，Windows 上是计划任务 `Workroam\workroamd`。日志在 `~/.workroam/logs/`。

**移除设备。** `workroam device remove <设备>`（设备 → 移除）会注销这台电脑并轮换存储密钥，被移除的电脑无法再读写。

**卸载。** 自建存储：在部署目录执行 `docker compose down -v`，会删除容器**和数据**。电脑上：`workroam config set autostart false` 移除后台服务，再删除应用和 `~/.workroam`。项目目录不会被改动。

## 12. 常见问题

| 现象 | 检查什么 |
| --- | --- |
| Caddy 申请不到证书 | 域名要解析到这台服务器，80 和 443 端口要能从公网访问；查看 `docker compose logs caddy` |
| 向导在“条件写入”检查失败 | 存储不支持 `If-None-Match` / `If-Match`。没有租约也能同步；需要租约保护时使用 RustFS、AWS S3 或 R2 |
| `SignatureDoesNotMatch` 或拒绝访问 | 检查区域和寻址方式（`--addressing path` 或 `virtual-host`）；AWS 和 OSS 的区域必须是桶所在区域 |
| 经过代理时大文件推送失败 | 调大代理的请求体限制（ingress-nginx 设置 `proxy-body-size: "0"`）；Cloudflare Tunnel 免费套餐单个请求限 100 MB |
| 云端 Agent 登录不了中继 | 中继地址必须是 HTTPS。临时隧道重启后会换地址：让 bot 报告新地址，或改用命名隧道 |
| Web 查看器报 CORS 错误 | 为查看器的来源配置 [CORS](#web-查看器的-cors) |
| 桌面应用或命令行连不上 `workroamd` | 运行 `workroam doctor`；查看 `~/.workroam/logs/workroamd.log`；重启服务，或在终端直接运行 `workroamd` |

**附带存储的许可证。** RustFS、SeaweedFS 为 Apache-2.0；Garage 为 AGPL-3.0，只作为独立容器运行，属于并列分发，不影响 Workroam 的 Apache-2.0 许可证。
