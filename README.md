<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="Invoice Desk" width="120" height="120" />
</p>

<h1 align="center">Invoice Desk &nbsp;智票</h1>

<p align="center">
  <strong>Turns a pile of Chinese invoices into one reimbursement sheet.</strong><br>
  Batch recognition, automatic categorisation, duplicate-claim detection, and the Excel file finance actually wants.
</p>

<p align="center">
  <a href="https://github.com/CatVinci-Studio/InvoiceDesk/releases/latest"><strong>Download</strong></a> ·
  <a href="./README.zh.md">中文</a> ·
  <a href="./docs/MANUAL.md">Manual (Chinese)</a>
</p>

<p align="center">
  <img alt="platform" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey">
  <a href="./LICENSE"><img alt="license" src="https://img.shields.io/badge/license-MIT-yellow"></a>
</p>

---

## What it is

Invoice Desk (智票) is a local-first tool for Chinese VAT invoices (增值税发票). Drop in
electronic invoices (PDF / OFD / XML) and photos of paper ones, and it reads the
fields, categorises them, flags anything that may already have been claimed, and
exports a reimbursement workbook — or fills in your company's own .xlsx form.

**The interface is Chinese only.** The domain is China's tax system: 数电票,
发票代码, 价税合计, 进项税额抵扣. Translating the labels without translating the
concepts would help nobody.

## The extraction pipeline

Recognition is layered by how far each source can be trusted, and OCR is the last
resort rather than the first:

| Layer | Source | Trust | Why |
| --- | --- | --- | --- |
| 1 | Invoice XML | Authoritative | 数电票 PDFs often embed the original XML, and OFD packages attach it. This is not a *reading* of the invoice — it is the invoice. |
| 2 | QR code | Exact | Every VAT invoice carries one. It either decodes to exactly what was encoded, or it fails to decode. There is no middle state to be wrong in — unlike OCR, which hands back a confident wrong digit. |
| 3 | PDF text layer | High | Electronic invoices are generated PDFs, so the values are real text. Free, offline, exact. |
| 4 | Vision model | Advisory | Only for photos the first three layers could not finish. **Off by default.** |

Every field records which layer produced it and how far to trust it. Anything below
the bar is highlighted for review — the app never silently reports a number it is
unsure of.

Scanned PDFs need no rasteriser: the original JPEG is already sitting in the PDF's
object graph, so it is extracted directly rather than re-rendered. That keeps the
binary free of a 4 MB pdfium per platform and gives a *better* image than rendering
would, because nothing is re-compressed.

## Privacy

Invoices are financial records carrying tax IDs and amounts, so everything stays in
a local SQLite file: no upload, no sync, no phone-home verification. The two AI
features are separate opt-in switches, both off by default. Vision sends the image;
category suggestion sends only the 票种, seller name and line items — never amounts,
tax IDs, or the buyer. API keys live in a 0600 file apart from the ledger, because
the ledger is a document users copy, back up, and hand to colleagues.

Providers are Chinese services only (Alibaba DashScope, Zhipu, Volcengine Ark,
Moonshot, StepFun, Tencent Hunyuan, MiniMax, SiliconFlow, DeepSeek), plus local
Ollama and a custom OpenAI-compatible endpoint.

## Install

Download from [Releases](https://github.com/CatVinci-Studio/InvoiceDesk/releases/latest):

| Platform | Package |
| --- | --- |
| macOS (Apple silicon) | `Invoice Desk_X.Y.Z_aarch64.dmg` |
| macOS (Intel) | `Invoice Desk_X.Y.Z_x64.dmg` |
| Windows | `Invoice Desk_X.Y.Z_x64-setup.exe` |

> **The first launch needs one extra step.** The packages are not code-signed:
>
> - **macOS** — **right-click** Invoice Desk → Open → Open again in the dialog. Once, ever.
> - **Windows** — on "Windows protected your PC", click More info → Run anyway.
>
> Nothing is wrong with the download; this is what both systems do with
> unsigned applications.

## Getting started

1. Drop this month's invoice files — or the whole folder — onto the window.
2. When the import finishes, look at **待复核** and **疑似重复** in the
   sidebar. Those two are the only rows that need a person; everything else is
   already usable.
3. Adjust the categorisation rules under **分类规则** if anything landed in the
   wrong bucket, then hit 重新分类全部发票.
4. Create a sheet under **报销单**, tick the invoices to claim, and export.

The full manual is [docs/MANUAL.md](./docs/MANUAL.md) (Chinese).

## Build from source

```bash
git clone https://github.com/CatVinci-Studio/InvoiceDesk.git
cd InvoiceDesk
npm install

npm run tauri dev     # dev mode
npm run tauri build   # package .dmg / .exe
npm run check         # typecheck + lint + test + format
cd src-tauri && cargo test
```

Requires [Node.js](https://nodejs.org) 20+ and [Rust](https://rustup.rs) stable.
macOS needs the Xcode command line tools; Windows needs the MSVC build tools.
Development conventions: [CONTRIBUTING.md](./CONTRIBUTING.md) (Chinese).
Release process: [docs/RELEASE.md](./docs/RELEASE.md).

To see the UI with something in it, seed the ledger with synthetic invoices —
they run through the real pipeline, so this doubles as an end-to-end check:

```bash
cd src-tauri && cargo run --example seed_sample_data
```

## How it is put together

Tauri 2 + React 19 + TypeScript, with a Rust backend. No OCR engine and no PDF
renderer as dependencies.

`src-tauri/src/` splits so that everything handling money is a pure function
over bytes, and anything touching the network, the disk or the window lives at
the edges:

| Module | Responsibility |
| --- | --- |
| `model` | the invoice, and the rule that money is integer 分 |
| `extract` | bytes → fields, layered by how far each source can be trusted |
| `parse` | text → fields, plus the cross-field checks |
| `classify` | fields → 报销类别, by rule first and by model only as a fallback |
| `db` | the local ledger, and duplicate-reimbursement detection |
| `report` | invoices → an .xlsx sheet, generic or into a company template |
| `ai` | provider catalog, credential routing, vision recognition |
| `commands` | the Tauri surface the frontend calls |

## License

[MIT](./LICENSE) © CatVinci Studio
