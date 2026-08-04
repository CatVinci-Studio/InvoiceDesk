# 发布流程

智票只发 macOS 与 Windows 两个平台，没有 Linux 构建。发布由
`.github/workflows/release.yml` 在收到 `v*.*.*` 标签时自动完成，本文档说的
是"打标签之前要做对什么"。

## 一、版本号：三处必须完全一致

| 文件                        | 字段                |
| --------------------------- | ------------------- |
| `package.json`              | `version`           |
| `src-tauri/Cargo.toml`      | `[package] version` |
| `src-tauri/tauri.conf.json` | `version`           |

标签本身是第四处：`v` + 同一个版本号（`v0.2.0`）。

这不是洁癖。`tauri.conf.json` 的 `version` 决定安装包文件名和 `latest.json`
里写的版本；`Cargo.toml` 的 `version` 编进二进制里，是应用运行时拿来跟
`latest.json` 比较的那个值。两者不一致时不会报错，只会出现两种安静的故障：
用户被反复提示同一个"新版本"，或者永远收不到更新。

> Tauri 允许 `tauri.conf.json` 省略 `version` 而直接读 `Cargo.toml`。本项目
> 两处都写了，所以只能手动保持同步——改版本号时三个文件一起改。

## 二、切一个版本

1. 改上面三处版本号。
2. 本地跑一遍 `npm run check`（typecheck + lint + test + format:check）。
   CI 会再跑一次，但在本地发现问题比等 runner 快得多。
3. 提交，合进 `main`。
4. 打标签并推送：

   ```sh
   git tag -a v0.2.0 -m "v0.2.0"
   git push origin main --follow-tags
   ```

5. 到 Actions → Release 看三个并行 job：macOS aarch64、macOS x86_64、
   Windows。首次构建较慢——`rusqlite` 用的是 `bundled`，SQLite 要从 C 源码
   编译；之后有 Rust 缓存会快很多。

### 产物清单

> `productName` 是「Invoice Desk」，带一个空格。GitHub 上传发布资源时会把
> 文件名里的空格替换成点，所以下载下来的是 `Invoice.Desk_0.0.2_aarch64.dmg`
> 而不是带空格的名字。dmg 里面的 `.app` 仍然叫「Invoice Desk.app」，Finder
> 和菜单栏显示的也是带空格的两个词。写文档时按**带点**的名字写。

跑完之后 Release 页面上应该有：

| 文件                                           | 用途                      |
| ---------------------------------------------- | ------------------------- |
| `Invoice Desk_<版本>_aarch64.dmg`              | macOS，Apple Silicon      |
| `Invoice Desk_<版本>_x64.dmg`                  | macOS，Intel              |
| `Invoice Desk_<版本>_x64-setup.exe`            | Windows，NSIS 安装包      |
| `Invoice Desk_<版本>_aarch64.app.tar.gz(.sig)` | 更新器用（Apple Silicon） |
| `Invoice Desk_<版本>_x64.app.tar.gz(.sig)`     | 更新器用（Intel）         |
| `Invoice Desk_<版本>_x64-setup.nsis.zip(.sig)` | 更新器用（Windows）       |
| `latest.json`                                  | 更新器清单                |

Windows 只出 NSIS `.exe`，不出 MSI——两种安装器注册在不同的位置，谁也认不出
对方装的那份，用户装过一次 `.msi` 之后就再也没法被 `.exe` 覆盖升级。原因和
细节写在 `src-tauri/tauri.windows.conf.json` 与 `release.yml` 的注释里。

`latest.json` 里必须同时有 `darwin-aarch64`、`darwin-x86_64`、
`windows-x86_64` 三个键。三个 job 是并行写同一个 `latest.json` 的，写法是
"把 Release 上已有的那份读回来、合并、再传上去"，不是覆盖。所以某个 job 失
败时，结果是 `latest.json` 少一个平台键、该平台用户收不到更新——**重跑失败
的那个 job 就行，不用重新打标签**。

## 三、预发布（rc）标签

标签里只要带连字符（`v0.2.0-rc.1`、`-beta.1`、`-alpha`），workflow 里的
`prerelease: ${{ contains(github.ref_name, '-') }}` 就会把这个 Release 标成
预发布。

这一条是把 rc 和普通用户隔开的**唯一**机制：`tauri.conf.json` 里的更新
endpoint 指向 `releases/latest/download/latest.json`，而 GitHub 的 "latest"
按定义排除预发布，所以稳定版用户根本读不到 rc 的 `latest.json`。反过来说，
如果没有这一行，标签规则 `v*.*.*` 同样会匹配 rc 标签，然后把它当成新的稳定
版推给所有人。

其余差别：

- rc 只能从 Release 页面手动下载安装。
- 装了 rc 的用户之后仍会被升级到下一个**稳定版**（endpoint 一样指向
  latest），这是期望行为。
- Windows 不受"MSI 不接受非数字预发布标识"的限制，因为本项目本来就只打
  NSIS。

## 四、需要在仓库里配置的 Secrets

Settings → Secrets and variables → Actions。`GITHUB_TOKEN` 由 Actions 自动
提供，不用配。

### 更新器签名（必需）

| Secret                               | 是什么                                           |
| ------------------------------------ | ------------------------------------------------ |
| `TAURI_SIGNING_PRIVATE_KEY`          | minisign 私钥**文件的内容**（不是路径）          |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 生成密钥时设的口令；没设就填空值，但 secret 要建 |

`tauri.conf.json` 里 `bundle.createUpdaterArtifacts` 是 `true`，所以这两个缺
失时构建会**直接失败**，而不是安静地发一个没法自动更新的版本。

### 代码签名：**没有，这是故意的**

智票不做 macOS 签名公证，也不做 Windows 代码签名。发布的就是普通的未签名
`.dmg` 和 `.exe`。

这不是待办事项，是一个权衡的结果：Apple 开发者账号每年 99 美元，Windows 的
OV/EV 证书更贵，而这个工具的用户群体很清楚自己在装什么。代价是可见的，也只有
一处：

| 平台    | 用户会看到                                | 绕过办法（各做一次）                       |
| ------- | ----------------------------------------- | ------------------------------------------ |
| macOS   | 「无法打开，因为无法验证开发者」          | **右键**点应用 → 打开 → 弹窗里再点「打开」 |
| Windows | 「Windows 已保护你的电脑」（SmartScreen） | 「更多信息」→「仍要运行」                  |

**这两句话必须出现在 README 和使用说明书里。** 不知道的用户会以为下载的文件坏
了，然后就不用了——一个可以靠一行文档解决的问题，不该变成流失。

以后如果买了证书：`release.yml` 里 `TAURI_SIGNING_PRIVATE_KEY` 下面的注释写明
了六个 `APPLE_*` secret 该加在哪，加回去，再把上面两条提示从文档里删掉。注意
六个要么全配、要么一个都别配——只配一半会让 macOS 的 job 直接失败。

## 五、生成更新器密钥对（minisign）

**一次性操作，整个项目生命周期只做一次。**换密钥等于所有已安装的旧版本再也
验不过更新签名，只能让用户手动重装。

```sh
npm run tauri -- signer generate -w ~/.tauri/invoicedesk_updater.key
```

命令产出两样东西：

- **私钥**（`~/.tauri/invoicedesk_updater.key`）——整份内容贴进
  `TAURI_SIGNING_PRIVATE_KEY`，口令贴进
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- **公钥**（命令输出里的 Public key，也存在
  `~/.tauri/invoicedesk_updater.key.pub`）——填进
  `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

该字段现在还是占位符 `REPLACE_WITH_MINISIGN_PUBKEY`，**首次发布前必须替
换**，否则应用侧无法校验任何下载下来的更新包。

私钥不要提交进仓库（`.gitignore` 之外也别放进项目目录），另外离线备份一份：
丢了就只能换密钥对，也就等于放弃全部存量用户的自动更新通道。

## 六、发布后自查

```sh
curl -sSL https://github.com/CatVinci-Studio/InvoiceDesk/releases/latest/download/latest.json
```

确认三件事：

1. `version` 是本次发布的版本；
2. `platforms` 里 `darwin-aarch64`、`darwin-x86_64`、`windows-x86_64` 都在；
3. 拿上一个版本装一次，能收到更新提示并成功升级。

发预发布标签时这个 URL 应当**仍然**返回上一个稳定版——如果它返回了 rc，说明
Release 没被标成预发布，需要立刻到 Release 页面手动勾上 "Set as a
pre-release"。
