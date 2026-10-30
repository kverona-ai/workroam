# 桌面应用与命令行功能对照

| 项目 | 内容 |
| --- | --- |
| 版本 | 1.0 |
| 最后更新 | 2026-10-30 |
| 关联文档 | [产品需求 §8.17 命令行 · §8.22 一致性](prd.md#822-界面与命令行一致性par) · [技术架构 §17](architecture.md#17-命令行与界面一致性) · [ADR-0009](adr/0009-desktop-cli-feature-parity.md) · [高保真原型](prototype/README.md) |

**规则：** 每一项面向用户的功能，在桌面应用（向导、主窗口、卡片、托盘）和 `workroam` 命令行里都必须有入口，两边调用同一个 IPC 方法，选项、默认值、校验规则和结果完全一致（PAR-01）。只在一侧存在的能力必须列在本文 [§11 豁免清单](#11-豁免清单) 中。

本表是唯一的对照依据：新增或修改功能时同时更新桌面应用、命令行与本表；实现阶段由 `crates/workroam-ipc/parity.toml` 承载同样的内容，并由 CI 检查（[技术架构 §17.4](architecture.md#174-一致性检查)）。

表中“命令行”一列省略了全局参数：命令默认作用于当前目录所在的项目，`-p, --project <项目>` 指定其他项目；`--json`、`-q, --quiet`、`--yes` 等见 [PRD §8.17.2](prd.md#8172-全局参数与输出)。

---

## 1. 首次初始化（向导 ↔ `workroam setup`）

`workroam setup` 交互模式的提问顺序、选项与默认值和向导逐步对应；非交互模式用参数一次给全。两者都调用 `setup.*`（ADR-0005）。

| 功能 | 桌面应用 | 命令行 | 版本 |
| --- | --- | --- | --- |
| 选择路径 | 欢迎页：新建工作区 / 加入已有工作区 / 从异地备份恢复 | `workroam setup` 交互选择；`--new` / `--join` / `--from-backup` | v0.1（前两条）/ v0.3 |
| 连接存储 | 5 个预设（自建、AWS S3、R2、OSS、其他）+ 端点；自建时显示部署命令 | `--preset rustfs\|aws\|r2\|oss\|other --endpoint <地址>`；交互模式选 `rustfs` 时同样打印 `install.sh` 命令 | v0.1 |
| 自动识别地址风格与区域 | 端点下方实时显示 | 输出识别结果；`--addressing path\|virtual-host`、`--region` 可强制指定 | v0.1 |
| 访问密钥 | Access Key / Secret 输入框 | `--access-key <ID> --secret-key-stdin`（或环境变量 `WORKROAM_SECRET_KEY`）；交互模式隐藏输入 | v0.1 |
| 密钥权限方式 | 创建专用密钥（读写 / 只读）/ 使用当前密钥；不支持创建时隐藏 | `--key-mode create\|current --key-access read-write\|read-only`；存储不支持时 `create` 报参数错误并说明 | v0.1（current）/ v0.2（create） |
| 权限过宽确认 | 警告 + “我了解风险，继续使用当前密钥” | 交互模式显示同一警告并要求确认；非交互需加 `--accept-broad-key`，否则以退出码 21 结束 | v0.1 |
| 存储桶 | 默认新建（可“换一个”）/ 选择已有（列表，标注已是工作区、非空、空桶）/ 自定义（实时校验） | `--bucket auto\|<桶名>`；交互模式列出可见桶并带同样的标注，自定义时逐条显示校验结果 | v0.1 |
| 桶内前缀 | 输入框；非空且非工作区的桶必须填写 | `--prefix <前缀>`；规则相同 | v0.1 |
| 所选桶已是工作区 | 提示返回第一步改为加入 | 同样提示，并给出等价的 `workroam setup --join` 用法 | v0.1 |
| 能力检查 | 16 项逐项刷新，通过 / 警告 / 失败 + 修复建议；“重新检查” | 逐项输出同样的结果与建议；交互模式失败后选择“重新检查 / 返回修改”；非交互失败以退出码 21 结束 | v0.1 |
| 加密密码 | 两次输入 + 强度提示 | 交互模式两次隐藏输入 + 强度提示；非交互 `--password-stdin` 或 `WORKROAM_PASSWORD` | v0.1 |
| 恢复密钥 | 复制 / 下载 .txt / 打印；输入指定两组确认 | 交互模式显示后要求输入指定两组；非交互必须给 `--recovery-key-out <文件>`（写入成功即视为已保存） | v0.1 |
| 加入：配对码 | 粘贴配对码，或用摄像头扫描二维码 | 交互模式粘贴；非交互 `--pairing-code-stdin`（二维码扫描见豁免清单） | v0.1 |
| 加入：解锁 | 加密密码；“忘记密码？使用恢复密钥” | 加密密码；`--recovery-key-stdin` 改用恢复密钥 | v0.1 |
| 本机设置 | 设备名、默认项目位置、可达主机（自动探测 + 连通性）、自动同步档位、开机自启、检测到的 Agent | `--device-name --root --reachable <主机,…> --auto off\|ask\|auto --autostart\|--no-autostart`；交互模式显示探测到的可达主机与 Agent 列表 | v0.1 |
| 加入：选择本机项目与目录 | 勾选项目，每个项目选择本机目录 | 交互模式逐项选择；非交互 `--project <项目>[=<目录>]`（可重复）或 `--all-projects` | v0.1 |
| 从异地备份恢复后重新加入 | 加入时选择“作为原设备 <名称>” | `workroam setup --join --as-device <设备名>` | v0.3 |
| 完成 | 配对码文本 + 二维码；“接入第一个项目” | 打印配对码；`--qr` 在终端用字符画显示二维码；最后提示 `workroam init` | v0.1 |

## 2. 项目

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 项目列表与状态 | 主窗口“项目”、托盘菜单 | `workroam project list`：工作区内全部项目，含“未添加到本机”的项目，列出本机目录与状态 | `project.list` | v0.1 |
| 项目概况 | 项目详情：租约、最新快照、待推送改动、代码仓库与本机能否推送 | `workroam status`：同样的字段 | `status` | v0.1 |
| 交接文档 | 项目详情“交接文档”区 | `workroam handoff`：显示 `HANDOFF.md` 的结构化摘要；`--template` 输出模板 | `handoff.read` | v0.1 |
| 接入项目 | “接入项目”对话框：选择目录、目录检查、发现的仓库与推荐模式、导入已有 Agent 配置、跨平台检查、“接入并推送” | `workroam init [<目录>]`：同样的步骤与输出，最后发起第一次推送（授权时可选“稍后”） | `project.scan` / `project.init` | v0.1 |
| 预览扫描结果 | 对话框中实时显示 | `workroam init --dry-run`：只扫描并报告，不写任何文件 | `project.scan` | v0.1 |
| 修改推荐的同步模式 | 仓库行中修改模式与历史档位 | 交互模式逐个确认；非交互 `--repo <路径>=<模式>[:<历史档位>]`（可重复） | `project.init` | v0.1 |
| 在本机添加已有项目 | 项目详情（未添加到本机）：选择本机目录 →“添加并拉取”；加入向导中勾选 | `workroam project add <项目> [--path <目录>]`：添加后立即发起拉取 | `project.add` | v0.1 |
| 更换本机目录 | 项目详情“本机目录 → 更换…”，只在目录为空、尚未同步时显示 | `workroam project move <项目> --path <目录>`；已同步时退出码 18 | `project.move` | v0.1 |
| 在本机移除项目 | 项目详情“在本机移除…” | `workroam project remove <项目>` | `project.remove` | v0.1 |
| 目录不存在：重新检查 | “目录已放回原处，重新检查” | `workroam status` 与 `workroam project list` 每次运行都重新检查绑定 | `status` | v0.1 |
| 目录不存在：移除后重新添加 | “移除后重新添加…” | `workroam project remove` 后 `workroam project add --path <新目录>` | `project.remove` / `project.add` | v0.1 |

## 3. 推送、拉取与租约

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 推送 | 托盘每个项目的“推送” | `workroam push` | `push.plan` + `job.start` | v0.1 |
| 推送并释放租约 | 项目详情“推送并释放” | `workroam push --release` | 同上 | v0.1 |
| 推送全部并释放 | 托盘“推送全部并释放” | `workroam push --all --release` | 同上（逐项目） | v0.1 |
| 拉取（获取租约） | 项目详情“拉取…”、托盘每个项目的“拉取”、到达卡片“拉取” | `workroam pull`：与桌面相同，拉取时获取租约（只读设备不获取） | `pull.plan` + `job.start` | v0.1 |
| 拉取全部 | 托盘“拉取最新” | `workroam pull --all` | 同上（逐项目） | v0.1 |
| 查看变更详情 | 授权卡片“查看详情” | `workroam push --dry-run` / `pull --dry-run`；已发起的用 `workroam approval show <id>` | `push.plan` / `pull.plan` | v0.1 |
| 租约冲突：等待释放 | 租约冲突卡片“等待释放” | `workroam pull --wait [<时长>]`；不加时直接以退出码 12 结束并说明持有者 | `pull.plan` | v0.1 |
| 租约冲突：强制接管 | “强制接管并拉取”；项目详情中的接管提示 | `workroam pull --takeover`；只接管不拉取用 `workroam lease takeover` | `lease.takeover` | v0.1 |
| 暂停自动同步 | 托盘“暂停自动同步 1 小时”，暂停期间变为“恢复自动同步” | `workroam pause [--for <时长>]`（默认 1 小时）/ `workroam resume` | `sync.pause` / `sync.resume` | v0.3 |

## 4. 授权、进度与提醒（卡片 ↔ 终端）

命令行发起的作业在终端里完成同样的交互；Agent 在非终端调用时，授权在桌面卡片上完成，命令行输出等待提示（UX-06）。

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 同意 | 授权卡片“同意” | 终端提示中选“同意”；或 `workroam approve <id>` | `approval.respond` | v0.1 |
| 稍后 | 授权卡片“稍后”；超时 | 终端提示中选“稍后”；或 `workroam later <id>`；超时（退出码 11） | `approval.respond` | v0.1 |
| 拒绝 | 授权卡片“查看详情”窗口中的“拒绝” | 终端提示中选“拒绝”；或 `workroam deny <id>`（退出码 10） | `approval.respond` | v0.1 |
| 本项目 1 小时内不再询问 | 授权卡片勾选项 | 终端提示中的同一选项；或 `workroam approve <id> --skip-ask 1h` | `approval.respond` | v0.1 |
| 待确认列表与详情 | 角落卡片（多张时堆叠）、详情窗口 | `workroam approval list` / `approval show <id>` | `approval.list` | v0.1 |
| 进度 | 进度卡片：阶段、已传 / 总量、速度、剩余时间、已阻止休眠 | 终端进度条，显示同样字段；非终端每约 10% 一行；`workroam job list` 查看后台作业 | `job.progress` / `job.list` | v0.1 |
| 取消 | 进度卡片“取消” | 前台作业按 Ctrl+C；其他作业 `workroam job cancel <id>` | `job.cancel` | v0.1 |
| 完成 | 完成卡片：快照编号、耗时、租约状态 | 命令结束时输出同样内容 | `job.finished` | v0.1 |
| 失败 | 失败卡片（常驻）：重试 / 打开检查 / 忽略；托盘警示 | 非零退出码 + 原因与修复建议；重新运行原命令即从断点续传（PUSH-07）；`workroam doctor` 打开检查；`workroam status` 显示最近一次失败 | `job.failed` | v0.1 |
| 到达 | 到达卡片：来源设备、时间、变更概要、冲突；稍后 / 拉取 | `workroam status` 显示“有新进度”与同样的概要；`workroam pull` | `arrival` / `status` | v0.3（卡片）/ v0.1（status） |

## 5. 同步历史与恢复

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 历史列表 | “同步历史”：时间线、来源设备、文件与提交数、说明 | `workroam history [--limit <N>]` | `history.list` | v0.1 |
| 查看一次快照 | 展开快照，列出改动的文件并可勾选 | `workroam history show <快照>` | `history.show` | v0.1 |
| 恢复所选文件到原位置 | “恢复所选到原位置” | `workroam restore <快照> --path <文件或目录>`（可重复） | `restore.plan` | v0.1 |
| 导出到其他目录 | “导出到其他目录…” | `workroam restore <快照> [--path …] --to <目录>` | `restore.plan` | v0.1 |
| 恢复整个项目 | “恢复整个项目…”，确认框显示冲突副本位置 | `workroam restore <快照>`，确认提示显示同样内容 | `restore.plan` | v0.1 |

## 6. 项目归档

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 归档列表与搜索 | “归档”页：列表、搜索 | `workroam archive list [--search <关键词>]` | `archive.list` | v0.2 |
| 归档项目 | 项目详情“归档…”、归档页“归档项目…”：备注、保留完整历史、未推送改动提示、“推送并归档” | `workroam archive create <项目> [--note <备注>] [--keep-history]`，确认提示内容相同，有未推送改动时先推送 | `archive.plan` | v0.2 |
| 恢复归档：场景判定 | 恢复对话框显示检测到的场景 ①～⑦ 与将要发生的变化 | `workroam archive restore <项目> [--to <目录>]` 先输出同样的判定；`--dry-run` 只判定不执行 | `archive.restore_plan` | v0.2 |
| 场景 ③ / ⑤ 的选择 | ③ 保留本地改动（默认）/ 以归档为准；⑤ 合并（默认）/ 以归档为准 / 恢复到另一个目录后自己对比 | 交互模式提供同样选项；非交互 `--prefer local\|merge\|archive`；“另存对比”即 `workroam archive extract` | `archive.restore` | v0.2 |
| 场景 ⑥ 换目录 | “换一个目录” | 加 `--to <目录>` 重新运行（退出码 2 时提示） | `archive.restore_plan` | v0.2 |
| 导出到其他目录 | “导出到其他目录” | `workroam archive extract <项目> --to <目录>` | `archive.extract` | v0.2 |
| 删除归档 | “删除归档”，输入项目名确认 | `workroam archive delete <项目>`：交互输入项目名；非交互 `--confirm <项目名>` | `archive.delete` | v0.2 |
| 导出 / 导入归档包 | 归档页“导出归档包…”/“导入归档包…” | `workroam archive export <项目> <文件>` / `archive import <文件>` | `archive.export` / `archive.import` | v0.3 |

## 7. 设备

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 设备列表 | “设备”页：名称、系统、权限、可访问内网、最后同步、持有的租约 | `workroam device list`：同样字段 | `device.list` | v0.1 |
| 添加设备 | “添加设备”：配对码 + 二维码，可勾选“只读设备” | `workroam device add [--read-only] [--qr]` | `device.pairing_code` | v0.1（只读 v0.2） |
| 移除设备并轮换密钥 | 设备行“移除”→“移除并轮换密钥” | `workroam device remove <设备>`，确认提示内容相同 | `device.remove` | v0.2 |

## 8. 检查与校验

| 功能 | 桌面应用 | 命令行 | IPC | 版本 |
| --- | --- | --- | --- | --- |
| 运行全部检查 | “检查”页：分组、上次运行时间、“重新运行”；托盘“检查…” | `workroam doctor` | `doctor.run` | v0.1 |
| 完整性校验 | “检查”页“校验数据完整性…”（可选主存储或镜像、抽样比例） | `workroam verify [--backup] [--read-data <百分比>]` | `verify.run` | v0.3 |

## 9. 设置

设置项写入 `machine.toml`（本机）或项目的 `workroam.toml`（项目覆盖），桌面“设置”页与 `workroam config` 读写同一组键。

| 功能 | 桌面应用 | 命令行 | 配置键 | 版本 |
| --- | --- | --- | --- | --- |
| 列出全部设置 | “设置”页 | `workroam config list [-p <项目>]` | — | v0.1 |
| 设备名、默认项目位置、可达主机 | 设置“本机” | `workroam config set device.name\|root\|reachable <值>` | `name` / `root` / `reachable` | v0.1 |
| 开机自启 | 设置“本机”、向导本机设置 | `workroam config set autostart true\|false` | `autostart` | v0.1 |
| 自动同步档位（全局 / 项目覆盖） | 设置“自动同步”；项目详情中覆盖 | `workroam config set sync.auto off\|ask\|auto [-p <项目>]` | `sync.auto` | v0.3 |
| 空闲后推送 | 开关 + 分钟数 | `workroam config set sync.idle_minutes <分钟>`（0 为关闭） | `sync.idle_minutes` | v0.3 |
| 合盖或关机前推送 | 开关 | `workroam config set sync.on_sleep off\|ask\|auto` | `sync.on_sleep` | v0.3 |
| 开机联网后检查更新 | 开关 | `workroam config set sync.on_wake true\|false` | `sync.on_wake` | v0.3 |
| Agent 发起的同步需要确认 | 开关，可在项目中放宽 | `workroam config set sync.agent_requires_approval true\|false [-p <项目>]` | `sync.agent_requires_approval` | v0.1 |
| 通知 | 手机推送（ntfy）、同步完成时提醒 | `workroam config set notify.ntfy <地址>` / `notify.on_success true\|false` | `notify.*` | v0.1（完成提醒）/ v0.3（ntfy） |
| Agent 与适配器 | 设置“Agent”：版本、适配器版本、匹配状态 | `workroam agent list` | — | v0.1 |
| 接入新装的本机 Agent | 设置“Agent”→“接入” | `workroam agent connect <agent>` | — | v0.1 |
| 本机 MCP server 开关 | 设置“Agent”中每个 Agent 的 MCP 开关 | `workroam config set mcp.enabled true\|false [-p <项目>]` | `mcp.enabled` | v0.3 |
| 连接云端个人 Agent | 设置“Agent”→“连接 <Agent>”：打开对方连接器页或复制设置说明；授权卡片核对确认码、选择权限与项目 | `workroam agent connect <agent>`：输出连接地址与设置说明；授权在终端核对确认码并选择权限与项目，或 `workroam approve <id> --access read-only\|sync --project <项目>` | — | v0.3 |
| 撤销云端 Agent 连接 | 设置“Agent”→连接行“撤销” | `workroam agent disconnect <agent>` | — | v0.3 |
| 远程连接开关（按项目）、默认代理设备 | 项目详情“允许远程连接”；设置“Agent”→“默认代理设备” | `workroam config set remote.enabled true\|false -p <项目>`；`config set remote.default_device <设备>` | `remote.*` | v0.3 |
| 事件 webhook | 设置“Agent”→连接行“webhook” | `workroam agent webhook <agent> --url <地址> --secret-stdin` | 系统密钥库 | v0.3 |
| 更新访问密钥 | 设置“存储”→“更新访问密钥” | `workroam credentials update --access-key <ID> --secret-key-stdin` | 系统密钥库 | v0.1 |
| 修改加密密码 | 设置“加密”→“修改加密密码” | `workroam password change` | 仓库密钥文件 | v0.1 |
| 忘记密码时用恢复密钥重设 | 设置“加密”→“使用恢复密钥重设密码” | `workroam password reset --recovery-key-stdin` | 仓库密钥文件 | v0.1 |
| 快照保留策略与清理 | 设置“存储”：保留策略、“立即清理” | `workroam config set retention.days\|retention.weekly <值>`；`workroam prune` | `retention.*` | v0.2 |
| 异地备份 | 设置“异地备份”：镜像目标、最近一次结果、“立即备份”、移除 | `workroam backup add\|list\|remove\|run` | `[backup]` | v0.3 |
| 横幅显示 | 不适用（只影响命令行） | `workroam config set ui.banner true\|false` | `ui.banner` | v0.1 |

## 10. 主界面与命令行首屏

| 功能 | 桌面应用 | 命令行 | 版本 |
| --- | --- | --- | --- |
| 首次启动引导初始化 | 首次启动自动打开初始化向导 | 尚未加入工作区时运行 `workroam`：显示横幅，询问是否开始 `workroam setup` | v0.1 |
| 首屏概况 | 主窗口侧栏（存储、工作区、本机）与项目列表；托盘头部 | `workroam`（无参数）：横幅 + 工作区、本机与各项目状态摘要 + 常用命令 | v0.1 |
| 品牌标识 | 应用图标、托盘模板图标 | 启动横幅（[PRD §8.17.3](prd.md#8173-启动横幅)） | v0.1 |

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

## 12. 本次核对修正的不一致

2026-10-30 逐项核对 PRD、技术架构与高保真原型后，修正了以下问题（文档已同步更新）：

| # | 问题 | 修正 |
| --- | --- | --- |
| 1 | 加入工作区只有桌面入口，`workroam setup` 只写了 `--new` | 补齐 `--join`、`--pairing-code-stdin`、`--recovery-key-stdin`、`--project <项目>[=<目录>]` |
| 2 | 向导中的存储预设、权限过宽确认、可达主机、自动同步、开机自启在命令行没有参数 | 补齐 `--preset`、`--accept-broad-key`、`--reachable`、`--auto`、`--autostart` |
| 3 | 完成页的二维码、设备页的只读配对码只在桌面 | `--qr`、`device add --read-only` |
| 4 | 托盘的“推送全部并释放”“拉取最新”“暂停自动同步 1 小时”没有命令 | `push --all --release`、`pull --all`、`pause` / `resume` |
| 5 | 桌面“拉取”总是获取租约，命令行却写成 `pull [--acquire]`，两边默认行为不同 | `workroam pull` 默认获取租约，去掉 `--acquire` |
| 6 | 租约冲突卡片的“等待释放”“强制接管并拉取”没有对应参数 | `pull --wait`、`pull --takeover` |
| 7 | 授权卡片有“稍后”，命令行只有 `approve` / `deny`；卡片没有“拒绝” | 命令行增加 `later`、`approval list/show`、`--skip-ask`；桌面在“查看详情”窗口提供“拒绝” |
| 8 | 进度卡片可以取消，命令行无法取消 Agent 在后台发起的作业 | `job list` / `job cancel` |
| 9 | 项目详情展示 HANDOFF.md，命令行没有对应命令（MCP 有 `handoff_template`） | `workroam handoff [--template]` |
| 10 | 同步历史可以展开快照勾选文件，命令行无法列出某次快照的文件 | `workroam history show <快照>` |
| 11 | 桌面同步历史 v0.3、桌面归档页 v0.3，命令行分别在 v0.1、v0.2 交付 | 桌面同步历史提前到 v0.1，桌面归档页提前到 v0.2，与命令行同版本 |
| 12 | 恢复归档的场景选择、删除归档的项目名确认只在桌面 | `archive restore --dry-run --prefer`、`archive delete --confirm` |
| 13 | 设置页的全部选项在命令行没有入口 | `workroam config list/get/set`，与设置页读写同一组键 |
| 14 | 云端 Agent 连接的权限与项目范围、按项目的远程连接开关、默认代理设备、webhook、本机 MCP 开关只在桌面描述 | `agent connect` 的终端授权与 `approve --access --project`、`config set remote.* / mcp.enabled`、`agent webhook` |
| 15 | PRD 7.5 写了“用恢复密钥解锁并设置新密码”，两边都没有入口 | `password change` / `password reset`；设置“加密” |
| 16 | 存储密钥失效后只能“检查”，两边都没有更新密钥的入口 | `credentials update`；设置“存储”→“更新访问密钥” |
| 17 | `prune`、`verify`、归档包导出导入只有命令行 | 设置“存储”保留策略与“立即清理”、检查页“校验数据完整性…”、归档页“导出 / 导入归档包” |
| 18 | `project move` 在桌面没有入口 | 项目详情“本机目录 → 更换…”（仅 Pending 状态） |
| 19 | `project list` 没有说明是否包含未添加到本机的项目，与桌面列表不一致 | 明确包含，状态为“未添加到本机” |
| 20 | `init` 接入时无法预览、无法修改推荐的同步模式 | `init --dry-run`、`init --repo` |
| 21 | 桌面首次启动自动打开向导，命令行没有对应的首屏 | `workroam` 无参数时显示横幅；未初始化时引导 `setup` |
| 22 | 从异地备份恢复后“作为原设备重新加入”只在 PRD 描述 | 桌面加入时选择设备；`setup --join --as-device` |

## 13. 高保真原型

本表要求的桌面入口已全部补进原型（`docs/prototype/index.html`，2026-10-30）：授权详情窗口（变更文件与“拒绝 / 稍后 / 同意”）、托盘的待确认数与“恢复自动同步”、Pending 项目的“更换…”目录与项目级同步覆盖、设置中的“本机 / 存储 / 加密”与异地备份操作、检查页“校验数据完整性…”、归档页搜索与归档包导入导出、加入向导中“作为原设备”加入（v0.3）。
