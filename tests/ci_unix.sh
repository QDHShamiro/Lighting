#!/usr/bin/env bash
set -u
CLIENT="$1"
LOG="$RUNNER_TEMP/lighting-ci.log"
SHOTS="$RUNNER_TEMP/shots"
mkdir -p "$SHOTS"
: > "$LOG"
chmod +x "bin/$CLIENT"
L="$PWD/bin/$CLIENT"

say() { printf '%s\n' "$*" | tee -a "$LOG"; }
run() {
  local name="$1"; shift
  say "== $name"
  local out
  out=$(perl -e "alarm shift; exec @ARGV" 600 "$@" 2>&1)
  local code=$?
  printf '%s\n' "$out" | tee -a "$LOG"
  say "-- exit $code"
  return 0
}

if [ "$(uname)" = Linux ]; then
  export DISPLAY=:99 XDG_SESSION_TYPE=x11 GTK_MODULES=gail:atk-bridge
  Xvfb :99 -screen 0 1600x1000x24 > /dev/null 2>&1 &
  sleep 1
  eval "$(dbus-launch --sh-syntax)"
  export DBUS_SESSION_BUS_ADDRESS
  /usr/libexec/at-spi-bus-launcher --launch-immediately > /dev/null 2>&1 &
  openbox > /dev/null 2>&1 &
  sleep 2
  gsettings set org.gnome.desktop.interface toolkit-accessibility true 2> /dev/null
fi

run "client version" "$L" --version
run "setup" "$L" setup --manual
export PATH="$HOME/.local/bin:$PATH"
run "which lighting" bash -c 'command -v lighting; readlink "$(command -v lighting)"'
run "unit tests" "$HOME/.lighting/venv/bin/python3" tests/test_units.py

if [ "$(uname)" = Darwin ]; then
  PY=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$HOME/.lighting/venv/bin/python3")
  say "python binary: $PY"
  NOW=$(date +%s)
  for svc in kTCCServiceAccessibility kTCCServiceScreenCapture kTCCServicePostEvent kTCCServiceListenEvent; do
    for db in "/Library/Application Support/com.apple.TCC/TCC.db" "$HOME/Library/Application Support/com.apple.TCC/TCC.db"; do
      sudo sqlite3 "$db" "INSERT OR REPLACE INTO access VALUES('$svc','$PY',1,2,4,1,NULL,NULL,0,'UNUSED',NULL,0,$NOW,NULL,NULL,'UNUSED',$NOW);" 2>> "$LOG" \
        || sudo sqlite3 "$db" "INSERT OR REPLACE INTO access VALUES('$svc','$PY',1,2,4,1,NULL,NULL,0,'UNUSED',NULL,0,$NOW);" 2>> "$LOG"
    done
  done
  run "ax trusted" "$HOME/.lighting/venv/bin/python3" -c 'import ApplicationServices as A; print("AXIsProcessTrusted", A.AXIsProcessTrusted())'
fi

run "status (cold start)" lighting status
run "doctor" lighting doctor --offline

PROFILE="$RUNNER_TEMP/profile"
mkdir -p "$PROFILE/NativeMessagingHosts"
cp "$HOME/.lighting/dev.qdhshamiro.lighting.json" "$PROFILE/NativeMessagingHosts/"
EXTRA=""
[ "$(uname)" = Linux ] && EXTRA="--no-sandbox"
"$CHROME" $EXTRA --user-data-dir="$PROFILE" --no-first-run --no-default-browser-check --disable-search-engine-choice-screen \
  --load-extension="$HOME/.lighting/extension" --window-size=1300,900 about:blank > "$RUNNER_TEMP/chrome.log" 2>&1 &
for i in $(seq 1 40); do
  lighting status 2>/dev/null | grep -q "^browser .* | ext " && break
  sleep 0.5
done
run "status (browser)" lighting status
run "chrome log" bash -c 'tail -25 "$RUNNER_TEMP/chrome.log"; ls -la "$RUNNER_TEMP/profile/NativeMessagingHosts"; cat ~/.lighting/host.sh' 
run "windows" lighting windows
run "open fixture" lighting open "file://$PWD/tests/fixture.html"
run "snap" lighting snap
run "selftest" lighting selftest
run "shot screen" lighting shot screen
cp "$HOME"/.lighting/out/*.jpg "$SHOTS/" 2> /dev/null
run "mcp ping" bash -c 'printf "%s\n" "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}" "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"lighting\",\"arguments\":{\"cmd\":\"ping\"}}}" | perl -e "alarm 20; exec @ARGV" lighting mcp'
run "daemon log tail" tail -40 "$HOME/.lighting/daemon.log"
