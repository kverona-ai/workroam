# 桌面应用与命令行功能对照

| 项目 | 内容 |
| --- | --- |
| 版本 | v0.1 |
| 最后更新 | 2026-10-30 |
| 关联文档 | [产品需求 §8.17 命令行 · §8.22 一致性](prd.md#822-界面与命令行一致性par) · [技术架构 §17](architecture.md#17-命令行与界面一致性) · [ADR-0009](adr/0009-desktop-cli-feature-parity.md) · [高保真原型](prototype/README.md) |

**规则：** 每一项面向用户的功能，在桌面应用（向导、主窗口、卡片、托盘）和 `workroam` 命令行里都必须有入口，两边调用同一个 IPC 方法，选项、默认值、校验规则和结果完全一致（PAR-01）。只在一侧存在的能力必须列在本文 [§11 豁免清单](#11-豁免清单) 中。

本表是唯一的对照依据：新增或修改功能时同时更新桌面应用、命令行与本表；实现时由 `crates/workroam-ipc/parity.toml` 承载同样的内容，并由 CI 检查（[技术架构 §17.4](architecture.md#174-一致性检查)）。[高保真原型](prototype/README.md) 包含本表要求的全部桌面入口。

表中“命令行”一列省略了全局参数：命令默认作用于当前目录所在的项目，`-p, --project <项目>` 指定其他项目；`--json`、`-q, --quiet`、`--yes` 等见 [PRD §8.17.2](prd.md#8172-全局参数与输出)。

---

## 1. 首次初始化（向导 ↔ `workroam setup`）

`workroam setup` 交互模式的提问顺序、选项与默认值和向导逐步对应；非交互模式用参数一次给全。两者都调用 `setup.*`（ADR-0005）。

| 功能 | 桌面应用 | 命令行 |
| --- | --- | --- |
| 选择路径 | 欢迎页：新建工作区 / 加入已有工作区 / 从异地备份恢复 | `workroam setup` 交互选择；`--new` / `--join` / `--from-backup` |
| 连接存储 | 5 个预设（自建、AWS S3、R2、OSS、其他）+ 端点；自建时显示部署命令 | `--preset rustfs\|aws\|r2\|oss\|other --endpoint <地址>`；交互模式选 `rustfs` 时同样打印 `install.sh` 命令 |
| 自动识别地址风格与区域 | 端点下方实时显示 | 输出识别结果；`--addressing path\|virtual-host`、`--region` 可强制指定 |
| 访问密钥 | Access Key / Secret 输入框 | `--access-key <ID> --secret-key-stdin`（或环境变量 `WORKROAM_SECRET_KEY`）；交互模式隐藏输入 |
| 密钥权限方式 | 创建专用密钥（读写 / 只读）/ 使用当前密钥；不支持创建时隐藏 | `--key-mode create\|current --key-access read-write\|read-only`；存储不支持时 `create` 报参数错误并说明 |
| 权限过宽确认 | 警告 + “我了解风险，继续使用当前密钥” | 交互模式显示同一警告并要求确认；非交互需加 `--accept-broad-key`，否则以退出码 21 结束 |
| 存储桶 | 默认新建（可“换一个”）/ 选择已有（列表，标注已是工作区、非空、空桶）/ 自定义（实时校验） | `--bucket auto\|<桶名>`；交互模式列出可见桶并带同样的标注，自定义时逐条显示校验结果 |
| 桶内前缀 | 输入框；非空且非工作区的桶必须填写 | `--prefix <前缀>`；规则相同 |
| 所选桶已是工作区 | 提示返回第一步改为加入 | 同样提示，并给出等价的 `workroam setup --join` 用法 |
| 能力检查 | 16 项逐项刷新，通过 / 警告 / 失败 + 修复建议；“重新检查” | 逐项输出同样的结果与建议；交互模式失败后选择“重新检查 / 返回修改”；非交互失败以退出码 21 结束 |
| 加密密码 | 两次输入 + 强度提示 | 交互模式两次隐藏输入 + 强度提示；非交互 `--password-stdin` 或 `WORKROAM_PASSWORD` |
| 恢复密钥 | 复制 / 下载 .txt / 打印；输入指定两组确认 | 交互模式显示后要求输入指定两组；非交互必须给 `--recovery-key-out <文件>`（写入成功即视为已保存） |
| 加入：配对码 | 粘贴配对码，或用摄像头扫描二维码 | 交互模式粘贴；非交互 `--pairing-code-stdin`（二维码扫描见豁免清单） |
| 加入：解锁 | 加密密码；“忘记密码？使用恢复密钥” | 加密密码；`--recovery-key-stdin` 改用恢复密钥 |
| 本机设置 | 设备名、默认项目位置、可达主机（自动探测 + 连通性）、自动同步档位、开机自启、检测到的 Agent | `--device-name --root --reachable <主机,…> --auto off\|ask\|auto --autostart\|--no-autostart`；交互模式显示探测到的可达主机与 Agent 列表 |
| 加入：选择本机项目与目录 | 勾选项目，每个项目选择本机目录 | 交互模式逐项选择；非交互 `--project <项目>[=<目录>]`（可重复）或 `--all-projects` |
| 从异地备份恢复后重新加入 | 加入时选择“作为原设备 <名称>” | `workroam setup --join --as-device <设备名>` |
| 完成 | 配对码文本 + 二维码；“接入第一个项目” | 打印配对码；`--qr` 在终端用字符画显示二维码；最后提示 `workroam init` |

## 2. 项目

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 项目列表与状态 | 主窗口“项目”、托盘菜单 | `workroam project list`：工作区内全部项目，含“未添加到本机”的项目，列出本机目录与状态 | `project.list` |
| 项目概况 | 项目详情：租约、最新快照、待推送改动、代码仓库与本机能否推送 | `workroam status`：同样的字段 | `status` |
| 交接文档 | 项目详情“交接文档”区 | `workroam handoff`：显示 `HANDOFF.md` 的结构化摘要；`--template` 输出模板 | `handoff.read` |
| 接入项目 | “接入项目”对话框：选择目录、目录检查、发现的仓库与推荐模式、导入已有 Agent 配置、跨平台检查、“接入并推送” | `workroam init [<目录>]`：同样的步骤与输出，最后发起第一次推送（授权时可选“稍后”） | `project.scan` / `project.init` |
| 预览扫描结果 | 对话框中实时显示 | `workroam init --dry-run`：只扫描并报告，不写任何文件 | `project.scan` |
| 修改推荐的同步模式 | 仓库行中修改模式与历史档位 | 交互模式逐个确认；非交互 `--repo <路径>=<模式>[:<历史档位>]`（可重复） | `project.init` |
| 在本机添加已有项目 | 项目详情（未添加到本机）：选择本机目录 →“添加并拉取”；加入向导中勾选 | `workroam project add <项目> [--path <目录>]`：添加后立即发起拉取 | `project.add` |
| 更换本机目录 | 项目详情“本机目录 → 更换…”，只在目录为空、尚未同步时显示 | `workroam project move <项目> --path <目录>`；已同步时退出码 18 | `project.move` |
| 在本机移除项目 | 项目详情“在本机移除…” | `workroam project remove <项目>` | `project.remove` |
| 目录不存在：重新检查 | “目录已放回原处，重新检查” | `workroam status` 与 `workroam project list` 每次运行都重新检查绑定 | `status` |
| 目录不存在：移除后重新添加 | “移除后重新添加…” | `workroam project remove` 后 `workroam project add --path <新目录>` | `project.remove` / `project.add` |

## 3. 推送、拉取与租约

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 推送 | 托盘每个项目的“推送” | `workroam push` | `push.plan` + `job.start` |
| 推送并释放租约 | 项目详情“推送并释放” | `workroam push --release` | 同上 |
| 推送全部并释放 | 托盘“推送全部并释放” | `workroam push --all --release` | 同上（逐项目） |
| 拉取（获取租约） | 项目详情“拉取…”、托盘每个项目的“拉取”、到达卡片“拉取” | `workroam pull`：与桌面相同，拉取时获取租约（只读设备不获取） | `pull.plan` + `job.start` |
| 拉取全部 | 托盘“拉取最新” | `workroam pull --all` | 同上（逐项目） |
| 查看变更详情 | 授权卡片“查看详情” | `workroam push --dry-run` / `pull --dry-run`；已发起的用 `workroam approval show <id>` | `push.plan` / `pull.plan` |
| 租约冲突：等待释放 | 租约冲突卡片“等待释放” | `workroam pull --wait [<时长>]`；不加时直接以退出码 12 结束并说明持有者 | `pull.plan` |
| 租约冲突：强制接管 | “强制接管并拉取”；项目详情中的接管提示 | `workroam pull --takeover`；只接管不拉取用 `workroam lease takeover` | `lease.takeover` |
| 暂停自动同步 | 托盘“暂停自动同步 1 小时”，暂停期间变为“恢复自动同步” | `workroam pause [--for <时长>]`（默认 1 小时）/ `workroam resume` | `sync.pause` / `sync.resume` |

## 4. 授权、进度与提醒（卡片 ↔ 终端）

命令行发起的作业在终端里完成同样的交互；Agent 在非终端调用时，授权在桌面卡片上完成，命令行输出等待提示（UX-06）。

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 同意 | 授权卡片“同意” | 终端提示中选“同意”；或 `workroam approve <id>` | `approval.respond` |
| 稍后 | 授权卡片“稍后”；超时 | 终端提示中选“稍后”；或 `workroam later <id>`；超时（退出码 11） | `approval.respond` |
| 拒绝 | 授权卡片“查看详情”窗口中的“拒绝” | 终端提示中选“拒绝”；或 `workroam deny <id>`（退出码 10） | `approval.respond` |
| 本项目 1 小时内不再询问 | 授权卡片勾选项 | 终端提示中的同一选项；或 `workroam approve <id> --skip-ask 1h` | `approval.respond` |
| 待确认列表与详情 | 角落卡片（多张时堆叠）、详情窗口 | `workroam approval list` / `approval show <id>` | `approval.list` |
| 进度 | 进度卡片：阶段、已传 / 总量、速度、剩余时间、已阻止休眠 | 终端进度条，显示同样字段；非终端每约 10% 一行；`workroam job list` 查看后台作业 | `job.progress` / `job.list` |
| 取消 | 进度卡片“取消” | 前台作业按 Ctrl+C；其他作业 `workroam job cancel <id>` | `job.cancel` |
| 完成 | 完成卡片：快照编号、耗时、租约状态 | 命令结束时输出同样内容 | `job.finished` |
| 失败 | 失败卡片（常驻）：重试 / 打开检查 / 忽略；托盘警示 | 非零退出码 + 原因与修复建议；重新运行原命令即从断点续传（PUSH-07）；`workroam doctor` 打开检查；`workroam status` 显示最近一次失败 | `job.failed` |
| 到达 | 到达卡片：来源设备、时间、变更概要、冲突；稍后 / 拉取 | `workroam status` 显示“有新进度”与同样的概要；`workroam pull` | `arrival` / `status` |

## 5. 同步历史与恢复

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 历史列表 | “同步历史”：时间线、来源设备、文件与提交数、说明 | `workroam history [--limit <N>]` | `history.list` |
| 查看一次快照 | 展开快照，列出改动的文件并可勾选 | `workroam history show <快照>` | `history.show` |
| 恢复所选文件到原位置 | “恢复所选到原位置” | `workroam restore <快照> --path <文件或目录>`（可重复） | `restore.plan` |
| 导出到其他目录 | “导出到其他目录…” | `workroam restore <快照> [--path …] --to <目录>` | `restore.plan` |
| 恢复整个项目 | “恢复整个项目…”，确认框显示冲突副本位置 | `workroam restore <快照>`，确认提示显示同样内容 | `restore.plan` |

## 6. 项目归档

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 归档列表与搜索 | “归档”页：列表、搜索 | `workroam archive list [--search <关键词>]` | `archive.list` |
| 归档项目 | 项目详情“归档…”、归档页“归档项目…”：备注、保留完整历史、未推送改动提示、“推送并归档” | `workroam archive create <项目> [--note <备注>] [--keep-history]`，确认提示内容相同，有未推送改动时先推送 | `archive.plan` |
| 恢复归档：场景判定 | 恢复对话框显示检测到的场景 ①～⑦ 与将要发生的变化 | `workroam archive restore <项目> [--to <目录>]` 先输出同样的判定；`--dry-run` 只判定不执行 | `archive.restore_plan` |
| 场景 ③ / ⑤ 的选择 | ③ 保留本地改动（默认）/ 以归档为准；⑤ 合并（默认）/ 以归档为准 / 恢复到另一个目录后自己对比 | 交互模式提供同样选项；非交互 `--prefer local\|merge\|archive`；“另存对比”即 `workroam archive extract` | `archive.restore` |
| 场景 ⑥ 换目录 | “换一个目录” | 加 `--to <目录>` 重新运行（退出码 2 时提示） | `archive.restore_plan` |
| 导出到其他目录 | “导出到其他目录” | `workroam archive extract <项目> --to <目录>` | `archive.extract` |
| 删除归档 | “删除归档”，输入项目名确认 | `workroam archive delete <项目>`：交互输入项目名；非交互 `--confirm <项目名>` | `archive.delete` |
| 导出 / 导入归档包 | 归档页“导出归档包…”/“导入归档包…” | `workroam archive export <项目> <文件>` / `archive import <文件>` | `archive.export` / `archive.import` |

## 7. 设备

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 设备列表 | “设备”页：名称、系统、权限、可访问内网、最后同步、持有的租约 | `workroam device list`：同样字段 | `device.list` |
| 添加设备 | “添加设备”：配对码 + 二维码，可勾选“只读设备” | `workroam device add [--read-only] [--qr]` | `device.pairing_code` |
| 移除设备并轮换密钥 | 设备行“移除”→“移除并轮换密钥” | `workroam device remove <设备>`，确认提示内容相同 | `device.remove` |

## 8. 检查与校验

| 功能 | 桌面应用 | 命令行 | IPC |
| --- | --- | --- | --- |
| 运行全部检查 | “检查”页：分组、上次运行时间、“重新运行”；托盘“检查…” | `workroam doctor` | `doctor.run` |
| 完整性校验 | “检查”页“校验数据完整性…”（可选主存储或镜像、抽样比例） | `workroam verify [--backup] [--read-data <百分比>]` | `verify.run` |

## 9. 设置

设置项写入 `machine.toml`（本机）或项目的 `workroam.toml`（项目覆盖），桌面“设置”页与 `workroam config` 读写同一组键。

| 功能 | 桌面应用 | 命令行 | 配置键 |
| --- | --- | --- | --- |
| 列出全部设置 | “设置”页 | `workroam config list [-p <项目>]` | — |
| 设备名、默认项目位置、可达主机 | 设置“本机” | `workroam config set device.name\|root\|reachable <值>` | `name` / `root` / `reachable` |
| 开机自启 | 设置“本机”、向导本机设置 | `workroam config set autostart true\|false` | `autostart` |
| 自动同步档位（全局 / 项目覆盖） | 设置“自动同步”；项目详情中覆盖 | `workroam config set sync.auto off\|ask\|auto [-p <项目>]` | `sync.auto` |
| 空闲后推送 | 开关 + 分钟数 | `workroam config set sync.idle_minutes <分钟>`（0 为关闭） | `sync.idle_minutes` |
| 合盖或关机前推送 | 开关 | `workroam config set sync.on_sleep off\|ask\|auto` | `sync.on_sleep` |
| 开机联网后检查更新 | 开关 | `workroam config set sync.on_wake true\|false` | `sync.on_wake` |
| Agent 发起的同步需要确认 | 开关，可在项目中放宽 | `workroam config set sync.agent_requires_approval true\|false [-p <项目>]` | `sync.agent_requires_approval` |
| 通知 | 手机推送（ntfy）、同步完成时提醒 | `workroam config set notify.ntfy <地址>` / `notify.on_success true\|false` | `notify.*` |
| Agent 与适配器 | 设置“Agent”：版本、适配器版本、匹配状态 | `workroam agent list` | — |
| 接入新装的本机 Agent | 设置“Agent”→“接入” | `workroam agent connect <agent>` | — |
| 本机 MCP server 开关 | 设置“Agent”中每个 Agent 的 MCP 开关 | `workroam config set mcp.enabled true\|false [-p <项目>]` | `mcp.enabled` |
| 连接云端个人 Agent | 设置“Agent”→“连接 <Agent>”：打开对方连接器页或复制设置说明；授权卡片核对确认码、选择权限与项目 | `workroam agent connect <agent> [--relay <地址>]`：没有中继时输出给 bot 的部署说明与绑定码，再用 `--relay` 绑定 bot 回报的地址；输出连接地址与设置说明；授权在终端核对确认码并选择权限与项目，或 `workroam approve <id> --access read-only\|sync --project <项目>` | — |
| 撤销云端 Agent 连接 | 设置“Agent”→连接行“撤销” | `workroam agent disconnect <agent>` | — |
| 远程连接开关（按项目）、默认代理设备 | 项目详情“允许远程连接”；设置“Agent”→“默认代理设备” | `workroam config set remote.enabled true\|false -p <项目>`；`config set remote.default_device <设备>` | `remote.*` |
| 事件 webhook | 设置“Agent”→连接行“webhook” | `workroam agent webhook <agent> --url <地址> --secret-stdin` | 系统密钥库 |
| 更新访问密钥 | 设置“存储”→“更新访问密钥” | `workroam credentials update --access-key <ID> --secret-key-stdin` | 系统密钥库 |
| 修改加密密码 | 设置“加密”→“修改加密密码” | `workroam password change` | 仓库密钥文件 |
| 忘记密码时用恢复密钥重设 | 设置“加密”→“使用恢复密钥重设密码” | `workroam password reset --recovery-key-stdin` | 仓库密钥文件 |
| 快照保留策略与清理 | 设置“存储”：保留策略、“立即清理” | `workroam config set retention.days\|retention.weekly <值>`；`workroam prune` | `retention.*` |
| 异地备份 | 设置“异地备份”：镜像目标、最近一次结果、“立即备份”、移除 | `workroam backup add\|list\|remove\|run` | `[backup]` |
| 横幅显示 | 不适用（只影响命令行） | `workroam config set ui.banner true\|false` | `ui.banner` |

## 10. 主界面与命令行首屏

| 功能 | 桌面应用 | 命令行 |
| --- | --- | --- |
| 首次启动引导初始化 | 首次启动自动打开初始化向导 | 尚未加入工作区时运行 `workroam`：显示横幅，询问是否开始 `workroam setup` |
| 首屏概况 | 主窗口侧栏（存储、工作区、本机）与项目列表；托盘头部 | `workroam`（无参数）：横幅 + 工作区、本机与各项目状态摘要 + 常用命令 |
| 品牌标识 | 应用图标、托盘模板图标 | 启动横幅（[PRD §8.17.3](prd.md#8173-启动横幅)） |

## 11. 豁免清单

以下能力只在一侧提供，是有意的设计，不算不一致：

| 只在桌面应用 | 原因 / 命令行中的对应 |
| --- | --- |
| 托盘图标三态（正常 / 同步中 / 警示） | 展示形式；同样的状态由 `workroam status` 输出 |
| 卡片固定在屏幕角落、外观（浅色 / 深色）切换 | 展示形式 |
| 用摄像头扫描配对二维码 | 输入方式；命令行粘贴配对码文本，结果相同 |
| 打开主窗口、退出桌面应用 | 窗口管理；退出桌面应用不会停止后台服务 |

| 只在命令行 | 原因 / 桌面应用中的对应 |
| --- | --- |
| `workroam mcp` | 给本机 Agent 用的 stdio 接口，不面向人；是否加入各 Agent 的项目配置由设置“Agent”中的 MCP 开关控制 |
| `--json`、退出码、非终端输出格式 | 给脚本与 Agent 的机器接口 |
| `workroam completions <shell>`、`help`、`--version` | 命令行自身的辅助功能；版本号在桌面“关于”中显示 |
| 启动横幅 | 命令行的品牌标识，对应桌面的应用图标 |
