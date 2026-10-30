//! Bilingual (zh / en) text shared by the desktop app and the CLI (PAR-04).
//!
//! The daemon returns text keys in IPC results; both clients resolve them
//! with [`tr`], so options, hints and confirmations read the same everywhere.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0); // 0 = unset, 1 = zh, 2 = en

pub fn set_lang(l: Lang) {
    LANG.store(if l == Lang::Zh { 1 } else { 2 }, Ordering::Relaxed);
}

/// Resolve the language from an explicit setting (`zh`, `en`, empty = system).
pub fn resolve_lang(setting: &str) -> Lang {
    match setting {
        "zh" | "zh-CN" | "zh_CN" => return Lang::Zh,
        "en" | "en-US" | "en_US" => return Lang::En,
        _ => {}
    }
    for var in ["WORKROAM_LANG", "LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            if v.is_empty() {
                continue;
            }
            return if v.to_lowercase().starts_with("zh") {
                Lang::Zh
            } else {
                Lang::En
            };
        }
    }
    #[cfg(windows)]
    {
        // Windows rarely sets LANG; fall back to Chinese only when asked.
        return Lang::En;
    }
    #[allow(unreachable_code)]
    Lang::En
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Zh,
        2 => Lang::En,
        _ => {
            let l = resolve_lang("");
            set_lang(l);
            l
        }
    }
}

/// Translate a key; unknown keys are returned unchanged.
pub fn tr(key: &str) -> String {
    tr_lang(key, lang())
}

pub fn tr_lang(key: &str, l: Lang) -> String {
    for (k, zh, en) in TEXTS {
        if *k == key {
            return if l == Lang::Zh {
                (*zh).to_string()
            } else {
                (*en).to_string()
            };
        }
    }
    key.to_string()
}

/// Translate and substitute `{name}` placeholders.
pub fn trf(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = tr(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

pub fn all_keys() -> impl Iterator<Item = &'static str> {
    TEXTS.iter().map(|(k, _, _)| *k)
}

/// (key, 中文, English)
pub static TEXTS: &[(&str, &str, &str)] = &[
    ("tagline", "带着 AI Agent 工作区，在每台电脑之间漫游", "Hand off your AI agent workspace between machines."),
    // setup paths
    ("setup.path", "这是第一台设备吗？", "Is this your first device?"),
    ("setup.path.new", "新建工作区", "Create a new workspace"),
    ("setup.path.join", "加入已有工作区", "Join an existing workspace"),
    ("setup.path.backup", "存储已丢失，从异地备份恢复", "Storage lost: restore from an off-site backup"),
    ("setup.preset", "选择存储", "Choose your storage"),
    ("preset.rustfs", "自建（install.sh 部署）", "Self-hosted (install.sh)"),
    ("preset.aws", "AWS S3", "AWS S3"),
    ("preset.r2", "Cloudflare R2", "Cloudflare R2"),
    ("preset.oss", "阿里云 OSS", "Alibaba Cloud OSS"),
    ("preset.other", "其他 S3 兼容", "Other S3 compatible"),
    ("preset.local", "本地或挂载目录", "Local or mounted directory"),
    ("setup.endpoint", "存储端点地址", "Storage endpoint"),
    ("setup.rustfs_hint", "还没有部署？在服务器上运行：curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh | sh -s -- --domain <域名> --storage rustfs", "Not deployed yet? On your server run: curl -fsSL https://raw.githubusercontent.com/kverona-ai/workroam/main/deploy/install.sh | sh -s -- --domain <domain> --storage rustfs"),
    ("setup.access_key", "Access Key", "Access Key"),
    ("setup.secret_key", "Secret Key（只保存到系统密钥库）", "Secret Key (stored in the system keychain only)"),
    ("setup.key_mode", "密钥权限方式", "Key permissions"),
    ("key_mode.create", "由 Workroam 创建专用密钥（推荐）：管理员密钥用完即丢弃", "Let Workroam create a scoped key (recommended); the admin key is discarded"),
    ("key_mode.current", "使用当前密钥", "Use the current key"),
    ("key_access.read-write", "读写", "Read-write"),
    ("key_access.read-only", "只读", "Read-only"),
    ("setup.broad_key", "当前密钥权限过宽：它能访问工作区之外的桶。建议收窄权限，或改用“创建专用密钥”。", "This key is too broad: it can access buckets outside the workspace. Narrow it, or let Workroam create a scoped key."),
    ("setup.broad_key.confirm", "我了解风险，继续使用当前密钥", "I understand the risk, keep using this key"),
    ("setup.bucket", "存储桶", "Bucket"),
    ("bucket.auto", "默认新建", "Create a new bucket"),
    ("bucket.existing", "选择已有桶", "Use an existing bucket"),
    ("bucket.custom", "自定义桶名", "Custom bucket name"),
    ("bucket.is_workspace", "已是 Workroam 工作区", "already a Workroam workspace"),
    ("bucket.non_empty", "非空", "not empty"),
    ("bucket.empty", "空桶", "empty"),
    ("bucket.workspace_found", "所选位置已经是一个 Workroam 工作区，请改为“加入已有工作区”（workroam setup --join）。", "This location already holds a Workroam workspace; join it instead (workroam setup --join)."),
    ("bucket.needs_prefix", "这个桶不是空的，也不是工作区：请设置桶内前缀，或换一个桶。", "This bucket is not empty and not a workspace: set a prefix or pick another bucket."),
    ("bucket.invalid", "桶名需为 3–63 位小写字母、数字或连字符，不能以连字符开头或结尾，不能是 IP 格式", "Bucket names are 3-63 lowercase letters, digits or hyphens, not starting or ending with a hyphen, not an IP address"),
    ("setup.prefix", "桶内前缀（可选）", "Prefix inside the bucket (optional)"),
    ("setup.checks", "能力检查", "Capability checks"),
    ("setup.checks.retry", "重新检查", "Run checks again"),
    ("setup.checks.back", "返回修改", "Go back and change"),
    ("setup.password", "设置加密密码", "Set an encryption password"),
    ("setup.password.again", "再次输入加密密码", "Repeat the encryption password"),
    ("setup.password.mismatch", "两次输入不一致", "The passwords do not match"),
    ("password.too_short", "太短：至少 8 个字符", "Too short: at least 8 characters"),
    ("password.weak", "弱", "Weak"),
    ("password.fair", "一般", "Fair"),
    ("password.good", "较强", "Good"),
    ("password.strong", "强", "Strong"),
    ("setup.recovery", "恢复密钥：忘记加密密码时用它解锁工作区。请下载、打印或复制保存。", "Recovery key: unlocks the workspace if you forget the password. Download, print or copy it."),
    ("setup.recovery.confirm", "请输入恢复密钥的第 {a} 组和第 {b} 组以确认已保存", "Type groups {a} and {b} of the recovery key to confirm you saved it"),
    ("setup.recovery.wrong", "输入不正确，请核对恢复密钥", "That does not match; check the recovery key"),
    ("setup.device_name", "设备名", "Device name"),
    ("setup.root", "默认项目位置", "Default project location"),
    ("setup.reachable", "可达主机（逗号分隔）", "Reachable hosts (comma separated)"),
    ("setup.auto", "自动同步档位", "Auto sync"),
    ("auto.off", "关闭", "Off"),
    ("auto.ask", "询问（默认）", "Ask (default)"),
    ("auto.auto", "全自动", "Automatic"),
    ("setup.autostart", "开机自启", "Start at login"),
    ("setup.agents", "检测到的 Agent", "Detected agents"),
    ("setup.pairing", "配对码", "Pairing code"),
    ("setup.pairing.paste", "粘贴配对码", "Paste the pairing code"),
    ("setup.unlock", "输入加密密码", "Enter the encryption password"),
    ("setup.unlock.forgot", "忘记密码？使用恢复密钥", "Forgot it? Use the recovery key"),
    ("setup.done", "完成！在其他电脑上用配对码加入这个工作区，然后接入第一个项目：workroam init", "Done! Use the pairing code on your other computers, then connect your first project: workroam init"),
    ("setup.projects", "选择本机需要的项目", "Choose the projects you need on this device"),
    ("setup.as_device", "作为原设备重新加入", "Rejoin as the original device"),
    // checks
    ("check.dns", "DNS 解析", "DNS resolution"),
    ("check.tls", "TLS 证书", "TLS certificate"),
    ("check.credentials", "凭证有效", "Credentials valid"),
    ("check.clock", "时钟偏差", "Clock skew"),
    ("check.bucket", "桶可访问", "Bucket reachable"),
    ("check.rwd", "读 / 写 / 删除", "Read / write / delete"),
    ("check.cond_create", "条件创建", "Conditional create"),
    ("check.cond_overwrite", "条件覆盖", "Conditional overwrite"),
    ("check.multipart", "分片上传", "Multipart upload"),
    ("check.addressing", "地址风格", "Addressing style"),
    ("check.key_scope", "密钥权限范围", "Key scope"),
    ("check.versioning", "版本控制与生命周期", "Versioning and lifecycle"),
    ("check.speed", "延迟与带宽", "Latency and bandwidth"),
    ("check.git", "本机 git", "Local git"),
    ("check.longpaths", "长路径（Windows）", "Long paths (Windows)"),
    ("check.agents", "Agent 检测", "Agent detection"),
    ("check.autocrlf", "core.autocrlf", "core.autocrlf"),
    ("check.adapters", "Agent 版本与适配器", "Agent versions and adapters"),
    ("check.backup", "异地备份", "Off-site backup"),
    ("check.daemon", "后台服务", "Background service"),
    ("hint.dns", "检查域名与 DNS 记录", "Check the domain name and its DNS records"),
    ("hint.tls", "自建时检查 Caddy 是否已签发证书", "If self-hosted, check that Caddy has issued a certificate"),
    ("hint.credentials", "核对 Access Key 与 Secret", "Check the Access Key and Secret"),
    ("hint.clock", "开启系统时间同步", "Turn on automatic time sync"),
    ("hint.bucket", "检查桶名与区域", "Check the bucket name and region"),
    ("hint.rwd", "检查密钥权限范围", "Check the key's permissions"),
    ("hint.conditional", "将以乐观并发方式运行：不使用租约，分叉在下次拉取时交给你处理", "Workroam will run with optimistic concurrency: no leases, forks are resolved on the next pull"),
    ("hint.multipart", "将限制单块大小", "Pack size will be limited"),
    ("hint.addressing", "检查反向代理配置", "Check the reverse proxy configuration"),
    ("hint.key_scope", "收窄权限或改用专用密钥", "Narrow the key or let Workroam create a scoped key"),
    ("hint.versioning", "生命周期规则可能删除快照数据，建议关闭或排除工作区前缀", "Lifecycle rules may delete snapshot data; disable them or exclude the workspace prefix"),
    ("hint.git", "升级 git 到 2.30 或更新版本", "Upgrade git to 2.30 or newer"),
    ("hint.longpaths", "以管理员身份运行：reg add HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem /v LongPathsEnabled /t REG_DWORD /d 1 /f，并执行 git config --global core.longpaths true", "Run as administrator: reg add HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem /v LongPathsEnabled /t REG_DWORD /d 1 /f, then git config --global core.longpaths true"),
    ("hint.autocrlf", "执行 git config --global core.autocrlf false，并在仓库中使用 .gitattributes（* text=auto eol=lf）", "Run git config --global core.autocrlf false and use .gitattributes (* text=auto eol=lf)"),
    ("hint.agents", "版本不在适配器支持范围时，部分配置可能无法生成", "When a version is outside the adapter's range some config may not be generated"),
    ("hint.backup", "核对镜像凭证，或连接移动硬盘 / NAS 后立即备份", "Check the mirror credentials, or connect the drive / NAS and back up now"),
    ("hint.daemon", "运行 workroamd 或重新登录以启动后台服务", "Start workroamd or log in again to start the background service"),
    // approvals
    ("approval.push", "推送 {project}", "Push {project}"),
    ("approval.pull", "拉取 {project}", "Pull {project}"),
    ("approval.restore", "恢复 {project}", "Restore {project}"),
    ("approval.archive", "归档 {project}", "Archive {project}"),
    ("approval.archive_restore", "恢复归档 {project}", "Restore archive {project}"),
    ("approval.connect", "{agent} 请求连接", "{agent} wants to connect"),
    ("approval.approve", "同意", "Approve"),
    ("approval.later", "稍后", "Later"),
    ("approval.deny", "拒绝", "Deny"),
    ("approval.details", "查看详情", "View details"),
    ("approval.skip_ask", "本项目 1 小时内不再询问", "Don't ask again for this project for 1 hour"),
    ("approval.waiting", "等待在 Workroam 卡片上确认…（{timeout} 后超时）", "Waiting for confirmation on the Workroam card… (times out in {timeout})"),
    ("approval.handoff_stale", "HANDOFF.md 自上次推送以来没有更新", "HANDOFF.md has not been updated since the last push"),
    ("approval.touches_local", "拉取会覆盖或合并本地尚未同步的改动", "This pull touches local changes that are not synced yet"),
    // misc
    ("lease.held_by", "租约被 {device} 持有（最后活动 {time}）", "The lease is held by {device} (last active {time})"),
    ("lease.stale", "可能已遗忘：超过 12 小时未续约", "Possibly forgotten: not renewed for more than 12 hours"),
    ("lease.wait", "等待释放", "Wait for release"),
    ("lease.takeover", "强制接管并拉取", "Take over and pull"),
    ("project.missing", "项目目录不存在", "Project directory is missing"),
    ("project.locked", "项目目录已同步，不能更换：请先在本机移除项目，再重新添加并选择新目录", "The directory is already synced and cannot change: remove the project on this device, then add it again with the new directory"),
    ("archive.confirm_name", "输入项目名以确认删除归档", "Type the project name to confirm deleting the archive"),
    ("device.remove.confirm", "移除设备 {device} 并轮换密钥？被移除的设备将失去访问权限。", "Remove {device} and rotate the access key? The removed device loses access."),
    ("remote.warning", "返回给云端 Agent 的状态摘要和 HANDOFF.md 会进入服务商的云端，不再受端到端加密保护。", "Status summaries and HANDOFF.md returned to cloud agents leave the end-to-end encryption boundary."),
    ("daemon.unavailable", "后台服务不可用", "The background service is not available"),
    ("not_initialized", "这台电脑还没有加入 Workroam 工作区。", "This computer has not joined a Workroam workspace yet."),
    ("start_setup", "现在开始初始化（workroam setup）？", "Start setup now (workroam setup)?"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<_> = all_keys().collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(n, keys.len(), "duplicate text keys");
    }

    #[test]
    fn translate() {
        assert_eq!(tr_lang("approval.approve", Lang::Zh), "同意");
        assert_eq!(tr_lang("approval.approve", Lang::En), "Approve");
        assert_eq!(tr_lang("no.such.key", Lang::En), "no.such.key");
    }
}
