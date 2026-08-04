<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="智税" width="120" height="120" />
</p>

<h1 align="center">智税 ZhiShui</h1>

<p align="center">
  <strong>把一堆发票变成一张报销表。</strong><br>
  批量识别、自动分类、重复报销预警，导出成财务要的那张 Excel。
</p>

<p align="center">
  <a href="https://github.com/CatVinci-Studio/ZhiShui/releases/latest"><strong>下载</strong></a> ·
  <a href="./README.md">English</a> ·
  <a href="./docs/MANUAL.md">使用说明书</a>
</p>

<p align="center">
  <img alt="platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey">
  <a href="./LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-yellow"></a>
</p>

---

## 这是什么

智税是一款本地运行的发票整理工具。把电子发票（PDF / OFD / XML）和纸质发票的照片一起拖进来，它会：

1. **读出字段** —— 发票号码、开票日期、购销方、金额、税额、价税合计、明细项目
2. **自动分类** —— 住宿、餐饮、市内交通、办公用品……按规则归类，规则你可以改
3. **查重预警** —— 同一张发票如果之前报销过，会被标出来
4. **导出报销单** —— 生成明细表 + 分类汇总表，或者直接填进你公司自己的 Excel 模板

## 为什么做这个

报销这件事的时间几乎全花在机械劳动上：一张一张打开 PDF、抄号码、抄金额、填表、再核对一遍有没有重复。这些都是计算机该做的事。

市面上的工具要么是 SaaS（发票是带着税号和金额的财务凭证，不想传到别人服务器上），要么依赖 OCR 把 `8` 认成 `3` 却看起来一切正常。

智税的做法不一样：**能不猜就不猜**。

## 识别是分层的

不是一上来就 OCR，而是按可靠程度从高到低试：

| 层级 | 来源 | 可靠性 | 说明 |
|------|------|--------|------|
| 1 | 发票 XML | 权威 | 数电票 PDF 常内嵌原始 XML，OFD 也带附件。这不是"识别"，这就是发票本身 |
| 2 | 二维码 | 精确 | 每张增值税发票都有。要么解出来，要么解不出来，不会给你一个"看着像对的"错数字 |
| 3 | PDF 文本层 | 高 | 电子发票是生成的 PDF，字段是真文本，离线、免费、精确 |
| 4 | AI 视觉识别 | 参考 | 只有前三层都读不出的照片才会走这一步，**默认关闭** |

每个字段都记着它是从哪一层来的、有多可信。低可信度的字段会在界面上高亮，等你确认——**这个软件不会悄悄给你一个它自己都不确定的金额**。

## 隐私

- 所有发票数据存在本机 SQLite 里，不上传、不同步、不联网校验
- AI 识别默认关闭。开启后，只有离线读不出来的**图片**会发送到你选的服务商
- AI 分类建议是另一个独立开关。它只发送票种、销售方名称、项目名称——不发金额、不发税号、不发购买方
- API Key 存在单独的 0600 权限文件里，不和发票数据放在一起（数据库你可能会备份或拷给同事，密钥不该跟着走）

AI 服务商只收录国内厂商：阿里云百炼（通义千问）、智谱 GLM、火山方舟（豆包）、月之暗面 Kimi、阶跃星辰、腾讯混元、MiniMax、硅基流动、DeepSeek，以及本机 Ollama 和自定义接口。

## 安装

从 [Releases](https://github.com/CatVinci-Studio/ZhiShui/releases/latest) 下载：

| 平台 | 安装包 |
| --- | --- |
| macOS（Apple 芯片） | `ZhiShui_X.Y.Z_aarch64.dmg` |
| macOS（Intel 芯片） | `ZhiShui_X.Y.Z_x64.dmg` |
| Windows | `ZhiShui_X.Y.Z_x64-setup.exe` |

> **第一次打开需要多一步。** 安装包没有做代码签名，所以：
>
> - **macOS**：**右键**点智税 →「打开」→ 弹窗里再点一次「打开」。只需要做这一次。
> - **Windows**：出现「Windows 已保护你的电脑」时，点「更多信息」→「仍要运行」。
>
> 这不是安装包有问题，是系统对未签名应用的默认拦截。

## 快速上手

1. 打开智税，把发票文件或整个文件夹拖进窗口
2. 导入完成后看一眼「待复核」和「疑似重复」——需要人看的就这两类
3. 需要的话在「分类规则」里调整归类规则，然后「重新分类全部发票」
4. 新建一张报销单，勾上要报的发票，导出 Excel

完整说明见 [使用说明书](./docs/MANUAL.md)。

## 从源码构建

```bash
git clone https://github.com/CatVinci-Studio/ZhiShui.git
cd ZhiShui
npm install

npm run tauri dev     # 开发模式
npm run tauri build   # 打包 .dmg / .exe
npm run check         # typecheck + lint + test + format
cd src-tauri && cargo test
```

需要 [Node.js](https://nodejs.org) 20+ 和 [Rust](https://rustup.rs) stable。macOS 需要 Xcode 命令行工具，Windows 需要 MSVC Build Tools。

想看有数据的界面，可以灌一批合成发票——它们走的是真实的识别流水线，所以这同时也是一次端到端检查：

```bash
cd src-tauri && cargo run --example seed_sample_data
```

发布流程见 [docs/RELEASE.md](./docs/RELEASE.md)。

## 技术说明

Tauri 2 + React 19 + TypeScript，后端 Rust。**不依赖 OCR 引擎，也不依赖 PDF 渲染器**——扫描件的位图是直接从 PDF 对象图里取出来的原始 JPEG，比重新渲染还清楚，还省掉每平台约 4MB 的 pdfium。

`src-tauri/src/` 这样划分，目的是让所有和金额有关的逻辑都是「字节进、结构出」的纯函数，联网、读盘、开窗口这些都推到边缘：

| 模块 | 职责 |
| --- | --- |
| `model` | 领域模型，以及「金额一律整数分」这条规矩 |
| `extract` | 字节 → 字段，按来源可信度分层 |
| `parse` | 文本 → 字段，以及跨字段校验 |
| `classify` | 字段 → 报销类别，规则优先，模型兜底 |
| `db` | 本地账本与重复报销检测 |
| `report` | 发票 → Excel，通用表或填进公司模板 |
| `ai` | 服务商目录、凭据路由、视觉识别 |
| `commands` | 前端调用的 Tauri 接口 |

金额的处理写在 [`model.rs`](./src-tauri/src/model.rs) 的 `Money` 上：整数分、不经浮点，唯一一次转成 `f64` 是写 Excel 数字单元格时（xlsx 规范如此），并且有测试证明每一分钱在往返中都不会丢。

## License

[MIT](./LICENSE) © CatVinci Studio
