#!/usr/bin/env bash
# Publica los instalables de VirtualKvm en https://install.stevearagonsite.com/virtualkvm/
#
#   scripts/release/publish.sh          compila (macOS local + Windows por ssh), empaqueta y sube
#   scripts/release/publish.sh --dry    solo compila y empaqueta en target/dist/
#
# Requisitos: el commit actual pusheado a GitHub (la página linkea el código fuente exacto, como pide
# la GPL), `ssh windows-kvm` con el repo en C:\Users\steve\vkvm\VirtualKvm, y el perfil de AWS
# DeliciasCastillo. La versión sale de ./VERSION: subila antes de publicar una versión nueva.
#
# VirtualKvm está basado en Lan Mouse (https://github.com/feschber/lan-mouse), GPL-3.0-or-later.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO"
DRY=0; [[ "${1:-}" == "--dry" ]] && DRY=1

VERSION="$(tr -d '[:space:]' < VERSION)"
COMMIT_FULL="$(git rev-parse HEAD)"
COMMIT="$(git rev-parse --short HEAD)"
BRANCH="$(git branch --show-current)"
DATE="$(date +%Y-%m-%d)"
BUCKET="stevearagonsite-install"
DISTRIBUTION="E3K2B712AJR9ZE"
WIN_REPO='C:\Users\steve\vkvm\VirtualKvm'
OUT="$REPO/target/dist/$VERSION"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "hay cambios sin commitear: commiteá antes de publicar" >&2; exit 1
fi
if [[ "$DRY" == 0 ]] && ! git branch -r --contains "$COMMIT_FULL" | grep -q "origin/"; then
  echo "el commit $COMMIT no está en GitHub: hacé push antes de publicar" >&2; exit 1
fi

echo "== VirtualKvm $VERSION ($COMMIT)"
rm -rf "$OUT"; mkdir -p "$OUT/macos/VirtualKvm" "$OUT/windows/VirtualKvm"
readme() { sed "s/{{VERSION}}/$VERSION/g" packaging/README.txt > "$1/README.txt"; cp LICENSE NOTICE "$1/"; }

echo "-- macOS (arm64)"
cargo build -q --release --no-default-features
cp target/release/vkvm packaging/macos/install.sh "$OUT/macos/VirtualKvm/"
readme "$OUT/macos/VirtualKvm"

echo "-- Windows (x64) en windows-kvm"
git push -q pc "HEAD:refs/heads/$BRANCH" 2>/dev/null
ssh windows-kvm "Set-Location $WIN_REPO; git checkout -q $BRANCH; git reset -q --hard $COMMIT_FULL; cargo build --release --no-default-features 2>&1 | Select-String -Pattern '^error' -Context 0,8 | Out-String; exit \$LASTEXITCODE" 2>/dev/null
scp -q "windows-kvm:C:/Users/steve/vkvm/VirtualKvm/target/release/vkvm.exe" "$OUT/windows/VirtualKvm/vkvm.exe" 2>/dev/null
cp packaging/windows/install.ps1 "$OUT/windows/VirtualKvm/"
readme "$OUT/windows/VirtualKvm"

echo "-- zips y checksums"
MAC_ZIP="VirtualKvm-$VERSION-macos-arm64.zip"; WIN_ZIP="VirtualKvm-$VERSION-windows-x64.zip"
(cd "$OUT/macos" && zip -qrX "../$MAC_ZIP" VirtualKvm)
(cd "$OUT/windows" && zip -qrX "../$WIN_ZIP" VirtualKvm)
mkdir -p "$OUT/latest"
cp "$OUT/$MAC_ZIP" "$OUT/latest/VirtualKvm-macos-arm64.zip"
cp "$OUT/$WIN_ZIP" "$OUT/latest/VirtualKvm-windows-x64.zip"
(cd "$OUT" && shasum -a 256 "$MAC_ZIP" "$WIN_ZIP" > SHA256SUMS)
(cd "$OUT/latest" && shasum -a 256 VirtualKvm-macos-arm64.zip VirtualKvm-windows-x64.zip > SHA256SUMS)
printf '{"version":"%s","commit":"%s","date":"%s"}\n' "$VERSION" "$COMMIT_FULL" "$DATE" > "$OUT/latest/latest.json"
sed -e "s/{{VERSION}}/$VERSION/g" -e "s/{{COMMIT}}/$COMMIT/g" -e "s/{{COMMIT_FULL}}/$COMMIT_FULL/g" -e "s/{{DATE}}/$DATE/g" \
  packaging/web/virtualkvm/index.html.tmpl > "$OUT/index.html"
cat "$OUT/SHA256SUMS"

if [[ "$DRY" == 1 ]]; then echo "dry run: todo quedó en $OUT"; exit 0; fi

echo "-- subiendo a s3://$BUCKET"
export AWS_PROFILE=DeliciasCastillo
put() { aws s3 cp --only-show-errors "$1" "s3://$BUCKET/$2" --content-type "$3" --cache-control "$4"; }
IMMUTABLE="public, max-age=31536000, immutable"; SHORT="public, max-age=300"
put "$OUT/$MAC_ZIP" "virtualkvm/$VERSION/$MAC_ZIP" application/zip "$IMMUTABLE"
put "$OUT/$WIN_ZIP" "virtualkvm/$VERSION/$WIN_ZIP" application/zip "$IMMUTABLE"
put "$OUT/SHA256SUMS" "virtualkvm/$VERSION/SHA256SUMS" "text/plain; charset=utf-8" "$IMMUTABLE"
put "$OUT/latest/VirtualKvm-macos-arm64.zip" "virtualkvm/latest/VirtualKvm-macos-arm64.zip" application/zip "$SHORT"
put "$OUT/latest/VirtualKvm-windows-x64.zip" "virtualkvm/latest/VirtualKvm-windows-x64.zip" application/zip "$SHORT"
put "$OUT/latest/SHA256SUMS" "virtualkvm/latest/SHA256SUMS" "text/plain; charset=utf-8" "$SHORT"
put "$OUT/latest/latest.json" "virtualkvm/latest/latest.json" "application/json" "$SHORT"
put "$OUT/index.html" "virtualkvm/index.html" "text/html; charset=utf-8" "$SHORT"
put packaging/web/virtualkvm/install.sh "virtualkvm/install.sh" "text/x-shellscript; charset=utf-8" "$SHORT"
put packaging/web/virtualkvm/install.ps1 "virtualkvm/install.ps1" "text/plain; charset=utf-8" "$SHORT"
put packaging/web/index.html "index.html" "text/html; charset=utf-8" "$SHORT"
put packaging/web/style.css "style.css" "text/css; charset=utf-8" "$SHORT"

echo "-- invalidando CloudFront"
aws cloudfront create-invalidation --distribution-id "$DISTRIBUTION" \
  --paths "/index.html" "/style.css" "/virtualkvm/*" --query Invalidation.Id --output text
echo "listo: https://install.stevearagonsite.com/virtualkvm/"
