#!/bin/bash
# Takes the screenshots in data/screenshots/ again: the real UI against an
# invented account (daemon/cmd/demodaemon), in a virtual X server, so no real
# chat is ever on screen. Needs Xvfb, ImageMagick, python3-pillow, go, cargo
# and the emoji font, installed by the RPM or kept in ~/.local/share.
#
#     packaging/screenshots/screenshots.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
here=$root/packaging/screenshots
work=$(mktemp -d)
# The socket's path must fit in 108 bytes, so it cannot go in $work.
run=$XDG_RUNTIME_DIR/wassup-screenshots
display=:99

font=
for f in /usr/share/wassup/fonts/TwemojiCBDT.ttf ~/.local/share/wassup/fonts/TwemojiCBDT.ttf; do
    [ -f "$f" ] && { font=$f; break; }
done
[ -n "$font" ] || { echo "TwemojiCBDT.ttf not found" >&2; exit 1; }

pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; rm -rf "$work" "$run"; }
trap cleanup EXIT

(cd "$root/ui" && cargo build --release)
(cd "$root/daemon" && go build -o "$work/demodaemon" ./cmd/demodaemon)
mkdir -p "$work"/{media,state/wassup,config,cache,data/wassup/fonts} "$run" "$root/data/screenshots"
chmod 700 "$run"
cp "$font" "$work/data/wassup/fonts/"
python3 -I "$here/art.py" "$work/media"

cat > "$work/state/wassup/ui.ini" <<INI
[window]
width=1280
height=720
maximized=false
chat=120363000000000010@g.us
INI

Xvfb $display -screen 0 1280x720x24 >/dev/null 2>&1 &
pids+=($!)
XDG_RUNTIME_DIR=$run "$work/demodaemon" -media "$work/media" >"$work/daemon.log" 2>&1 &
pids+=($!)
sleep 2

for style in light dark; do
    # No session bus: the window must not join a running Wassup.
    env -u DBUS_SESSION_BUS_ADDRESS DISPLAY=$display GDK_BACKEND=x11 XDG_RUNTIME_DIR=$run \
        XDG_STATE_HOME=$work/state XDG_CONFIG_HOME=$work/config XDG_CACHE_HOME=$work/cache \
        XDG_DATA_HOME=$work/data LANGUAGE=en LC_ALL=en_US.UTF-8 ADW_DEBUG_COLOR_SCHEME=prefer-$style \
        "$root/ui/target/release/wassup" >"$work/ui-$style.log" 2>&1 &
    ui=$!
    sleep 7
    DISPLAY=$display import -window root "$work/$style.png"
    kill $ui
    wait $ui 2>/dev/null || true
    python3 -I "$here/round-corners.py" "$work/$style.png" "$root/data/screenshots/chat-$style.png"
done
echo "wrote data/screenshots/chat-light.png and chat-dark.png"
