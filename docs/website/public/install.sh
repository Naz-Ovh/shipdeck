#!/bin/sh
# Installs shipdeck: the latest release, or the one -V names.
#
#   curl -LsSf https://shipdeck.naz.ovh/install.sh | sh
#   curl -LsSf https://shipdeck.naz.ovh/install.sh | sh -s -- -V=v0.3.1
#
# It runs the installer published with that release on GitHub, which picks
# the build for this machine (on Linux: gnu with glibc 2.28 or newer, static
# musl otherwise), checks its SHA-256 and puts shipdeck in ~/.local/bin.
# Every argument other than -V goes on to that installer.
#
# Everything is inside main, so a download cut short runs nothing.

main() {
  set -eu
  releases=https://github.com/Naz-Ovh/shipdeck/releases
  version=

  # take -V out of the arguments; the rest go to the release installer
  n=$#
  while [ "$n" -gt 0 ]; do
    arg=$1
    shift
    n=$((n - 1))
    case $arg in
      -V=* | --version=*) version=${arg#*=} ;;
      -V | --version)
        if [ "$n" -eq 0 ]; then
          echo "error: $arg needs a version, such as v0.3.1" >&2
          exit 1
        fi
        version=$1
        shift
        n=$((n - 1))
        ;;
      -h | --help)
        cat <<'EOF'
Installs shipdeck from its GitHub releases.

  -V, --version <version>  install this release, such as v0.3.1 (default: the latest)
  --no-modify-path         leave ~/.profile alone
  -q, --quiet              print errors only

SHIPDECK_INSTALL_DIR sets the folder the binary goes into.
EOF
        exit 0
        ;;
      *) set -- "$@" "$arg" ;;
    esac
  done

  if [ -n "$version" ]; then
    version=v${version#v}
    case ${version#v} in
      [0-9]*) ;;
      *) version=bad ;;
    esac
    case ${version#v} in
      *[!0-9A-Za-z.-]*) version=bad ;;
    esac
    if [ "$version" = bad ]; then
      echo "error: -V takes a version such as v0.3.1" >&2
      exit 1
    fi
    url=$releases/download/$version/shipdeck-installer.sh
  else
    url=$releases/latest/download/shipdeck-installer.sh
  fi

  if command -v curl >/dev/null 2>&1; then
    installer=$(curl --proto '=https' --tlsv1.2 -fsSL "$url")
  elif command -v wget >/dev/null 2>&1; then
    installer=$(wget -qO- "$url")
  else
    echo "error: installing shipdeck needs curl or wget" >&2
    exit 1
  fi || {
    echo "error: could not download $url" >&2
    if [ -n "$version" ]; then
      echo "Is $version a shipdeck release? They are listed at $releases" >&2
    fi
    exit 1
  }

  printf '%s\n' "$installer" | sh -s -- "$@"
}

main "$@"
