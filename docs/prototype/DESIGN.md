# Workroam 界面设计规范

本规范是高保真原型（[`index.html`](index.html)）的设计依据，后续 Tauri 桌面应用实现时沿用同一套数值。风格方向是 Apple Liquid Glass：半透明玻璃、背景模糊、镜面高光边缘，支持浅色与深色。

## 参考来源

整理自设计工程师在 X 上分享、并被广泛转述的界面细节做法，以及 Apple 的 Liquid Glass 设计方向：

- Jakub Krehel，[Details that make interfaces feel better](https://jakub.kr/writing/details-that-make-interfaces-feel-better)（同心圆角、阴影、图标、光学对齐、点击区域、可中断的动效）与 [Concentric border radius](https://jakub.kr/work/concentric-border-radius)
- Emil Kowalski，[Agents with taste](https://emilkowal.ski/ui/agents-with-taste)（按下缩放、缓出曲线、动效时长）
- Rauno Freiberg，Invisible Details of Interaction Design（[概述](https://construkt.beehiiv.com/p/invisible-details-interaction-design)）
- 嵌套圆角公式：[Hightouch Border Radius](https://hightouch-ui.vercel.hightouch.com/foundation/border-radius)、[UX Planet](https://uxplanet.org/corner-radius-of-nested-elements-in-ui-design-4c27bb24a854)
- 让界面不像“AI 生成”的常见建议：[DEV: Why AI-generated UIs look off](https://dev.to/kiwibreaksme/why-ai-generated-uis-look-off-and-the-one-principle-that-fixes-it-4j20)、[note.com: AIっぽさを減らすコツ](https://note.com/it_audit/n/na94228399197?hl=en)
- Liquid Glass 的同心与圆角取值：[STRV: How to apply Liquid Glass](https://www.strv.com/blog/how-to-apply-liquid-glass-to-your-app)

## 1. 圆角：同心

**内层圆角 = 外层圆角 − 内边距。**两层圆角共用一个圆心，间隙在拐角处才不会显得忽宽忽窄。内边距大于外层圆角时，把内层当作独立表面，取自己的圆角。全圆（胶囊、圆形）不受此规则约束。

| Token | 值 | 用途 | 推导 |
| --- | --- | --- | --- |
| `--r-win` | 26 | 窗口、托盘桌面 | 基准 |
| `--r-panel` | 18 | 窗口内悬浮的侧栏、向导步骤栏 | 26 − 8（距窗口边 8） |
| `--r-ctl` | 10 | 侧栏条目、输入框、提示条 | 18 − 8（侧栏内边距 8） |
| `--r-float` | 22 | 角落卡片、弹窗、品牌页卡片 | 独立表面 |
| `--r-card` | 16 | 内容区里的分组底板 | 独立表面 |
| `--r-row` | 12 | 分组底板里的行 | 16 − 4 |
| 菜单 | 14 / 9 | 托盘菜单与菜单项 | 14 − 5 |
| 选项卡片 | 20 / 8 | 向导里的选项卡与左侧图标块 | 20 − 12 |
| `--r-chip` | 6 | 版本标签 | — |

按钮、分段控件、开关、状态标签全部用胶囊形。

## 2. 边框：用阴影和色调代替

- 不用 1px 实线框。边缘用 **0.5px 色环**（`0 0 0 .5px var(--ring)`），深度用 **多层带蓝色调的柔和阴影**，不用纯黑。
- 三档阴影：`--sh-1` 控件，`--sh-2` 卡片，`--sh-3` 窗口与悬浮层（色环 + 2px + 28px + 80px 四层）。
- 玻璃表面：自上而下的渐变底色、`blur(32px) saturate(165%)`，顶部 0.5px 高光，外加左上亮、右下弱的镜面描边。
- 区域分隔优先靠留白和底色差；必须分隔时用 **0.5px、带缩进** 的分隔线，与文字左缘对齐，不贯穿整行。
- 不再做“卡片里套卡片”：项目详情直接放在窗口上，只有确实成组的内容（代码仓库、检查项、设备、设置）才放进分组底板。

## 3. 布局与间距

- 4pt 网格：4 / 8 / 12 / 16 / 20 / 24 / 32。
- 窗口：标题栏 44，侧栏悬浮在窗口内、四周留 8；工具栏与内容左缘对齐 20。
- 列表行：项目、快照、设备、检查项都用 **分组列表**（一个底板、行间缩进分隔线），选中态是浅强调色底 + 0.5px 强调色环，不用 2px 粗框。
- 同列数字用 `tabular-nums`；标签 / 数值对齐成两列网格。
- 手机宽度下侧栏变为横向滚动，窗口内容单列。

## 4. 字体

| 层级 | 字号 / 字重 | 用途 |
| --- | --- | --- |
| Display | 44 / 750，字距 −3.5% | 品牌页标题 |
| Large title | 26 / 700 | 场景标题 |
| Title | 22 / 700 | 向导步骤标题 |
| Headline | 18 / 700 | 窗口工具栏标题 |
| Body | 13 / 400–550 | 窗口内正文（macOS 默认 13pt） |
| Secondary | 12 / 400 | 说明、提示 |
| Caption | 11 / 650，大写字距 +6% | 分组标题 |
| Mono | 12–12.5 / 600 | 项目名、路径、命令 |

标题 `text-wrap: balance`，正文 `text-wrap: pretty`；大号标题负字距，小号大写标签正字距。

## 5. 图标

- 一套线性图标，线宽 1.6（14px 时 1.75，20px 时 1.5），圆端点、圆拐角。
- 尺寸跟随相邻文字：12–13px 文字配 16px 图标，侧栏 17px，按钮内 14–16px，选项图标块内 18px。
- 只保留表达含义的图标，不做装饰。
- 光学对齐：带前置图标的按钮左侧内边距比右侧少 3px，图标自带的留白会让两侧看起来相等。

## 6. 颜色

- 中性色带一点蓝色倾向，不用纯灰。
- 强调色 Roam Blue 只用于主操作、选中态和链接；语义色（成功 / 警告 / 失败）只表达状态。
- Baton Amber 只表示“工作区 / 租约”，不做装饰色。
- 状态标签（胶囊）只在详情和卡片里用；列表行里改用“圆点 + 彩色文字”，降低视觉噪音。

## 7. 动效

- 统一缓出曲线 `cubic-bezier(.23, 1, .32, 1)`；悬停与按下 140–160ms，弹窗与卡片 260–340ms。
- 按下 `scale(.97)`；卡片从 `scale(.97)` + 16px 位移进入，不从 0 放大，不回弹。
- 状态变化用 `transition`，保证可中断；尊重 `prefers-reduced-motion`。

## 8. 点击区域

- 窗口内按钮高 30（小号 26），侧栏条目 30，菜单项 26，输入框 32；不小于 26。
- 角落卡片不设回车默认按钮，不抢键盘焦点（PRD UX-05）。
