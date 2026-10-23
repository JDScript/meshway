#!/bin/sh
# Build the Web Console against a Server on another origin and deploy it to
# Cloudflare Pages. See dist/console-pages/README.md.
#
#   deploy.sh                      # production branch (main) of the project
#   deploy.sh --branch preview     # a preview deployment (*.pages.dev only)
#   deploy.sh --build-only         # build, do not deploy
#
# MESHWAY_API_ORIGIN       Server origin the Console calls (VITE_MESHWAY_API_ORIGIN);
#                          that Server must run with --console-origin set to the
#                          Console's origin. Default https://meshway-prod-api.jdscript.dev
# MESHWAY_SERVER_URL       Server origin shown in the Node install commands
#                          (VITE_MESHWAY_SERVER_URL); default MESHWAY_API_ORIGIN
# VITE_MESHWAY_RELEASE     release tag for the install commands (optional)
# MESHWAY_PAGES_PROJECT    Pages project; default meshway-console
# WRANGLER                 wrangler command; default a pinned `pnpm dlx wrangler@...`
set -eu

WRANGLER_VERSION=4.152.0
PROJECT=${MESHWAY_PAGES_PROJECT:-meshway-console}
PRODUCTION_BRANCH=main
API_ORIGIN=${MESHWAY_API_ORIGIN:-https://meshway-prod-api.jdscript.dev}
SERVER_URL=${MESHWAY_SERVER_URL:-$API_ORIGIN}
WRANGLER=${WRANGLER:-pnpm dlx wrangler@$WRANGLER_VERSION}

branch=$PRODUCTION_BRANCH
deploy=1
while [ $# -gt 0 ]; do
  case $1 in
    --branch)
      [ $# -ge 2 ] || { echo "--branch needs a value" >&2; exit 2; }
      branch=$2
      shift 2
      ;;
    --build-only)
      deploy=0
      shift
      ;;
    -h | --help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

root=$(cd "$(dirname "$0")/../.." && pwd)
console=$root/apps/meshway-web-console
cd "$root"

# The build validates VITE_MESHWAY_API_ORIGIN and fails on a malformed value.
VITE_MESHWAY_API_ORIGIN=$API_ORIGIN \
  VITE_MESHWAY_SERVER_URL=$SERVER_URL \
  pnpm --filter @meshway/web-console build

[ -f "$console/dist/_redirects" ] || { echo "dist/_redirects is missing" >&2; exit 1; }
grep -qF "$API_ORIGIN" "$console"/dist/assets/*.js \
  || { echo "the build does not contain $API_ORIGIN" >&2; exit 1; }
echo "Built $console/dist for API origin $API_ORIGIN"
[ "$deploy" = 1 ] || exit 0

# The project is created once (direct upload, production branch main).
# Wrangler 4.15x would turn a new project into a Workers static-assets app;
# --force keeps it a Pages project (*.pages.dev, _redirects, Pages domains).
if ! $WRANGLER pages project list 2>/dev/null | grep -q "│ $PROJECT "; then
  $WRANGLER pages project create "$PROJECT" --production-branch "$PRODUCTION_BRANCH" --force
fi
commit=$(git -C "$root" rev-parse HEAD)
dirty=$(git -C "$root" status --porcelain | head -n 1)
$WRANGLER pages deploy "$console/dist" \
  --project-name "$PROJECT" \
  --branch "$branch" \
  --commit-hash "$commit" \
  --commit-message "meshway-console ${commit}${dirty:+ (dirty)}" \
  --commit-dirty="$([ -n "$dirty" ] && echo true || echo false)"
