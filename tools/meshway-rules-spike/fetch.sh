#!/usr/bin/env bash
# Fetch the R0 spike inputs at pinned upstream commits into <spike-root> (private,
# not committed). Needs curl and, for sing-box `.srs` decompilation, a sing-box
# binary (spike only; the production converter reads `.srs` itself).
# Usage: fetch.sh <spike-root> <sing-box-binary>
set -euo pipefail
root=$1
singbox=$2
geosite=df6851d87aeaeb9d6a9464288b317e7ff2288d78 # SagerNet/sing-geosite rule-set
geoip=7fe82a879ad2666526730c195b55a6d8d9147908   # SagerNet/sing-geoip rule-set
meta=83c8208bdfae92b1caaa29745f51f233b6f1847e    # MetaCubeX/meta-rules-dat meta
bm7=19c84682407580056d0ae2f27e406f70db679de1     # blackmatrix7/ios_rule_script master
raw=https://raw.githubusercontent.com
mkdir -p "$root/data" "$root/extra"
cd "$root/data"
for f in geosite-cn geosite-openai 'geosite-geolocation-!cn'; do
  curl -sSfL -o "$f.srs" "$raw/SagerNet/sing-geosite/$geosite/$f.srs"
done
curl -sSfL -o geoip-cn.srs "$raw/SagerNet/sing-geoip/$geoip/geoip-cn.srs"
for f in *.srs; do "$singbox" rule-set decompile "$f" -o "${f%.srs}.json"; done
for f in cn openai 'geolocation-!cn'; do
  curl -sSfL -o "mihomo-geosite-$f.yaml" "$raw/MetaCubeX/meta-rules-dat/$meta/geo/geosite/$f.yaml"
  curl -sSfL -o "mihomo-geosite-$f.list" "$raw/MetaCubeX/meta-rules-dat/$meta/geo/geosite/$f.list"
done
curl -sSfL -o mihomo-geoip-cn.yaml "$raw/MetaCubeX/meta-rules-dat/$meta/geo/geoip/cn.yaml"
curl -sSfL -o mihomo-geoip-cn.list "$raw/MetaCubeX/meta-rules-dat/$meta/geo/geoip/cn.list"
cd "$root/extra"
for p in Global/Global.list China/China.list OpenAI/OpenAI.list Advertising/Advertising.list; do
  curl -sSfL -o "bm7-$(basename "$p")" "$raw/blackmatrix7/ios_rule_script/$bm7/rule/Clash/$p"
done
