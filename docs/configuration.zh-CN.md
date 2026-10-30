# 配置说明

[English](configuration.md) · **简体中文**

本文列出 Workroam 读取的全部文件、设置项和环境变量。大多数情况下不需要手动编辑文件：桌面应用的**设置**页面和 `workroam config` 修改的是同一组设置项，保存前都会校验取值。存储和 `workroam-server` 的部署见 [部署指南](deployment.zh-CN.md)。

## 目录

- [文件位置](#文件位置)
- [修改设置](#修改设置)
- [设置项一览](#设置项一览)
- [machine.toml（每台电脑一份）](#machinetoml每台电脑一份)
- [workroam.toml（每个项目一份）](#workroamtoml每个项目一份)
- [mcp.toml（MCP 服务器）](#mcptomlmcp-服务器)
- [环境变量](#环境变量)
- [命令行通用选项](#命令行通用选项)
- [workroam-server](#workroam-server)
- [安装脚本](#安装脚本)

## 文件位置

**每台电脑上**（`~` 为用户主目录；设置 `WORKROAM_HOME` 可以把 `~/.workroam` 换到别处）：

| 路径 | 内容 |
| --- | --- |
| `~/.workroam/machine.toml` | 本机设置与工作区位置，见 [下文](#machinetoml每台电脑一份) |
| `~/.workroam/logs/` | `workroamd.log`，按天滚动 |
| `~/.workroam/state/` | 后台服务的本地状态 |
| `~/.workroam/adapters/` | 自定义 Agent 适配器，与内置适配器 `id` 相同时覆盖内置版本 |
| 系统密钥库 | 存储访问密钥、加密密码等机密：macOS Keychain、Windows 凭据管理器、Linux Secret Service。不会写入 `machine.toml` |
| `~/.workroam/secrets.json` | 只在没有可用的系统密钥库，或设置了 `WORKROAM_SECRETS=file` 时使用（权限 0600） |

后台服务监听本地套接字：macOS 为 `~/Library/Application Support/Workroam/workroamd.sock`，Linux 为 `$XDG_RUNTIME_DIR/workroam/workroamd.sock`（或 `~/.workroam/workroamd.sock`），Windows 为命名管道 `\\.\pipe\workroam-<用户名>`。

**每个项目目录中**（除特别说明外都随项目同步）：

```text
proj-a/
├── AGENTS.md              # 共享规则 + 本机环境块（自动生成）
├── CLAUDE.md              # 仅一行 @AGENTS.md（需要时生成）
├── .workroam/
│   ├── workroam.toml      # 本项目同步策略
│   ├── mcp.toml           # MCP 唯一定义，含各系统覆盖项
│   ├── env/               # windows.md / macos.md / linux.md 环境说明
│   └── local/             # 仅本机：哈希缓存、操作日志、映射表（不同步）
├── .agents/skills/        # Skills 唯一来源
├── .handoff/HANDOFF.md    # 跨 Agent 交接
└── repo-a/                # 代码仓库（每台电脑自有 .git）
```

请在项目根目录启动 Agent，不要在 `repo-a/` 里启动：部分 Agent 只从 git 仓库根往下找规则文件，会看不到项目根的 `AGENTS.md`。

**存储桶中**的内容全部加密。工作区级别的设置（保留策略、云端 Agent 默认使用的设备）保存在存储桶里，所以每台电脑看到的都一样。

## 修改设置

```sh
workroam config list                         # 全部设置项、当前值及来源
workroam config get sync.auto
workroam config set sync.auto auto           # 本机
workroam config set sync.auto off -p proj-a  # 只覆盖某个项目
workroam --json config list                  # 机器可读输出
```

每个设置项有自己的作用范围：

| 范围 | 保存位置 | 说明 |
| --- | --- | --- |
| machine | `machine.toml` | 只影响本机 |
| project | `machine.toml`，可在 `workroam.toml` 中按项目覆盖 | 不加 `-p` 时设置本机默认值；加 `-p <项目>` 时设置项目覆盖值，覆盖值随项目同步 |
| project-only | `workroam.toml` | 必须加 `-p <项目>` |
| workspace | 存储桶 | 所有电脑共用 |

`workroam config list` 会显示每个值的 `source`：`machine`、`project`、`workspace` 或 `default`。

## 设置项一览

| 设置项 | 范围 | 取值 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `device.name` | machine | 文本 | 初始化时设置 | 本机名称，显示在所有设备和接力路线上 |
| `root` | machine | 路径 | `~/projects` | 新项目的默认位置（`<root>/<项目>`），每个项目仍可单独指定目录 |
| `reachable` | machine | 逗号分隔的主机名 | 空 | 本机能访问的内网主机（如 `git.corp.internal`），用于选择代码仓库模式以及判断能在哪里推送提交 |
| `autostart` | machine | `true` / `false` | `true` | 登录时启动 `workroamd`（安装或移除用户服务） |
| `sync.auto` | project | `off` / `ask` / `auto` | `ask` | 自动同步：从不、弹卡片询问、不询问直接同步 |
| `sync.agent_requires_approval` | project | `true` / `false` | `true` | Agent 发起的同步需要你批准 |
| `sync.idle_minutes` | machine | 数字 | `15` | 多少分钟没有改动后提示自动推送 |
| `sync.on_sleep` | machine | `off` / `ask` / `auto` | `ask` | 电脑休眠或关机前推送并释放租约。macOS 会为推送最多等待 25 秒；Windows 与 Linux 不等待，作业在唤醒后继续（连接中断导致失败时会像其他作业一样提示）。`ask` 弹出授权卡片；关机前无人能回答，只有 `auto` 会推送 |
| `sync.on_wake` | machine | `true` / `false` | `true` | 唤醒并联网后检查是否有新进度 |
| `sync.approval_timeout` | machine | 时长（`10m`、`1h`） | `10m` | 授权卡片等待你回应的时间 |
| `notify.ntfy` | machine | URL | 空 | 手机通知的 ntfy 主题（直接发送，或由完整模式服务端转发） |
| `notify.on_success` | machine | `true` / `false` | `true` | 同步成功完成时也发送通知 |
| `ui.banner` | machine | `true` / `false` | `true` | 显示命令行启动横幅 |
| `ui.lang` | machine | `zh` / `en` / 空 | 空 | 界面语言，空为跟随系统 |
| `mcp.enabled` | project | `true` / `false` | `true` | 供 Agent 使用的本机 MCP server（`workroam mcp`） |
| `remote.enabled` | project-only | `true` / `false` | `false` | 允许云端 Agent 经中继访问这个项目 |
| `remote.default_device` | workspace | 设备名 | 空 | 经中继为云端 Agent 提供服务的电脑 |
| `retention.days` | workspace | 数字 | `30` | 保留最近 N 天的全部快照 |
| `retention.weekly` | workspace | 数字 | `52` | 更早的快照，在 N 周内每周保留最新的一个（`0` 表示每周一个、永久保留） |

`workroam pause --for 2h` 与 `workroam resume`（托盘的“暂停自动同步”）会写入 `sync.paused_until`。

## machine.toml（每台电脑一份）

`~/.workroam/machine.toml` 由初始化流程和 `workroam config set` 写入。完整示例：

```toml
name = "office-win"
os = "windows"                   # windows | macos | linux
device_id = "d_7f3a…"            # 初始化时分配，不要修改
access = "read-write"            # read-write | read-only
root = 'D:\projects'
reachable = ["git.corp.internal", "github.com"]
autostart = true

[workspace]
endpoint = "https://s3.example.com"   # 本地目录为 file:///路径
bucket = "workroam"
prefix = ""                           # 可选：桶内的对象前缀
addressing = "path"                   # path | virtual-host（初始化时探测）
region = "us-east-1"
preset = "rustfs"                     # rustfs | aws | r2 | oss | other | local
workspace_id = "w_…"
capabilities = { conditional_create = true, conditional_overwrite = true, multipart = true }

[sync]
auto = "ask"
idle_minutes = 15
on_sleep = "ask"
on_wake = true
agent_requires_approval = true
approval_timeout = "10m"
paused_until = ""

[notify]
ntfy = ""
on_success = true

[ui]
banner = true
lang = ""

[mcp]
enabled = true

[remote]                              # 由 `workroam agent connect --relay` 写入
relay_url = ""
relay_id = ""

[backup]                              # 由 `workroam backup add` 写入
kind = "s3"                           # s3 | local
endpoint = "https://s3.backup.example.com"
bucket = "workroam-mirror"
prefix = ""
region = "us-east-1"
path = ""                             # kind = "local" 时使用
schedule = "daily"                    # hourly | daily | weekly | manual
delete_delay_days = 30                # 删除操作延迟这么多天才同步到镜像

[projects.proj-a]
path = 'D:\projects\proj-a'
state = "bound"                       # pending | bound | missing | archived
```

工作区位置和存储能力在初始化时探测得出，请不要手动修改：更换存储时重新运行 `workroam setup`，更换访问密钥用 `workroam credentials update`。访问密钥与加密密码永远不在这个文件里。

## workroam.toml（每个项目一份）

`<项目>/.workroam/workroam.toml` 由 `workroam init` 创建并随项目同步，所以每台电脑执行同样的策略。

```toml
name = "proj-a"

[sync]
exclude = ["node_modules/", "target/", ".venv/", "dist/", "__pycache__/", ".DS_Store", "Thumbs.db"]
auto = "ask"                     # 可选：覆盖本机的 sync.auto
agent_requires_approval = true   # 可选
sessions = true                  # Agent 会话随项目迁移（默认 true）

[agents]
claude = { state = true }
codex  = { state = true }
grok   = { state = false }

[repos.order-service]            # 内网仓库
path = "repo-a"
mode = "bundle"
history = "recent:50"
branches = ["main", "feature/*"]
extras = ["docs/generated/"]

[repos.oss-tool]                 # 每台电脑都能连 origin
path = "repo-b"
mode = "remote"
via = "origin"

[repos.scripts]
path = "repo-c"
mode = "files"

[mcp]
enabled = true                   # 可选：覆盖 mcp.enabled

[remote]
enabled = false                  # 允许云端 Agent 访问这个项目
```

| 键 | 说明 |
| --- | --- |
| `sync.exclude` | 不放进快照的路径模式（类似 gitignore），默认列表如上 |
| `sync.auto`、`sync.agent_requires_approval` | 同名设置项的项目覆盖值 |
| `sync.sessions` | Agent 会话随项目迁移，在另一台电脑上可以继续同一段对话 |
| `agents.<id>.state` | 为哪些 Agent 生成本项目的配置。内置 id：`claude`（Claude Code）、`codex`、`grok`（Grok Build）、`antigravity`、`zcode`、`pi`；云端 Agent `grok-bot`、`muse` |
| `repos.<名称>.path` | 代码仓库目录，相对于项目根目录 |
| `repos.<名称>.mode` | `bundle`：提交以 git bundle 的形式随快照传递（适合内网仓库）。`remote`：经每台电脑都能访问的 git 服务器传递（`via`）。`files`：工作区按文件同步，`.git` 留在本机 |
| `repos.<名称>.history` | `bundle` 模式的历史档位：`full`、`recent:N`（只带最近 N 次提交，回到公司后变基到真实历史上）或 `snapshot`（不带历史） |
| `repos.<名称>.branches` | 要带上的分支名或通配模式，当前检出的分支总会带上；留空时只带当前分支 |
| `repos.<名称>.extras` | 仓库中另外按普通文件同步的路径（例如未纳入版本控制的生成文档） |
| `repos.<名称>.via` | `remote` 模式下用于推送和拉取的 git 远端（默认 `origin`） |

`workroam init --dry-run` 会列出发现的每个仓库的推荐模式及原因；`--repo <路径>=<模式>[:<历史>]` 与 `--skip-repo <路径>` 可以覆盖推荐。

## mcp.toml（MCP 服务器）

`<项目>/.workroam/mcp.toml` 中每个 MCP server 只定义一次。每次拉取后，Workroam 据此为每个启用的 Agent 生成原生 MCP 配置，并使用适合当前系统的命令。

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "{project_dir}/docs"]
targets = ["claude", "codex", "grok", "antigravity"]

[servers.filesystem.windows]
command = "cmd"
args = ["/c", "npx", "-y", "@modelcontextprotocol/server-filesystem", "{project_dir}/docs"]

[servers.jira]
url = "https://jira.example.com/mcp"
headers = { X-Project = "{project}" }
```

| 键 | 说明 |
| --- | --- |
| `command`、`args`、`env` | 本地（stdio）server |
| `url`、`headers` | 远程（HTTP）server |
| `targets` | 使用这个 server 的 Agent id；留空表示全部 Agent |
| `windows`、`macos`、`linux` | 各系统覆盖项。写了 `command` 时替换命令和参数，只写 `args` 时只替换参数，`url` 替换地址，`env` / `headers` 合并 |

任意取值中，`{project_dir}` 会替换为本机的项目目录，`{project}` 替换为项目名。

## 环境变量

**桌面应用、命令行与 `workroamd`**

| 变量 | 说明 |
| --- | --- |
| `WORKROAM_HOME` | 用其他目录代替 `~/.workroam`（便携安装、测试）。不同目录的密钥库条目和套接字互相独立 |
| `WORKROAM_SECRETS=file` | 机密保存在 `~/.workroam/secrets.json`（权限 0600）而不是系统密钥库，例如没有 Secret Service 的 Linux 服务器 |
| `WORKROAM_SOCKET` | `workroamd` 套接字路径（Windows 为命名管道） |
| `WORKROAM_LOG` | 日志过滤，例如 `debug` 或 `workroam_engine=debug,info`（默认 `info`） |
| `WORKROAM_LANG` | 界面语言 `zh` 或 `en`；未设置时依次参考 `LC_ALL`、`LC_MESSAGES`、`LANG`、`LANGUAGE` |
| `WORKROAM_NO_BANNER=1` | 不显示命令行启动横幅 |
| `WORKROAM_AGENT` | 运行命令行的 Agent 名称。没有终端时执行的命令视为 Agent 请求并弹出授权卡片；Workroam 会通过各自的环境变量识别 Claude Code、Codex、Grok Build 与 Antigravity，这个变量用来显式指定名称 |
| `WORKROAM_NO_AUTOSTART` | 命令需要 `workroamd` 时不自动启动它 |
| `WORKROAM_IN_PROCESS=1` | 不经过 `workroamd`，在命令行进程内直接执行 |
| `WORKROAM_SECRET_KEY` | `setup` 与 `credentials update` 使用的存储 Secret Key（代替 `--secret-key-stdin`） |
| `WORKROAM_PASSWORD` | `setup` 与 `password change` 使用的加密密码（代替 `--password-stdin`） |
| `WORKROAM_RECOVERY_KEY` | `password reset` 使用的恢复密钥（代替 `--recovery-key-stdin`） |
| `WORKROAM_ADMIN_TOKEN` | 存储创建专用密钥时需要的管理 API 令牌 |
| `WORKROAM_BACKUP_SECRET_KEY` | `backup add` 使用的异地备份存储 Secret Key |

机密请通过标准输入或上述环境变量传递，不要写在命令行参数里：其他用户可以在进程列表中看到参数。

## 命令行通用选项

以下选项适用于所有 `workroam` 命令：

| 选项 | 说明 |
| --- | --- |
| `-p, --project <名称>` | 操作的项目（默认：当前目录所属的项目） |
| `--json` | 在标准输出给出稳定的 JSON（含 `schema_version`、`code`），没有进度条和横幅；Agent 与脚本应使用这种输出 |
| `-q, --quiet` | 只输出错误和最终结果 |
| `-y, --yes` | 对命令自身的确认回答“是”（不会代替同步授权，也不会代替需要输入名称的确认） |
| `--color auto\|always\|never` | 彩色输出 |
| `--no-banner` | 本次不显示启动横幅 |
| `--lang zh\|en` | 本次命令的界面语言 |

`workroam <命令> --help` 列出各命令自己的选项；`workroam completions <shell>` 输出 bash、zsh、fish、PowerShell 或 elvish 的补全脚本。退出码见 [技术架构 §15](architecture.md#cli-退出码)。

## workroam-server

| 选项 | 环境变量 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--listen <地址>` | `WORKROAM_SERVER_LISTEN` | `127.0.0.1:8787` | 监听地址 |
| `--public-url <url>` | `WORKROAM_SERVER_PUBLIC_URL` | 无 | 能访问到这个端口的公网 `https://` 地址 |
| `--tunnel` | | 关闭 | 建立出站的 Cloudflare 隧道获得公网 HTTPS 地址（缺少 `cloudflared` 时自动下载）。设置 `CLOUDFLARED_TOKEN` 时使用命名隧道而不是临时隧道 |
| `--bind <绑定码>` | | | 用 `workroam agent connect` 给出的一次性绑定码绑定到设备 |
| `--state <文件>` | `WORKROAM_SERVER_STATE` | `~/.workroam-server/state.json` | 状态文件（`WORKROAM_SERVER_HOME` 可以更换整个目录） |
| `--status` | | | 打印绑定、设备和连接情况后退出 |
| `--full` | | 关闭 | 完整模式：配对码、每台设备独立密钥、ntfy、Web 查看器（容器镜像及 `--features full` 构建提供） |
| `--admin-endpoint <url>` | `WORKROAM_ADMIN_ENDPOINT` | 存储端点 | 完整模式：存储管理 API 地址，与设备使用的端点不同时填写 |
| `--ntfy <url>` | `WORKROAM_NTFY` | 无 | 完整模式：手机通知的 ntfy 主题 |
| `--web-dir <目录>` | | 内置 | 完整模式：从这个目录提供 Web 查看器 |
| | `WORKROAM_ADMIN_ACCESS_KEY`、`WORKROAM_ADMIN_SECRET_KEY` | | 完整模式：用于创建每台设备独立密钥的存储管理员密钥。只能通过环境变量传入，不能用参数，也不会写入状态文件 |
| | `WORKROAM_ADMIN_TOKEN` | | 完整模式：需要令牌的存储所用的管理 API 令牌 |
| | `WORKROAM_LOG` | `workroam_server=info` | 日志过滤 |

## 安装脚本

`install-cli.sh` / `install-server.sh` 支持 `--version <x.y.z>` 与 `--dir <路径>`；PowerShell 脚本读取 `WORKROAM_VERSION`、`WORKROAM_INSTALL_DIR`、`WORKROAM_DOWNLOAD_URL`、`WORKROAM_COMPONENT`。`WORKROAM_DOWNLOAD_URL` 让所有脚本改从镜像下载，见 [安装 · 镜像与内网](install.zh-CN.md#镜像与内网)。

存储部署脚本 `install.sh` 及其镜像变量见 [部署指南](deployment.zh-CN.md#3-一条命令自建存储)。
