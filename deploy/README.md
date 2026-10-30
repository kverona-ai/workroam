# 部署

> 完整的部署指南（云存储、自建、Kubernetes、中继、运维）见 [docs/deployment.zh-CN.md](../docs/deployment.zh-CN.md)（[English](../docs/deployment.md)）。本文说明本目录中各文件的用途。

Workroam 的全部逻辑都在客户端里，服务器只需要一个 S3 兼容存储。已经有 AWS S3、Cloudflare R2、阿里云 OSS 时**什么都不用部署**，直接在初始化向导里选对应预设即可。本目录用于自建存储，以及可选的 `workroam-server`（[技术架构 §18、§20](../docs/architecture.md#20-部署架构)）。

| 文件 | 用途 |
| --- | --- |
| [`install.sh`](install.sh) | 一条命令部署存储 + Caddy（自动 HTTPS），可选 workroam-server、Gitea、Cloudflare Tunnel |
| [`compose/`](compose/) | `install.sh` 使用的 Docker Compose 片段：`rustfs.yml`、`garage.yml`、`seaweedfs.yml`、`caddy.yml`、`server.yml`、`gitea.yml`、`cloudflared.yml` |
| [`caddy/Caddyfile`](caddy/Caddyfile) | 单一域名 443 端口的路由模板 |
| [`helm/workroam/`](helm/workroam/) | Kubernetes Helm Chart（RustFS + 可选 workroam-server + Ingress） |
| [`server/Dockerfile`](server/Dockerfile) | `ghcr.io/kverona-ai/workroam-server` 镜像（发布流水线使用） |

## 一条命令部署存储

准备一台有公网 IP 的 Linux 服务器，安装 Docker（含 `docker compose` 插件），把域名解析到这台服务器，开放 80 与 443 端口，然后运行：

```sh
curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh \
  | sh -s -- --domain s3.example.com
```

脚本会在 `/opt/workroam`（非 root 时为 `~/workroam`）生成 Compose 项目，启动存储与 Caddy，Caddy 自动申请 HTTPS 证书，最后打印向导里需要填写的内容。密钥保存在该目录的 `.env`（权限 0600）。

| 选项 | 说明 |
| --- | --- |
| `--domain <域名>` | 必填，指向这台服务器的域名 |
| `--storage rustfs\|garage\|seaweedfs` | 存储，默认 `rustfs` |
| `--with-server` | 同时运行完整模式的 `workroam-server`：6 位配对码、每台设备独立密钥、手机通知转发、Web 查看器 `https://<域名>/web/` |
| `--with-gitea` | 同时运行 Gitea（`https://<域名>/git/`），作为 `remote` 模式代码仓库的中转库 |
| `--tunnel <令牌>` | 没有公网 IP：经 Cloudflare Tunnel 接入，见下文 |
| `--bucket <名称>` | Garage / SeaweedFS 预先创建的桶，默认 `workroam` |
| `--ntfy <地址>` | 手机通知的 ntfy 主题（配合 `--with-server`） |
| `--dir <目录>` | 部署目录 |
| `--ref <git 引用>` | 使用哪个版本的部署文件，默认 `main` |
| `--no-start` | 只生成文件，不启动 |

用同样的 `--dir` 再运行一次即可更新镜像，已有密钥保持不变。镜像可以在运行前用环境变量替换，例如 `RUSTFS_IMAGE=rustfs/rustfs:<版本>`、`WORKROAM_SERVER_IMAGE=ghcr.io/kverona-ai/workroam-server:0.1.0`。

### 在向导中填写

| 存储 | 向导里选择 | 密钥 |
| --- | --- | --- |
| RustFS（推荐） | “自建（install.sh 部署）”，端点 `https://<域名>` | 填脚本打印的管理员密钥，选“由 Workroam 创建专用密钥”：向导建桶并创建只能访问这个桶的密钥，管理员密钥用完即弃，不保存在电脑上 |
| Garage | “其他 S3 兼容”，端点 `https://<域名>`，区域 `us-east-1` | 脚本已创建桶和只能访问这个桶的密钥，填它并选“使用当前密钥”，桶选 `workroam` |
| SeaweedFS | 同 Garage | 同 Garage |

命令行等价写法（脚本会打印完整命令）：

```sh
workroam setup --new --preset rustfs --endpoint https://s3.example.com \
  --access-key <管理员 Access Key> --secret-key-stdin --key-mode create
```

存储不支持条件写入时（例如 Garage），项目租约显示为“不支持”，同步仍然可用；需要租约保护时选 RustFS。

### 同一个域名上的路由

全部流量都走 `https://<域名>:443`（PRD DEP-02）。Caddy 按请求区分：

- 带 S3 签名的请求（`Authorization: AWS4-HMAC-SHA256 …` 或预签名 URL）一律转给存储，所以桶名不受限制；
- 未签名的 `/healthz`、`/relay`、`/mcp`、`/.well-known/oauth-*`、`/oauth/*`、`/pair`、`/devices/*`、`/notify` 转给 `workroam-server`；`/web/` 是 Web 查看器；
- `/git/` 转给 Gitea。

Web 查看器与存储同源，**不需要给桶配置 CORS**。

## workroam-server

`--with-server` 部署的是完整模式。先绑定到一台自己的电脑，之后其他电脑可以用 6 位配对码加入，云端 Agent 也可以经它连接：

```sh
workroam agent connect <agent>                    # 在电脑上：输出绑定码 wrb1:…
cd /opt/workroam
docker compose run --rm workroam-server --bind '<绑定码>' --status
docker compose restart workroam-server
workroam agent connect <agent> --relay https://s3.example.com
```

只给云端个人 Agent 用、不需要存储时，不必用本目录：在 bot 的云端电脑或任意 VPS 上运行中继模式即可，见 [安装文档](../docs/install.md)。

服务端从不持有加密密码，看不到明文；管理员密钥只放在服务端容器的环境变量里，用来给新设备创建独立密钥。

## 没有公网 IP：Cloudflare Tunnel

1. 在 Cloudflare Zero Trust → Networks → Tunnels 创建隧道，复制令牌；
2. 运行 `install.sh --domain s3.example.com --tunnel <令牌>`：不开放任何入站端口，Caddy 只在容器网络内提供 HTTP；
3. 在隧道的 Public Hostname 里添加 `s3.example.com` → `http://caddy:80`。

Cloudflare 免费套餐限制单个请求体 100 MB；仓库很大、打包文件可能超过这个大小时，建议改用有公网 IP 的服务器。

## 代码仓库中转：Gitea

`remote` 模式的代码仓库需要一个两台电脑都能访问的 git 服务器。`--with-gitea` 在 `https://<域名>/git/` 运行 Gitea（SQLite，关闭注册与 SSH，只用 HTTPS 克隆），脚本会创建管理员 `workroam` 并打印密码。

## Kubernetes

```sh
helm install workroam deploy/helm/workroam \
  --set host=s3.example.com \
  --set ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt \
  --set server.enabled=true
```

Chart 部署 RustFS（StatefulSet + PVC）、可选的 workroam-server 与一个 Ingress。管理员密钥未指定时自动生成并保存在 Secret 里，`helm status workroam` 会打印读取命令。Ingress 不能可靠地去掉路径前缀，所以 Web 查看器需要单独的 `server.webHost`；这种情况下查看器与存储跨域，需要按下文配置 CORS。使用 ingress-nginx 时建议加上 `nginx.ingress.kubernetes.io/proxy-body-size: "0"`。

## 使用云存储时的 Web 查看器 CORS

Web 查看器在浏览器里直接读取 S3。查看器与存储不在同一个域名时（例如存储用 AWS S3 / R2 / OSS，或 Helm 的 `server.webHost`），需要允许查看器的来源读取桶：

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

## 运维

```sh
cd /opt/workroam
docker compose ps                 # 状态
docker compose logs -f storage    # 日志
docker compose pull && docker compose up -d   # 升级
docker compose down               # 停止（数据保留在 Docker 卷里）
```

数据在 Docker 卷 `workroam_storage-data`（Garage 另有 `workroam_storage-meta`）。仓库端到端加密，备份这些卷或在 Workroam 里开启“异地备份”都不会暴露明文；丢失 `.env` 只影响管理员密钥，不影响数据解密。

## 许可证

RustFS、SeaweedFS 为 Apache-2.0；Garage 为 AGPL-3.0，只作为独立容器运行，属于并列分发，不影响 Workroam 代码的 Apache-2.0 许可证。
