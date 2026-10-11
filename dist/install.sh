#!/bin/sh
# Install, upgrade or remove the Meshway Node (meshwayd + meshway) as a systemd
# service on Linux x86_64/aarch64. See docs/v2/install.md.
#
#   install.sh --server https://controller.example [--version vX.Y.Z]
#   install.sh --artifact ./meshway-aarch64-unknown-linux-musl.tar.gz --server ...
#   install.sh --uninstall [--purge]
#
# Private repositories: export GITHUB_TOKEN (read access to releases).
set -eu

REPO=${MESHWAY_REPO:-JDScript/meshway}
BIN_DIR=/usr/local/bin
UNIT_PATH=/etc/systemd/system/meshwayd.service
CONFIG_DIR=/etc/meshway
ENV_PATH=$CONFIG_DIR/meshwayd.env
DATA_DIR=/var/lib/meshway
RUNTIME_DIR=/run/meshway
SOCKET=$DATA_DIR/control.sock

version=""
artifact=""
server=""
options=""
options_set=0
action=install
purge=0
start=1
work=""

usage() {
	cat <<'EOF'
Usage: install.sh [options]

Install or upgrade:
  --version TAG       Release tag to install (default: latest stable release)
  --artifact PATH     Install a local release tarball or directory instead of downloading;
                      a SHA256SUMS file next to a tarball is verified
  --server URL        Server origin written to /etc/meshway/meshwayd.env
                      (required on first install; kept on upgrade when omitted)
  --daemon-options S  Extra meshwayd arguments (MESHWAYD_OPTIONS), e.g. "--hostname web-1"
  --no-start          Install and enable without starting or restarting the service

Remove:
  --uninstall         Run `meshway down`, stop and remove the service, unit and binaries
  --purge             With --uninstall, also delete /var/lib/meshway (Node identity),
                      /etc/meshway and drop-ins in /etc/systemd/system/meshwayd.service.d

Environment: MESHWAY_REPO (default JDScript/meshway), GITHUB_TOKEN for private releases.
EOF
}

log() { printf 'meshway-install: %s\n' "$*" >&2; }
die() {
	log "error: $*"
	exit 1
}
cleanup() { if [ -n "$work" ]; then rm -rf "$work"; fi; }
trap cleanup EXIT
trap 'exit 130' INT TERM

while [ $# -gt 0 ]; do
	case $1 in
	--version) [ $# -ge 2 ] || die "--version needs a value"; version=$2; shift 2 ;;
	--artifact) [ $# -ge 2 ] || die "--artifact needs a value"; artifact=$2; shift 2 ;;
	--server) [ $# -ge 2 ] || die "--server needs a value"; server=$2; shift 2 ;;
	--daemon-options) [ $# -ge 2 ] || die "--daemon-options needs a value"; options=$2; options_set=1; shift 2 ;;
	--no-start) start=0; shift ;;
	--uninstall) action=uninstall; shift ;;
	--purge) purge=1; shift ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; die "unknown option: $1" ;;
	esac
done

[ "$(id -u)" = 0 ] || die "run as root (sudo)"
[ "$(uname -s)" = Linux ] || die "only Linux is supported"
if [ ! -d /run/systemd/system ] || ! command -v systemctl >/dev/null; then die "systemd is required"; fi
[ $purge = 0 ] || [ $action = uninstall ] || die "--purge requires --uninstall"
[ -z "$version" ] || [ -z "$artifact" ] || die "--version and --artifact are exclusive"

case $(uname -m) in
x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
*) die "unsupported architecture $(uname -m); supported: x86_64, aarch64" ;;
esac
asset=meshway-$target.tar.gz

daemon_active() { systemctl is-active --quiet meshwayd.service; }

uninstall() {
	if daemon_active; then
		# Explicit down removes the TUN, DNS anchor and negative capture boundary;
		# stopping alone keeps them for the next start.
		log "meshway down (cleans owned network resources)"
		"$BIN_DIR/meshway" --socket "$SOCKET" down ||
			die "meshway down failed; the service is left running. Inspect 'meshway status', resolve the reported problem and retry"
		systemctl disable --now meshwayd.service
	elif [ -f "$UNIT_PATH" ]; then
		systemctl disable meshwayd.service 2>/dev/null || true
		if [ -d "$RUNTIME_DIR" ] && [ -n "$(ls -A "$RUNTIME_DIR" 2>/dev/null)" ]; then
			log "warning: the service was not running; resources kept for a later start (DNS anchor," \
				"capture rules) were not removed. Start it and rerun --uninstall for a clean removal"
		fi
	fi
	rm -f "$UNIT_PATH" "$BIN_DIR/meshwayd" "$BIN_DIR/meshway"
	systemctl daemon-reload
	systemctl reset-failed meshwayd.service 2>/dev/null || true
	if daemon_active; then die "meshwayd is still active"; fi
	rm -rf "$RUNTIME_DIR"
	if [ $purge = 1 ]; then
		rm -rf "$DATA_DIR" "$CONFIG_DIR" "$UNIT_PATH.d"
		systemctl daemon-reload
		log "removed Node identity and state ($DATA_DIR), $CONFIG_DIR and unit drop-ins"
	else
		log "kept Node identity and state in $DATA_DIR and settings in $CONFIG_DIR (--purge removes them)"
	fi
	log "uninstalled"
}

fetch() { # url output [accept]
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		curl -fsSL --proto '=https' -H "Authorization: Bearer $GITHUB_TOKEN" \
			-H "Accept: ${3:-application/vnd.github+json}" -o "$2" "$1"
	else
		curl -fsSL --proto '=https' -o "$2" "$1"
	fi
}

download() { # asset-name output
	if [ -n "${GITHUB_TOKEN:-}" ]; then
		# The API is the only download path for private releases.
		if [ -n "$version" ]; then release=tags/$version; else release=latest; fi
		fetch "https://api.github.com/repos/$REPO/releases/$release" "$work/release.json" ||
			die "cannot read release ${version:-latest} of $REPO"
		url=$(tr ',{}' '\n' <"$work/release.json" | sed -n \
			-e 's/^ *"url": *"\(https:\/\/api\.github\.com\/[^"]*\/releases\/assets\/[0-9]*\)"$/\1/p' \
			-e 's/^ *"name": *"\([^"]*\)"$/name \1/p' |
			awk -v want="$1" '/^https:/ { url = $0; next } $1 == "name" && $2 == want { print url; exit }')
		[ -n "$url" ] || die "release ${version:-latest} has no asset $1"
		fetch "$url" "$2" application/octet-stream || die "cannot download $1"
	else
		if [ -n "$version" ]; then base=https://github.com/$REPO/releases/download/$version; else
			base=https://github.com/$REPO/releases/latest/download; fi
		fetch "$base/$1" "$2" || die "cannot download $base/$1 (private repository: export GITHUB_TOKEN)"
	fi
}

verify() { # directory-with-SHA256SUMS file-name
	line=$(awk -v f="$2" '$2 == f || $2 == "*" f { print; exit }' "$1/SHA256SUMS")
	[ -n "$line" ] || die "SHA256SUMS has no entry for $2"
	(cd "$1" && printf '%s\n' "$line" | sha256sum -c --status) || die "checksum mismatch for $2"
	log "verified $2 sha256 $(printf '%s' "$line" | cut -c1-16)"
}

stage() { # -> $work/payload with meshwayd, meshway, meshwayd.service
	payload=$work/payload
	mkdir -p "$payload"
	if [ -n "$artifact" ]; then
		if [ -d "$artifact" ]; then
			cp "$artifact/meshwayd" "$artifact/meshway" "$payload/" || die "$artifact lacks meshwayd/meshway"
			[ ! -f "$artifact/meshwayd.service" ] || cp "$artifact/meshwayd.service" "$payload/"
		else
			[ -f "$artifact" ] || die "no such artifact: $artifact"
			sums=$(dirname "$artifact")/SHA256SUMS
			if [ -f "$sums" ]; then
				cp "$sums" "$work/SHA256SUMS"
				cp "$artifact" "$work/$(basename "$artifact")"
				verify "$work" "$(basename "$artifact")"
			else
				log "warning: no SHA256SUMS next to $artifact; local artifact not verified"
			fi
			tar -xzf "$artifact" -C "$payload" meshwayd meshway meshwayd.service ||
				die "$artifact is not a Meshway release tarball"
		fi
	else
		command -v curl >/dev/null || die "curl is required for downloads (or use --artifact)"
		download SHA256SUMS "$work/SHA256SUMS"
		download "$asset" "$work/$asset"
		verify "$work" "$asset"
		tar -xzf "$work/$asset" -C "$payload" meshwayd meshway meshwayd.service ||
			die "$asset is not a Meshway release tarball"
	fi
	if [ ! -f "$payload/meshwayd.service" ]; then
		unit=$(dirname "$0")/systemd/meshwayd.service
		[ -f "$unit" ] || die "no meshwayd.service in the artifact or next to this script"
		cp "$unit" "$payload/meshwayd.service"
	fi
	chmod 0755 "$payload/meshwayd" "$payload/meshway"
	"$payload/meshway" version >/dev/null 2>&1 || die "the CLI does not run on this machine (wrong architecture?)"
	"$payload/meshwayd" --version >/dev/null 2>&1 || die "the daemon does not run on this machine"
}

same() { [ -f "$2" ] && cmp -s "$1" "$2"; }

place() { # source destination mode
	tmp=$(dirname "$2")/.$(basename "$2").new
	cp "$1" "$tmp" && chmod "$3" "$tmp" && mv -f "$tmp" "$2"
}

install_node() {
	work=$(mktemp -d)
	stage
	payload=$work/payload
	installed=$("$payload/meshway" version | head -n 1)
	if [ -z "$server" ] && [ ! -f "$ENV_PATH" ]; then
		die "--server is required on first install"
	fi
	changed=0
	for binary in meshwayd meshway; do
		if ! same "$payload/$binary" "$BIN_DIR/$binary"; then
			place "$payload/$binary" "$BIN_DIR/$binary" 0755
			changed=1
		fi
	done
	if ! same "$payload/meshwayd.service" "$UNIT_PATH"; then
		place "$payload/meshwayd.service" "$UNIT_PATH" 0644
		changed=1
	fi
	install -d -m 0755 "$CONFIG_DIR"
	if [ -n "$server" ] || [ $options_set = 1 ]; then
		if [ -z "$server" ]; then
			server=$(sed -n 's/^MESHWAY_SERVER_URL=//p' "$ENV_PATH")
		fi
		if [ $options_set = 0 ] && [ -f "$ENV_PATH" ]; then
			options=$(sed -n 's/^MESHWAYD_OPTIONS=//p' "$ENV_PATH")
		fi
		case $server in https://* | http://127.* | http://\[::1\]*) ;; *) die "--server must be an https:// origin" ;; esac
		printf 'MESHWAY_SERVER_URL=%s\nMESHWAYD_OPTIONS=%s\n' "$server" "$options" >"$work/meshwayd.env"
		if ! same "$work/meshwayd.env" "$ENV_PATH"; then
			place "$work/meshwayd.env" "$ENV_PATH" 0644
			changed=1
		fi
	fi
	systemctl daemon-reload
	systemctl enable meshwayd.service >/dev/null
	if [ $start = 1 ]; then
		if ! daemon_active; then
			systemctl start meshwayd.service
		elif [ $changed = 1 ]; then
			log "restarting meshwayd (Node intent and identity are kept)"
			systemctl restart meshwayd.service
		fi
		daemon_active || die "meshwayd did not become ready; see journalctl -u meshwayd"
	fi
	if [ $changed = 1 ]; then log "installed $installed"; else log "$installed already installed"; fi
	if [ $start = 1 ] && ! "$BIN_DIR/meshway" --socket "$SOCKET" status --json 2>/dev/null | grep -q '"desiredUp":true'; then
		log "next: meshway up (browser approval) or meshway up --enrollment-key @/path/to/key"
	fi
}

case $action in
uninstall) uninstall ;;
install) install_node ;;
esac
