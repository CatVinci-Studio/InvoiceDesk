# 参与开发

## 环境

- [Node.js](https://nodejs.org) 20+
- [Rust](https://rustup.rs) stable —— **请保持最新**，CI 用的是
  `dtolnay/rust-toolchain@stable`，本地落后一个版本就会出现「本地全绿、CI 挂在
  clippy」的情况
- macOS：Xcode 命令行工具 · Windows：MSVC Build Tools

```bash
npm install
npm run tauri dev
```

## 提交前必须跑

```bash
npm run check                    # typecheck + eslint + vitest + prettier
cd src-tauri
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

CI 跑的是同一组命令，`-D warnings` 也一样，所以本地过了 CI 就会过。

想看有数据的界面：

```bash
cd src-tauri && cargo run --example seed_sample_data
```

它会造几张合成发票走完整条识别流水线写进账本，覆盖 UI 需要渲染的每一种状态
（干净的、金额对不上的、疑似重复的、完全读不出来的）。因为走的是真实流水线，
这同时也是一次脱离测试框架的端到端检查。

## 代码里的两条硬规矩

### 1. 金额只能是整数「分」

`Money` 是 `i64`，单位是分。**不要**在任何地方用浮点表示金额，包括中间计算。

浮点表示不了 `0.1`。`1.005 * 100` 等于 `100.49999999999999`，四舍五入变成
¥1.00 —— 凭空少一分钱，而且不会报错。一千笔 ¥0.07 用浮点累加也不等于 ¥70.00。
在一张要和银行流水对账的表里，这是缺陷，不是精度问题。

唯一允许出现的浮点转换是 `Money::to_yuan_f64()`，因为 xlsx 的数字单元格按规范
就是 IEEE double，`rust_xlsxwriter` 的 API 只收 `f64`。它有文档说明为什么安全，
也有测试证明每一分钱在往返中都不会丢。模板导出连这一次都没有 —— 它自己写单元格
XML，直接把整数算出的十进制字符串放进去。

前端同理：`Cents` 是整数，`parseYuan` 用字符串整数运算，不用 `parseFloat`。
除以 100 只发生在 `formatMoney` 里，也就是数字变成文本的最后一刻。

### 2. 读不准就明说，不要猜

每个字段是一个 `Field<T>`，带着来源（`FieldSource`）和置信度。`merge_from`
保证更可信的来源永远不会被更差的覆盖，人工录入的值优先级最高。

新增识别路径时，**宁可返回 `None` 也不要返回一个看起来合理的值**。一个编造的
发票号码会通过所有位数检查、让查重失效，然后再也没人看第二眼 —— 比读不出来糟
得多。`Money::parse` 和 `parse_payload` 都是这么写的，遇到不确定的输入就拒绝。

## 目录

| 路径                      | 内容                                      |
| ------------------------- | ----------------------------------------- |
| `src/`                    | React 前端，按功能分模块                  |
| `src-tauri/src/model.rs`  | 领域模型：`Money`、`Field`、`InvoiceKind` |
| `src-tauri/src/extract/`  | 分层提取：XML / 二维码 / PDF / OFD        |
| `src-tauri/src/parse/`    | 文本字段解析与跨字段校验                  |
| `src-tauri/src/classify/` | 分类规则引擎与内置规则                    |
| `src-tauri/src/db/`       | SQLite 账本与查重                         |
| `src-tauri/src/report/`   | Excel 导出与公司模板填充                  |
| `src-tauri/src/ai/`       | 服务商目录、凭据路由、视觉识别            |
| `src-tauri/src/commands/` | Tauri 命令层（很薄，不放业务逻辑）        |

模块头部的文档注释解释了「为什么这么做」，改之前值得先读。尤其是
`extract/mod.rs`（分层顺序）、`extract/qrcode.rs`（为什么二维码优于 OCR）、
`classify/mod.rs`（为什么规则优于模型）和 `report/template.rs`（为什么直接改
XML 而不是重建工作簿）。

## 增加一种发票格式

1. 在 `extract/detect.rs` 里加上识别（按内容，不要按扩展名）
2. 写一个提取模块，产出带 `FieldSource` 的 `Field`
3. 在 `extract/mod.rs` 的流水线里按可信度插到合适的位置
4. 加测试 —— 尤其是「读不出来时返回空而不是错值」这一条

## 增加一个 AI 服务商

只需要在 `src-tauri/src/ai/catalog.rs` 的 `PROVIDER_CATALOG` 里加一条，前提是
它兼容 OpenAI Chat Completions（国内厂商基本都兼容）。同步更新
`src/settings/provider-catalog.ts`，那边有测试会比对两份表是否一致。

## 提交信息

用中文，说清楚**为什么**，不只是改了什么。仓库里的注释也是这个风格 —— 代码本身
已经说明了「做了什么」。

## License

MIT。提交即表示同意以该许可证发布你的贡献。
