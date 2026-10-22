#!/bin/sh
# Install, upgrade or remove a Meshway component as a systemd service on Linux
# x86_64/aarch64: the Node (meshwayd + meshway, default) or the Server
# (meshway-server). See docs/v2/install.md.
#
#   install.sh --server https://controller.example [--version vX.Y.Z]
#   install.sh --artifact ./meshway-aarch64-unknown-linux-musl.tar.gz --server ...
#   install.sh --component server [--version vX.Y.Z | --artifact PATH]
#   install.sh [--component server] --uninstall [--purge]
#
# Private repositories: export GITHUB_TOKEN (read access to releases).
set -eu

REPO=${MESHWAY_REPO:-JDScript/meshway}
BIN_DIR=/usr/local/bin

# Node
UNIT_PATH=/etc/systemd/system/meshwayd.service
CONFIG_DIR=/etc/meshway
ENV_PATH=$CONFIG_DIR/meshwayd.env
DATA_DIR=/var/lib/meshway
RUNTIME_DIR=/run/meshway
SOCKET=$DATA_DIR/control.sock

# Server
SERVER_USER=meshway-server
SERVER_UNIT=meshway-server.service
SERVER_UNIT_PATH=/etc/systemd/system/$SERVER_UNIT
SERVER_CONFIG_DIR=/etc/meshway-server
SERVER_ENV_PATH=$SERVER_CONFIG_DIR/server.env
SERVER_DATA_DIR=/var/lib/meshway-server
SERVER_TOOLS_DIR=/usr/local/libexec/meshway-server
SERVER_TOOLS="ip-datasets reset-password"
# PRAGMA application_id of a Server database ("MSHW").
SERVER_APPLICATION_ID=1297303639

component=node
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

Component:
  --component NAME    node (default: meshwayd and meshway) or server (meshway-server)

Install or upgrade:
  --version TAG       Release tag to install (default: latest stable release)
  --artifact PATH     Install a local release tarball or directory instead of downloading;
                      a SHA256SUMS file next to a tarball is verified
  --no-start          Install and enable without starting or restarting the service

Node only:
  --server URL        Server origin written to /etc/meshway/meshwayd.env
                      (required on first install; kept on upgrade when omitted)
  --daemon-options S  Extra meshwayd arguments (MESHWAYD_OPTIONS), e.g. "--hostname web-1"

Server upgrades check the data directory's schema before stopping the running
Server, migrate it with the new build, keep the replaced program as
<name>.previous and roll back automatically while no Node has synced with the
new Server. Settings live in /etc/meshway-server/server.env (created once).

Remove:
  --uninstall         Node: run `meshway down`, then stop and remove the service, unit and
                      binaries. Server: stop and remove the service, unit and programs
  --purge             With --uninstall, also delete the state directory and settings:
                      Node identity (/var/lib/meshway, /etc/meshway); Server database and
                      master key (/var/lib/meshway-server, /etc/meshway-server) and its user.
                      Drop-ins in /etc/systemd/system/<unit>.d go too

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
	--component) [ $# -ge 2 ] || die "--component needs a value"; component=$2; shift 2 ;;
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
case $component in
node) ;;
server)
	[ -z "$server" ] && [ $options_set = 0 ] || die "--server and --daemon-options are Node options"
	;;
*) die "unknown component $component; supported: node, server" ;;
esac

case $(uname -m) in
x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
*) die "unsupported architecture $(uname -m); supported: x86_64, aarch64" ;;
esac

daemon_active() { systemctl is-active --quiet meshwayd.service; }
server_active() { systemctl is-active --quiet $SERVER_UNIT; }

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

stage() { # asset unit program... -> $work/payload with the programs and the unit
	stage_asset=$1
	stage_unit=$2
	shift 2
	payload=$work/payload
	mkdir -p "$payload"
	if [ -n "$artifact" ]; then
		if [ -d "$artifact" ]; then
			for program in "$@"; do
				cp "$artifact/$program" "$payload/" || die "$artifact lacks $program"
			done
			[ ! -f "$artifact/$stage_unit" ] || cp "$artifact/$stage_unit" "$payload/"
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
			tar -xzf "$artifact" -C "$payload" "$@" "$stage_unit" ||
				die "$artifact is not a Meshway release tarball"
		fi
	else
		command -v curl >/dev/null || die "curl is required for downloads (or use --artifact)"
		download SHA256SUMS "$work/SHA256SUMS"
		download "$stage_asset" "$work/$stage_asset"
		verify "$work" "$stage_asset"
		tar -xzf "$work/$stage_asset" -C "$payload" "$@" "$stage_unit" ||
			die "$stage_asset is not a Meshway release tarball"
	fi
	if [ ! -f "$payload/$stage_unit" ]; then
		unit=$(dirname "$0")/systemd/$stage_unit
		[ -f "$unit" ] || die "no $stage_unit in the artifact or next to this script"
		cp "$unit" "$payload/$stage_unit"
	fi
	for program in "$@"; do chmod 0755 "$payload/$program"; done
}

same() { [ -f "$2" ] && cmp -s "$1" "$2"; }

place() { # source destination mode
	tmp=$(dirname "$2")/.$(basename "$2").new
	cp "$1" "$tmp" && chmod "$3" "$tmp" && mv -f "$tmp" "$2"
}

install_node() {
	work=$(mktemp -d)
	stage "meshway-$target.tar.gz" meshwayd.service meshwayd meshway
	"$payload/meshway" version >/dev/null 2>&1 || die "the CLI does not run on this machine (wrong architecture?)"
	"$payload/meshwayd" --version >/dev/null 2>&1 || die "the daemon does not run on this machine"
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

# --- Server -----------------------------------------------------------------

server_programs() { printf '%s %s\n' "$BIN_DIR/meshway-server" "$(for t in $SERVER_TOOLS; do printf '%s/%s ' "$SERVER_TOOLS_DIR" "$t"; done)"; }

uninstall_server() {
	systemctl disable --now $SERVER_UNIT 2>/dev/null || true
	rm -f "$SERVER_UNIT_PATH"
	for program in $(server_programs); do rm -f "$program" "$program.previous"; done
	rmdir "$SERVER_TOOLS_DIR" 2>/dev/null || true
	systemctl daemon-reload
	systemctl reset-failed $SERVER_UNIT 2>/dev/null || true
	if server_active; then die "meshway-server is still active"; fi
	if [ $purge = 1 ]; then
		rm -rf "$SERVER_DATA_DIR" "$SERVER_CONFIG_DIR" "$SERVER_UNIT_PATH.d"
		if id -u $SERVER_USER >/dev/null 2>&1; then userdel $SERVER_USER; fi
		systemctl daemon-reload
		log "removed the Server database and master key ($SERVER_DATA_DIR), $SERVER_CONFIG_DIR," \
			"unit drop-ins and the $SERVER_USER user"
	else
		log "kept the Server database and master key in $SERVER_DATA_DIR and settings in" \
			"$SERVER_CONFIG_DIR (--purge removes them)"
	fi
	log "uninstalled"
}

# Big-endian 32-bit integer at byte offset $2 of the SQLite header of file $1.
header_u32() {
	# shellcheck disable=SC2046
	set -- $(od -An -tu1 -j "$2" -N 4 "$1")
	[ $# = 4 ] || return 1
	echo $((($1 << 24) | ($2 << 16) | ($3 << 8) | $4))
}

server_env() { # name default -> value from server.env
	value=$(sed -n "s/^$1=//p" "$SERVER_ENV_PATH" 2>/dev/null | tail -n 1)
	printf '%s\n' "${value:-$2}"
}

server_healthy() { # The listener answers the unauthenticated auth options.
	listen=$(server_env MESHWAY_LISTEN 127.0.0.1:8420)
	port=${listen##*:}
	host=${listen%:*}
	case $host in 0.0.0.0) host=127.0.0.1 ;; \[::\]) host='[::1]' ;; esac
	attempt=0
	while [ $attempt -lt 10 ]; do
		if curl -fsS --max-time 3 -o /dev/null "http://$host:$port/api/v1/auth/options"; then return 0; fi
		attempt=$((attempt + 1))
		sleep 1
	done
	return 1
}

server_synced_since() { # A Node holds a lease of a Server process started since $1.
	journalctl -u $SERVER_UNIT --since "@$1" -o cat 2>/dev/null | grep -q 'First Node sync answered'
}

server_start() { # -> 0 when ready (READY=1) and healthy
	systemctl start $SERVER_UNIT && server_active && server_healthy
}

write_server_env() {
	install -d -m 0755 "$SERVER_CONFIG_DIR"
	[ ! -f "$SERVER_ENV_PATH" ] || return 0
	cat >"$work/server.env" <<'EOF'
# Meshway Server instance settings, read by meshway-server.service; see
# docs/v2/server.md. install.sh creates this file once and never rewrites it.
MESHWAY_LISTEN=127.0.0.1:8420
#MESHWAY_CONSOLE_ORIGIN=https://console.example.com
#MESHWAY_PUBLIC_ORIGIN=https://meshway.example.com
#MESHWAY_REGISTRATION_ENABLED=true
# Further arguments, e.g. --stun-listen 0.0.0.0:3478 --stun-server main=meshway.example.com:3478
MESHWAY_SERVER_OPTIONS=
EOF
	place "$work/server.env" "$SERVER_ENV_PATH" 0644
	log "created $SERVER_ENV_PATH; set MESHWAY_CONSOLE_ORIGIN and the listen address there"
}

install_server() {
	for tool in curl od runuser useradd; do
		command -v $tool >/dev/null || die "$tool is required"
	done
	work=$(mktemp -d)
	# shellcheck disable=SC2086
	stage "meshway-server-$target.tar.gz" $SERVER_UNIT meshway-server $SERVER_TOOLS
	"$payload/meshway-server" --version >"$work/version" 2>/dev/null ||
		die "meshway-server does not run on this machine (wrong architecture?)"
	installed=$(head -n 1 "$work/version")
	range=$(sed -n 's/^schema: \([0-9][0-9]*\) (migrates from \([0-9][0-9]*\))$/\1 \2/p' "$work/version")
	[ -n "$range" ] || die "$installed does not report its schema range"
	schema=${range% *}
	oldest=${range#* }

	# Preflight, before anything stops: the database must be one this build
	# opens or migrates, with room for the migration.
	database=$SERVER_DATA_DIR/controller.sqlite
	from=""
	if [ -s "$database" ]; then
		application=$(header_u32 "$database" 68) || die "cannot read $database"
		from=$(header_u32 "$database" 60) || die "cannot read $database"
		[ "$application" = $SERVER_APPLICATION_ID ] || die "$database is not a Meshway Server database"
		[ "$from" -le "$schema" ] ||
			die "$database has schema $from, newer than $installed writes ($schema); nothing was changed"
		[ "$from" -ge "$oldest" ] ||
			die "$database has schema $from; $installed migrates from schema $oldest on; nothing was changed"
		if [ "$from" -lt "$schema" ]; then
			size=$(wc -c <"$database")
			free=$(df -Pk "$SERVER_DATA_DIR" | awk 'NR == 2 { print $4 }')
			[ $((free * 1024)) -ge $((size + size / 10)) ] ||
				die "not enough free space in $SERVER_DATA_DIR to migrate $size bytes; nothing was changed"
		fi
	fi

	if ! id -u $SERVER_USER >/dev/null 2>&1; then
		useradd --system --user-group --home-dir "$SERVER_DATA_DIR" --no-create-home \
			--shell /usr/sbin/nologin $SERVER_USER
		log "created system user $SERVER_USER"
	fi
	write_server_env
	install -d -m 0755 "$SERVER_TOOLS_DIR"

	changed=0
	same "$payload/meshway-server" "$BIN_DIR/meshway-server" || changed=1
	for tool in $SERVER_TOOLS; do same "$payload/$tool" "$SERVER_TOOLS_DIR/$tool" || changed=1; done
	same "$payload/$SERVER_UNIT" "$SERVER_UNIT_PATH" || changed=1
	if [ $changed = 0 ]; then
		systemctl enable $SERVER_UNIT >/dev/null
		if [ $start = 1 ] && ! server_active; then
			server_start || die "meshway-server did not become ready; see journalctl -u meshway-server"
		fi
		log "$installed already installed"
		return
	fi

	upgrade=0
	[ ! -f "$BIN_DIR/meshway-server" ] || upgrade=1
	was_active=0
	! server_active || was_active=1
	started=$(date +%s)
	# The new program must be readable by the service user for the migration.
	place "$payload/meshway-server" "$BIN_DIR/meshway-server.new" 0755
	if [ $was_active = 1 ]; then
		log "stopping meshway-server (Nodes keep their leases meanwhile)"
		systemctl stop $SERVER_UNIT
	fi
	backup=""
	if [ -n "$from" ]; then
		if ! runuser -u $SERVER_USER -- "$BIN_DIR/meshway-server.new" migrate \
			--data-dir "$SERVER_DATA_DIR" >"$work/migrate" 2>"$work/migrate.log"; then
			cat "$work/migrate.log" >&2
			rm -f "$BIN_DIR/meshway-server.new"
			if [ $was_active = 1 ]; then systemctl start $SERVER_UNIT || true; fi
			die "the migration failed; the database is unchanged and the installed Server was started again"
		fi
		cat "$work/migrate" >&2
		backup=$(sed -n 's/^Database migrated .* the previous database is kept as \(.*\)\.$/\1/p' "$work/migrate")
	fi

	# Replace: the running programs stay as <name>.previous.
	if [ $upgrade = 1 ]; then
		cp -p "$BIN_DIR/meshway-server" "$BIN_DIR/meshway-server.previous"
		for tool in $SERVER_TOOLS; do
			[ ! -f "$SERVER_TOOLS_DIR/$tool" ] || cp -p "$SERVER_TOOLS_DIR/$tool" "$SERVER_TOOLS_DIR/$tool.previous"
		done
		[ ! -f "$SERVER_UNIT_PATH" ] || cp -p "$SERVER_UNIT_PATH" "$work/previous.service"
	fi
	mv -f "$BIN_DIR/meshway-server.new" "$BIN_DIR/meshway-server"
	for tool in $SERVER_TOOLS; do place "$payload/$tool" "$SERVER_TOOLS_DIR/$tool" 0755; done
	place "$payload/$SERVER_UNIT" "$SERVER_UNIT_PATH" 0644
	systemctl daemon-reload
	systemctl enable $SERVER_UNIT >/dev/null
	if [ $start = 0 ]; then
		log "installed $installed (not started)"
		return
	fi
	if server_start; then
		if [ $upgrade = 1 ]; then
			log "upgraded to $installed; the previous program is kept as $BIN_DIR/meshway-server.previous${backup:+, the previous database as $backup}"
		else
			log "installed $installed"
		fi
		return
	fi
	if [ $upgrade = 0 ]; then
		die "meshway-server did not become ready; see journalctl -u meshway-server"
	fi
	if server_synced_since "$started"; then
		die "$installed did not become healthy after a Node synced with it: not rolled back (Nodes now" \
			"hold its netmaps). Fix forward; see journalctl -u meshway-server"
	fi
	log "$installed did not become ready and healthy; rolling back"
	systemctl stop $SERVER_UNIT || true
	if server_synced_since "$started"; then
		die "a Node synced with $installed while it was stopped: not rolled back; fix forward"
	fi
	mv -f "$BIN_DIR/meshway-server.previous" "$BIN_DIR/meshway-server"
	for tool in $SERVER_TOOLS; do
		[ ! -f "$SERVER_TOOLS_DIR/$tool.previous" ] || mv -f "$SERVER_TOOLS_DIR/$tool.previous" "$SERVER_TOOLS_DIR/$tool"
	done
	[ ! -f "$work/previous.service" ] || place "$work/previous.service" "$SERVER_UNIT_PATH" 0644
	if [ -n "$backup" ]; then
		# The migrated database and its journal are kept aside, never replayed
		# onto the restored one.
		aside=$SERVER_DATA_DIR/controller.sqlite.failed-$started
		for suffix in "" -wal -shm; do
			[ ! -e "$database$suffix" ] || mv "$database$suffix" "$aside$suffix"
		done
		mv "$backup" "$database"
		log "restored the database from $backup; the migrated one is kept as $aside"
	fi
	systemctl daemon-reload
	server_start || die "the previous Server did not start again either; see journalctl -u meshway-server"
	die "rolled back to $("$BIN_DIR/meshway-server" -V 2>/dev/null || echo 'the previous Server'); $installed was not installed"
}

case $component.$action in
node.uninstall) uninstall ;;
node.install) install_node ;;
server.uninstall) uninstall_server ;;
server.install) install_server ;;
esac
