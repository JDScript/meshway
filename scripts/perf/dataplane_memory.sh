# Bounded diagnostic sampler in the disposable namespace. stdout retains the
# ordinary three-column CSV; stderr streams raw, timestamped attribution to the
# host, so evidence does not itself accumulate in the measured cgroup's tmpfs.
pid=$(pgrep -f '^/fixtures/meshway-dataplane-perf' | head -1)
i=0
while [ "$i" -lt "$1" ] && [ ! -e /tmp/perf-memory-stop ]; do
  read -r uptime rest < /proc/uptime
  read -r current < /sys/fs/cgroup/memory.current
  rss=null
  if [ -n "$pid" ] && [ -r /proc/"$pid"/status ]; then
    rss=$(awk '/^VmRSS:/ {print $2 * 1024}' /proc/"$pid"/status)
  fi
  printf '%s,%s,%s\n' "$uptime" "$current" "${rss:-null}"
  if [ "$2" = 1 ]; then
    printf 'sample %s %s\n' "$uptime" "$current" >&2
    cat /sys/fs/cgroup/memory.stat >&2
    for status in /proc/[0-9]*/status; do
      process=${status%/status}
      printf 'process %s\n' "${process##*/}" >&2
      awk '/^(Name|Pid|PPid|VmSize|VmRSS|RssAnon|RssFile|RssShmem|Threads):/' "$status" >&2
      cat "$process/smaps_rollup" >&2
    done
    printf 'end_sample\n' >&2
  fi
  i=$((i+1))
  sleep 0.25
done
