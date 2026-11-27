#!/bin/sh
# Builds .deb and .rpm packages from the Linux release archives.
#
#   packaging/linux-packages.sh VERSION ARCHIVE_DIR OUT_DIR
#
# ARCHIVE_DIR holds tunwarden-cli-{x86_64,aarch64}-unknown-linux-musl.tar.xz
# as uploaded by dist. Needs nfpm on PATH. Run from the repository root.
set -eu
version=$1
archives=$2
out=$3
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for pair in x86_64:amd64 aarch64:arm64; do
  target="${pair%%:*}-unknown-linux-musl"
  arch="${pair##*:}"
  archive="$archives/tunwarden-cli-$target.tar.xz"
  [ -f "$archive" ] || { echo "missing $archive" >&2; exit 1; }
  mkdir -p "$work/$target"
  tar -xJf "$archive" -C "$work/$target"
  bin=$(find "$work/$target" -type f -name tunwarden | head -n 1)
  [ -n "$bin" ] || { echo "no tunwarden binary in $archive" >&2; exit 1; }
  stage="$work/stage-$arch"
  mkdir -p "$stage"
  cp packaging/nfpm.yaml LICENSE-MIT LICENSE-APACHE README.md "$stage/"
  cp "$bin" "$stage/tunwarden"
  for packager in deb rpm; do
    (cd "$stage" && ARCH=$arch VERSION=$version \
      nfpm package --config nfpm.yaml --packager "$packager" --target "$out/")
  done
done
ls -l "$out"
