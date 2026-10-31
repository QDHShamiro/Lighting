#!/usr/bin/env bash
set -u
OUT="$RUNNER_TEMP/selftest.txt"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=.exe ;;
  *) EXE= ;;
esac
target/release/wisply$EXE setup --manual
L="$HOME/.wisply/bin/wisply$EXE"
HOST="$HOME/.wisply/dev.qdhshamiro.wisply.json"
PROFILE="$RUNNER_TEMP/cft-profile"
mkdir -p "$PROFILE/NativeMessagingHosts"
cp "$HOST" "$PROFILE/NativeMessagingHosts/"
if [ -n "$EXE" ]; then
  WIN_HOST=$(cygpath -w "$HOST")
  for key in "Google\\Chrome for Testing" "Google\\Chrome" "Chromium"; do
    reg add "HKCU\\Software\\$key\\NativeMessagingHosts\\dev.qdhshamiro.wisply" //ve //t REG_SZ //d "$WIN_HOST" //f > /dev/null
  done
fi
if [ "$(uname -s)" = Linux ]; then
  export DISPLAY=:99
  Xvfb :99 -screen 0 1600x1000x24 > /dev/null 2>&1 &
  sleep 1
fi
CHROME=$(npx -y @puppeteer/browsers install chrome@stable --path "$RUNNER_TEMP/cft" | tail -1)
CHROME="${CHROME#* }"
echo "chrome: $CHROME"
EXTRA=""
[ "$(uname -s)" = Linux ] && EXTRA="--no-sandbox"
EXT="$HOME/.wisply/extension"
[ -n "$EXE" ] && EXT=$(cygpath -w "$EXT")
"$CHROME" $EXTRA --user-data-dir="$PROFILE" --no-first-run --no-default-browser-check --disable-search-engine-choice-screen \
  --load-extension="$EXT" --window-size=1300,900 about:blank > "$RUNNER_TEMP/chrome.log" 2>&1 &
for _ in $(seq 1 60); do
  "$L" status 2> /dev/null | grep -q "^browser .* | ext " && break
  sleep 0.5
done
"$L" status
"$L" selftest | tee "$OUT"
grep -E "^(FAIL|SKIP)" "$OUT" | sed 's/^/::warning title=selftest::/' || true
if ! tail -1 "$OUT" | grep -qE "^([0-9]+)/\1 passed$"; then
  tail -25 "$RUNNER_TEMP/chrome.log"
  tail -40 "$HOME/.wisply/daemon.log"
  exit 1
fi
