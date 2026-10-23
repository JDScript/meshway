#!/bin/sh
# Install the soak collector on this host (kr-icn-arm64), idempotently. Run from a copy of scripts/soak/.
#
#   sh install.sh [--root DIR] [--no-cron]   copy scripts to DIR/bin, create directories (0700), create the
#                                            collector key if missing, render gate specs, print the authorized_keys
#                                            lines and gate install commands per remote host, install the cron block
#   sh install.sh --uninstall [--root DIR]   remove only the cron block (data, key and scripts stay)
#
# Nothing is written to any other host: the printed commands are for the coordinator to run.
set -eu
ROOT=$HOME/meshway-soak
CRON=1
UNINSTALL=0
while [ $# -gt 0 ]; do
  case $1 in
    --root) ROOT=$2; shift 2 ;;
    --no-cron) CRON=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
SRC=$(cd "$(dirname "$0")" && pwd)
say() { echo "meshway-soak install: $*"; }

if [ $UNINSTALL = 1 ]; then
  (crontab -l 2>/dev/null || true) | python3 "$SRC/render.py" replace-block - | crontab -
  say "cron block removed; $ROOT kept"
  exit 0
fi

command -v python3 >/dev/null || { say "python3 missing"; exit 1; }
command -v zstd >/dev/null || python3 -c 'import compression.zstd' 2>/dev/null || { say "zstd missing (apt-get install zstd)"; exit 1; }
command -v flock >/dev/null || { say "flock missing (util-linux)"; exit 1; }
[ "$(date +%Z)" = UTC ] || say "WARNING: system time zone is $(date +%Z); cron times are read in it, the schedule assumes UTC"

umask 077
mkdir -p "$ROOT/bin" "$ROOT/keys" "$ROOT/state" "$ROOT/samples" "$ROOT/logs" "$ROOT/reports" "$ROOT/gate"
chmod 700 "$ROOT" "$ROOT/keys" "$ROOT/gate"
if [ "$SRC" != "$ROOT/bin" ]; then
  for f in "$SRC"/*.py "$SRC"/install.sh "$SRC"/config.example.json "$SRC"/README.md; do
    install -m 0644 "$f" "$ROOT/bin/"
  done
fi
say "scripts in $ROOT/bin"

KEY=$ROOT/keys/collector_ed25519
if [ -f "$KEY" ]; then say "key $KEY exists (kept)"; else
  ssh-keygen -q -t ed25519 -N '' -C meshway-soak-collector -f "$KEY"; say "key $KEY created"; fi

if [ ! -f "$ROOT/config.json" ]; then
  install -m 0600 "$ROOT/bin/config.example.json" "$ROOT/config.json"
  say "wrote $ROOT/config.json from the example: fill every REPLACE, then run this again"
  exit 0
fi
chmod 600 "$ROOT/config.json"
python3 "$ROOT/bin/render.py" --config "$ROOT/config.json" gates "$ROOT/gate" >/dev/null
say "gate specs in $ROOT/gate"
echo
python3 "$ROOT/bin/render.py" --config "$ROOT/config.json" keys "$KEY.pub" "$ROOT/bin" "$ROOT/gate"

if [ $CRON = 1 ]; then
  BLOCK=$(mktemp)
  python3 "$ROOT/bin/render.py" --config "$ROOT/config.json" crontab "$ROOT" >"$BLOCK"
  (crontab -l 2>/dev/null || true) | python3 "$ROOT/bin/render.py" replace-block "$BLOCK" | crontab -
  rm -f "$BLOCK"
  say "cron block installed (crontab -l shows it between # BEGIN/# END meshway-soak)"
else
  say "cron not touched (--no-cron)"
fi
