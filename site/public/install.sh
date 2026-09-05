#!/bin/sh
# Cratebase installer — https://cratebase.dev/install.sh
#
#   curl -fsSL https://cratebase.dev/install.sh | sh
#
# Served verbatim as a static file (see site/public/install.sh in the
# cratebase.dev source) — what you see here is byte-for-byte what runs.
# POSIX sh only, no bashisms: `curl | sh` invokes `sh`, and shells like
# `dash` (the default /bin/sh on Debian/Ubuntu) reject bash-only syntax.
set -e

# Single line to change if the final GitHub org/repo slug ever differs.
REPO="cratebasehq/cratebase"

err() {
    echo "error: $*" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# 1. OS/arch detection
# ---------------------------------------------------------------------------

os=$(uname -s)
arch=$(uname -m)

case "$os" in
    MINGW* | MSYS* | CYGWIN*)
        echo "Cratebase does not have a POSIX shell installer for Windows." >&2
        echo "Download the Windows binary directly:" >&2
        echo "  https://github.com/${REPO}/releases/latest/download/cratebase_<version>_x86_64-pc-windows-msvc.zip" >&2
        echo "(replace <version>, or use the \"Latest\" release page: https://github.com/${REPO}/releases/latest)" >&2
        echo "Or install via Docker:" >&2
        echo "  docker run -p 8090:8090 ghcr.io/${REPO}:latest" >&2
        exit 1
        ;;
esac

case "$os" in
    Linux)
        case "$arch" in
            x86_64) target="x86_64-unknown-linux-musl" ;;
            aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
            *) target="" ;;
        esac
        ;;
    Darwin)
        case "$arch" in
            x86_64) target="x86_64-apple-darwin" ;;
            arm64 | aarch64) target="aarch64-apple-darwin" ;;
            *) target="" ;;
        esac
        ;;
    *)
        target=""
        ;;
esac

if [ -z "${target:-}" ]; then
    err "Cratebase does not currently publish a prebuilt binary for ${os}/${arch}.
See https://github.com/${REPO}/releases for manual options: build from
source with 'cargo build --release -p cratebase-server', or file an
issue requesting this target."
fi

# ---------------------------------------------------------------------------
# 2. Resolve the latest version
# ---------------------------------------------------------------------------
# GitHub's /releases/latest page redirects to /releases/tag/vX.Y.Z. Following
# that redirect (without downloading a body) is enough to learn the version,
# with no GitHub API call (no rate limit) and no JSON parsing.

release_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest") \
    || err "could not reach https://github.com/${REPO}/releases/latest"

version=${release_url##*/}
version=${version#v}

case "$release_url" in
    */tag/v*) : ;;
    *) err "unexpected redirect from GitHub releases/latest: ${release_url}" ;;
esac

[ -n "$version" ] || err "could not determine the latest Cratebase version"

name="cratebase_${version}_${target}"
zip_url="https://github.com/${REPO}/releases/download/v${version}/${name}.zip"
sha_url="${zip_url}.sha256"

# ---------------------------------------------------------------------------
# 3. Download and verify the checksum (non-negotiable, no bypass flag)
# ---------------------------------------------------------------------------

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT INT TERM

echo "Downloading Cratebase ${version} for ${target}..."

curl -fsSL -o "${tmpdir}/${name}.zip" "$zip_url" \
    || err "failed to download ${zip_url}"
curl -fsSL -o "${tmpdir}/${name}.zip.sha256" "$sha_url" \
    || err "failed to download ${sha_url}"

(
    cd "$tmpdir"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "${name}.zip.sha256"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "${name}.zip.sha256"
    else
        err "neither sha256sum nor shasum is available — cannot verify the download"
    fi
) || err "checksum verification failed — refusing to install a possibly-corrupted or tampered download"

# ---------------------------------------------------------------------------
# 4. Extract and install
# ---------------------------------------------------------------------------

(cd "$tmpdir" && unzip -q "${name}.zip") || err "failed to extract ${name}.zip"

# The binary is nested one directory deep inside the zip:
# cratebase_<version>_<target>/cratebase
bin_src="${tmpdir}/${name}/cratebase"
[ -f "$bin_src" ] || err "expected binary not found at ${bin_src} after extraction"

install_dir="${CRATEBASE_INSTALL:-$HOME/.local/bin}"
mkdir -p "$install_dir" || err "could not create ${install_dir}"

cp "$bin_src" "${install_dir}/cratebase"
chmod +x "${install_dir}/cratebase"

# ---------------------------------------------------------------------------
# 5. PATH check
# ---------------------------------------------------------------------------

on_path=0
case ":$PATH:" in
    *":${install_dir}:"*) on_path=1 ;;
esac

if [ "$on_path" = "0" ]; then
    echo ""
    echo "cratebase was installed to ${install_dir}/cratebase, but that directory"
    echo "is not on your PATH."
    echo ""
    echo "Add it to your shell profile:"
    echo "  echo 'export PATH=\"${install_dir}:\$PATH\"' >> ~/.bashrc   # bash"
    echo "  echo 'export PATH=\"${install_dir}:\$PATH\"' >> ~/.zshrc    # zsh"
    echo "Then restart your shell, or run:"
    echo "  export PATH=\"${install_dir}:\$PATH\""
fi

# ---------------------------------------------------------------------------
# 6. Success
# ---------------------------------------------------------------------------

echo ""
echo "Cratebase ${version} installed to ${install_dir}/cratebase"
echo ""
echo "Get started:"
echo "  cratebase serve"
echo ""
echo "Then open http://localhost:8090 in your browser — Cratebase will"
echo "walk you through creating your first superuser account right there,"
echo "no extra command needed."
echo ""
echo "(Prefer to script it? cratebase superuser create you@example.com yourpassword)"
