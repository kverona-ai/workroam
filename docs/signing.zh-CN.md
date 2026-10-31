# 代码签名

[English](signing.md) · **简体中文**

本文面向维护者：发布流水线如何给三个平台的产物签名，每个 GitHub Secret 怎么获取，以及配好之后如何验证。发布流程本身见 [release.md](release.md)。

所有签名都是可选的：某个平台没有配置时照常发布，只是那个平台的文件不签名。

> **私钥、证书和密码只放进 GitHub Secrets。** 在 [Settings → Secrets and variables → Actions](https://github.com/kverona-ai/workroam/settings/secrets/actions) 的 **Secrets** 页添加；不是秘密的配置（如 SignPath 组织 ID）放在同一页面的 **Variables** 页。不要把它们发到 issue、PR、聊天或提交进仓库。

## 总览

| 平台 | 签什么 | 方式 | 需要 |
| --- | --- | --- | --- |
| macOS | 桌面应用 `.app` 与 `.dmg`，命令行 `workroam`、`workroamd`，`workroam-server` | Developer ID 签名 + Apple 公证（notarization），`.app` 与 `.dmg` 装订公证票据 | Apple Developer Program 账号 |
| Windows | 所有 `.exe`（含桌面应用内的随附二进制），`.msi` 与 `-setup.exe` 安装程序 | [SignPath Foundation](https://signpath.org) 为开源项目免费提供的证书，通过 SignPath 签名 | SignPath 开源项目申请通过 |
| Linux | `SHA256SUMS`（它列出了所有文件的校验和） | GPG 分离签名 `SHA256SUMS.asc`，同时附上公钥 `workroam-release-key.asc` | 一把本地生成的 GPG 密钥 |

另外，每次正式发布都会为所有文件生成 GitHub 构建来源证明（SLSA provenance），不需要配置。

每次运行 Release 工作流时，Prepare 任务的摘要里有一张 **签名状态** 表，显示哪些签名已配置。

## macOS

需要 Apple Developer Program 账号（个人或公司）。创建 Developer ID 证书需要账号持有人（Account Holder）权限。

### 1. 申请 Developer ID Application 证书

1. 在 Mac 上打开“钥匙串访问”，菜单选 **钥匙串访问 → 证书助理 → 从证书颁发机构请求证书**。填开发者账号邮箱和一个常用名称（如公司名），CA 邮箱留空，选“存储到磁盘”，得到 `.certSigningRequest` 文件。私钥留在这台 Mac 上，之后也要在这台 Mac 上导出 `.p12`；
2. 打开 [Certificates](https://developer.apple.com/account/resources/certificates/add)，选 **Developer ID Application**（不是 Developer ID Installer，也不是 Apple Development），中间证书选 **G2 Sub-CA**（旧的 Previous Sub-CA 于 2027 年 2 月 1 日到期），上传上一步的文件；
3. 下载 `.cer`，双击安装到“登录”钥匙串。

### 2. 导出 .p12

在“钥匙串访问”的 **我的证书** 里找到 `Developer ID Application: …`，展开能看到它下面的私钥。右键证书 → 导出，格式选 `.p12`，设置一个密码。然后：

```sh
base64 -i DeveloperID.p12 | pbcopy        # 复制到剪贴板
security find-identity -v -p codesigning  # 查看签名身份
```

| Secret | 填什么 |
| --- | --- |
| `APPLE_CERTIFICATE` | 上面 `base64` 的输出 |
| `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 时设置的密码 |
| `APPLE_SIGNING_IDENTITY` | `find-identity` 输出中引号里的整串，如 `Developer ID Application: Example Inc. (AB12CD34EF)` |

### 3. 公证凭据（二选一）

**方式一：App Store Connect API 密钥（推荐，尤其是公司账号）。** 不绑定个人 Apple ID，也不受双重认证影响。

1. 打开 [App Store Connect → 用户和访问 → 集成 → App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api)，在 **团队密钥** 中生成一个密钥，访问权限选 **Developer**；
2. 下载 `AuthKey_<密钥 ID>.p8`（只能下载一次，妥善保存）。

| Secret | 填什么 |
| --- | --- |
| `APPLE_API_KEY` | 密钥 ID（10 位，如 `2X9R4HXF34`） |
| `APPLE_API_ISSUER` | 页面顶部的 Issuer ID（UUID） |
| `APPLE_API_PRIVATE_KEY` | `.p8` 文件的全部内容，包括 `-----BEGIN PRIVATE KEY-----` 与 `-----END PRIVATE KEY-----` 两行 |

**方式二：Apple ID 与 App 专用密码。**

| Secret | 填什么 |
| --- | --- |
| `APPLE_ID` | 开发者账号的 Apple ID 邮箱 |
| `APPLE_PASSWORD` | 在 [appleid.apple.com](https://account.apple.com) → 登录与安全 → App 专用密码 中生成的密码，不是 Apple ID 的登录密码 |
| `APPLE_TEAM_ID` | [Membership details](https://developer.apple.com/account#MembershipDetailsCard) 中的 Team ID（10 位） |

两种都配置时优先使用 API 密钥。只配置证书、不配置公证凭据时，文件会签名但不公证，用户首次打开仍会被 Gatekeeper 拦下。

### 流水线做了什么

- 桌面应用：Tauri 用证书签名 `.app`（包括随附的 `workroam`、`workroamd`），提交公证并装订票据，再签名 `.dmg`；流水线随后公证 `.dmg` 并装订票据，最后检查 `codesign --verify --deep --strict`、`stapler validate` 与 `spctl --assess`；
- 命令行与 `workroam-server`：用同一张证书加 hardened runtime 签名，打包提交公证。单个可执行文件不能装订票据，Gatekeeper 会在线查询公证结果；
- 公证失败时任务失败，日志里有 `notarytool log` 的详细原因。

## Windows（SignPath）

Workroam 使用 [SignPath Foundation](https://signpath.org) 为开源项目免费提供的代码签名证书。证书属于 SignPath Foundation，所以 Windows 上显示的发布者是 **SignPath Foundation**；SignPath 会核实每个被签名的文件都是从本仓库、由 GitHub Actions 构建出来的。

> 2023 年 6 月起，新签发的 OV/EV 代码签名证书都必须保存在硬件或云端密钥库里，不能再导出成 `.pfx` 文件，所以流水线不再支持 `.pfx`。

### 1. 申请

在 [signpath.org](https://signpath.org) 提交开源项目申请。审核关注：

- 使用 OSI 认可的开源许可证（Workroam 是 Apache-2.0），仓库公开，不含闭源组件；
- 项目在持续维护，已经有发布版本，功能有文档说明；
- 项目网站（本仓库）上有代码签名策略，内容见下文 [代码签名策略](#代码签名策略)；
- 参与签名的成员在 GitHub 和 SignPath 上都开启双重认证。

具体条件与流程以 SignPath 的申请页面和审核回复为准。

### 2. 在 SignPath 中配置

申请通过后，SignPath 会在 [app.signpath.io](https://app.signpath.io) 为项目开通组织。登录后：

1. **项目**：确认项目的 slug（默认流水线使用 `workroam`）；
2. **可信构建系统**：在组织设置的 Trusted Build Systems 中添加 **GitHub.com**，并把它关联到该项目；
3. **产物配置（Artifact configuration）**：在项目中新建一个产物配置，内容粘贴仓库中的 [`.github/signpath/artifact-configuration.xml`](../.github/signpath/artifact-configuration.xml)，并设为默认。它会给提交上去的压缩包里所有 `.exe` 与 `.msi` 做 Authenticode 签名；
4. **签名策略**：开源项目通常有 `test-signing` 与 `release-signing` 两个策略。流水线默认用 `release-signing`；
5. **API 令牌**：在 Users 中新建一个 CI 用户，生成 API token，并把这个用户加为签名策略的 Submitter。

### 3. 填写 GitHub Secret 与 Variables

| 名称 | 类型 | 填什么 |
| --- | --- | --- |
| `SIGNPATH_API_TOKEN` | Secret | CI 用户的 API token |
| `SIGNPATH_ORGANIZATION_ID` | Variable | 组织 ID（GUID，在组织设置或网址中） |
| `SIGNPATH_PROJECT_SLUG` | Variable | 项目 slug，默认 `workroam`，一致时可不填 |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Variable | 签名策略 slug，默认 `release-signing` |
| `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | Variable | 产物配置 slug，不填时用项目的默认产物配置 |
| `SIGNPATH_WAIT_SECONDS` | Variable | 等待签名完成的秒数，默认 `7200` |

`SIGNPATH_API_TOKEN` 与 `SIGNPATH_ORGANIZATION_ID` 都配置后，流水线才会提交签名。

### 流水线做了什么

每次发布向 SignPath 提交两次签名请求：

1. **windows-sign**：命令行 `workroam.exe`、`workroamd.exe`、`workroam-server.exe`，以及桌面应用的 `workroam-desktop.exe` 与随附二进制；
2. **windows-bundle** 用签好名的 exe 打出安装包（`--no-binary-patching`，不改写已签名的主程序），然后 **windows-installers** 提交 `.msi` 与 `-setup.exe`。

`release-signing` 策略的请求需要审批人在 SignPath 网页上批准（SignPath 会发邮件），批准后任务继续。超过 `SIGNPATH_WAIT_SECONDS` 没有批准时任务失败，可以在 Actions 中重新运行失败的任务。签名完成后流水线用 `osslsigncode verify` 检查每个文件。

### 代码签名策略

申请 SignPath Foundation 时需要公开的策略：

- Windows 版本的免费代码签名由 [SignPath.io](https://about.signpath.io) 提供，证书由 [SignPath Foundation](https://signpath.org) 颁发；
- 只有从本仓库 `main` 分支或发布 tag、由 GitHub Actions 构建的文件会提交签名；
- 提交者与审阅者：本仓库的维护者（[协作者列表](https://github.com/kverona-ai/workroam/graphs/contributors)）；所有改动通过 PR 合并，`main` 受分支保护；
- 审批人：项目维护者，在 SignPath 上逐次批准发布签名；
- 隐私：Workroam 只在用户自己配置或明确要求时连接网络（例如用户填写的 S3 存储或服务端），不会把信息发送给其他系统。

## Linux（GPG）

Linux 的安装包不需要购买证书。流水线用 GPG 密钥签名 `SHA256SUMS`，用户验证签名后再用校验和核对下载的文件。

### 1. 生成密钥

在自己的电脑上（不是 CI 里）生成一把专门用于发布的签名密钥。把邮箱换成你们能收信的真实地址（它会公开写在公钥里）；GnuPG 会弹窗让你设置一个密码，这个密码就是下面的 `GPG_PASSPHRASE`：

```sh
gpg --quick-gen-key "Workroam Release <release@example.com>" ed25519 sign 3y
gpg --list-secret-keys --keyid-format long      # 记下指纹
```

### 2. 导出并保存

```sh
gpg --armor --export-secret-keys <指纹> | pbcopy    # Linux 可用 xclip -selection clipboard
```

| Secret | 填什么 |
| --- | --- |
| `GPG_PRIVATE_KEY` | 上面导出的整段文字，包括 `-----BEGIN PGP PRIVATE KEY BLOCK-----` 与结尾一行 |
| `GPG_PASSPHRASE` | 密钥的密码；没有设置密码时不填 |

- **备份**：生成密钥时 GnuPG 会在 `~/.gnupg/openpgp-revocs.d/` 下存一份吊销证书，把它离线保存；导出的私钥和密码一起存进密码管理器（续期、换电脑都要用）。`~/.gnupg` 里的文件不要删除，也不需要上传其中任何一个。
- **公钥不用手动上传**：流水线会从 `GPG_PRIVATE_KEY` 导出公钥，作为 `workroam-release-key.asc` 附在每个版本里。
- **可选**：再把公钥（`gpg --armor --export <指纹> > workroam-release-key.asc`）上传到 [keys.openpgp.org](https://keys.openpgp.org/upload)，让用户能从第三方渠道核对。它会给公钥里的邮箱发验证信，验证后才能按邮箱搜到，否则只能按指纹查找。

### 流水线做了什么

生成 `SHA256SUMS` 后签名得到 `SHA256SUMS.asc`，导出公钥 `workroam-release-key.asc`，并立即验证一次。发布说明的“校验”部分会自动多出一行 GPG 验证命令。

## 配好之后验证

1. 打开 [Actions → Release → Run workflow](https://github.com/kverona-ai/workroam/actions/workflows/release.yml)，分支选 `main`，**tag 留空**（演练，不发布）；
2. 在 Prepare 任务的摘要中确认 **签名状态** 表里配置的项目都是 ✅；
3. 配置了 SignPath 时，到 SignPath 网页批准两次签名请求；
4. 运行完成后，从本次运行的 Artifacts 下载 `release-dist`，在对应系统上检查：

```sh
# macOS
spctl --assess --type open --context context:primary-signature -vv Workroam-*.dmg
xcrun stapler validate Workroam-*.dmg
codesign -dv --verbose=2 workroam   # 命令行压缩包解开后
```

```powershell
# Windows：也可以右键文件 → 属性 → 数字签名
Get-AuthenticodeSignature .\Workroam-*-setup.exe | Format-List Status, SignerCertificate
```

```sh
# Linux
gpg --import workroam-release-key.asc
gpg --fingerprint will@kverona.llc   # 确认指纹是 6A82 4B92 90A6 55CD 1B5E  9D56 9094 6593 7E99 AFD6
gpg --verify SHA256SUMS.asc SHA256SUMS
```

都通过后，正常发布下一个版本即可，签名会自动生效。

## 证书与密钥到期

- Developer ID Application 证书自 2026 年 10 月起由 G2 中间证书签发，有效期 1 年（以前是 5 年）。每年到期前按第 1、2 步重新申请并更新 `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`；证书过期后流水线的 macOS 签名会失败。已签名并带时间戳的旧版本不受影响；
- App Store Connect API 密钥不会过期，但可以在页面上吊销，吊销后更新三个 `APPLE_API_*`；
- GPG 密钥按生成时的期限到期，可以用 `gpg --quick-set-expire <指纹> 3y` 延期，然后重新导出 `GPG_PRIVATE_KEY`；
- SignPath 的证书由 SignPath Foundation 维护，不需要处理。
