#!/usr/bin/env bash
#
# docs/MANUAL.md → docs/智票使用说明书.pdf
#
# Two steps rather than one: pandoc renders the Markdown to a self-contained
# HTML file, and headless Chrome prints that to PDF.
#
# pandoc can reach PDF on its own via LaTeX, and that route is the obvious
# one to try - but it needs a CJK-capable engine (xelatex plus a CJK package
# plus a font that has the box-drawing characters the pipeline diagrams are
# made of), which is a large dependency for a document that is mostly tables.
# Chrome is already on every machine that builds this, already shapes Chinese
# correctly, and honours the same `@page`/`break-before` CSS the stylesheet
# uses - so the layout is described in one place (docs/manual.css) instead of
# split between a stylesheet and a LaTeX template.
set -euo pipefail

cd "$(dirname "$0")/.."

CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
IN="docs/MANUAL.md"
HTML="$(mktemp -t manual).html"
OUT="docs/智票使用说明书.pdf"

command -v pandoc >/dev/null || { echo "需要 pandoc：brew install pandoc" >&2; exit 1; }
[ -x "$CHROME" ] || { echo "找不到 Chrome：$CHROME" >&2; exit 1; }

VERSION=$(python3 -c "import json;print(json.load(open('package.json'))['version'])")

# No --toc and no title metadata: MANUAL.md already opens with its own
# heading and a hand-written 目录 whose entries link to the chapters. Letting
# pandoc add its own produced a first page that repeated both.
pandoc "$IN" \
  --from=gfm \
  --to=html5 \
  --standalone \
  --metadata title="智票 Invoice Desk 使用说明书" \
  --css=manual.css \
  --resource-path=docs \
  --embed-resources \
  --output="$HTML"

# `--headless=new` is required: the old headless mode ignores `--no-pdf-header-footer`
# and stamps every page with the temp file's path.
"$CHROME" \
  --headless=new \
  --disable-gpu \
  --no-pdf-header-footer \
  --print-to-pdf="$OUT" \
  "file://$HTML" 2>/dev/null

rm -f "$HTML"
echo "已生成 ${OUT} (v${VERSION}, $(du -h "${OUT}" | cut -f1))"
