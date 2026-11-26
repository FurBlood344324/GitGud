#!/usr/bin/env bash
# Runs one probe file against the real app, the same way applab/run.sh runs a
# spec: own Xvfb, own ports, own XDG sandbox, one fixture, and the debug binary
# as built. It exists because specs/*.mjs is the regression suite and must stay
# green: a probe here records the state of a suspicion (including a bug it is
# meant to catch), so it is run by hand, not on every lab run.
#
#   applab/probes/run-probe.sh visual-hunt           # applab/probes/visual-hunt.mjs
#   applab/probes/run-probe.sh visual-hunt --keep    # keep the work dir
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
binary="$root/src-tauri/target/debug/gitgud"
name="${1:?usage: run-probe.sh <probe-name> [--keep]}"
probe="$here/$name.mjs"
[ -f "$probe" ] || { echo "no probe at $probe" >&2; exit 2; }
[ -x "$binary" ] || { echo "no debug binary at $binary; run applab/run.sh --rebuild" >&2; exit 1; }

work="$(mktemp -d "${TMPDIR:-/tmp}/gitgud-probe.XXXXXX")"
display=""
driver_pid=""

cleanup() {
  status=$?
  trap '' PIPE
  trap - EXIT INT TERM HUP
  [ -n "$driver_pid" ] && kill -TERM "$driver_pid" 2>/dev/null
  if [ -n "$display" ]; then
    for pid in $(pgrep -f "$binary" 2>/dev/null || true); do
      tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null | grep -qx "DISPLAY=$display" &&
        kill -TERM "$pid" 2>/dev/null
    done
  fi
  [ -n "${native_port:-}" ] && pkill -f "WebKitWebDriver --port=$native_port" 2>/dev/null
  [ -n "$display" ] && pkill -f "Xvfb $display" 2>/dev/null
  [ "${LAB_KEEP:-0}" = 1 ] && echo "kept: $work (display $display)"
  echo "torn down"
  exit "$status"
}
trap cleanup EXIT INT TERM HUP PIPE

for n in $(seq 90 120); do
  [ ! -e "/tmp/.X11-unix/X$n" ] && { display=":$n"; break; }
done
[ -n "$display" ] || { echo "no free X display" >&2; exit 1; }

Xvfb "$display" -screen 0 1400x1000x24 -nolisten tcp >"$work/xvfb.log" 2>&1 &
for _ in $(seq 1 100); do [ -e "/tmp/.X11-unix/X${display#:}" ] && break; sleep 0.1; done
export DISPLAY="$display"
unset WAYLAND_DISPLAY
export GDK_BACKEND=x11

free_port() {
  for p in $(seq "$1" "$2"); do
    timeout -k 1 1 bash -c "exec 3<>/dev/tcp/127.0.0.1/$p" 2>/dev/null || { echo "$p"; return 0; }
  done
  return 1
}
driver_port="$(free_port 4444 4460)"
native_port="$(free_port 4461 4480)"
export LAB_APP_BINARY="$binary" LAB_DRIVER_PORT="$driver_port" LAB_WORK="$work" LAB_KEEP="${LAB_KEEP:-0}"
export LAB_REPO="$work/fixture"
export XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config"
mkdir -p "$LAB_REPO" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
"$root/applab/fixtures/make-repo.sh" "$LAB_REPO" >/dev/null

tauri-driver --port "$driver_port" --native-port "$native_port" >"$work/driver.log" 2>&1 &
driver_pid=$!
for _ in $(seq 1 100); do
  timeout -k 1 2 curl -sf "http://127.0.0.1:$driver_port/status" 2>/dev/null | grep -q '"ready":true' && break
  sleep 0.2
done

(cd "$root" && timeout -k 5 180 node "$probe")
