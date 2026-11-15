#!/bin/sh
# Test-only VNC server (standard VNC authentication). VNC_PASSWORD comes from
# dev/keys/test.env; the protocol only uses its first 8 characters.
set -eu
: "${VNC_PASSWORD:?VNC_PASSWORD is not set: run dev/servers.sh up (it generates dev/keys/test.env)}"
install -d -o tester -g tester -m 700 /home/tester/.vnc
printf '%s\n' "$VNC_PASSWORD" | runuser -u tester -- vncpasswd -f > /home/tester/.vnc/passwd
chown tester:tester /home/tester/.vnc/passwd
chmod 600 /home/tester/.vnc/passwd
export HOME=/home/tester
# A stopped container keeps its filesystem: without this, restarting it fails with
# "Server is already active for display 0" because of the lock left by the previous run.
rm -f /tmp/.X0-lock /tmp/.X11-unix/X0
# Start a minimal desktop once the X server is up.
runuser -u tester -- sh -c 'sleep 2; DISPLAY=:0 openbox >/dev/null 2>&1 & DISPLAY=:0 xterm -geometry 100x30+20+20 >/dev/null 2>&1 &' &
exec runuser -u tester -- Xvnc :0 -geometry 1280x800 -depth 24 \
  -rfbport 5900 -SecurityTypes VncAuth -PasswordFile /home/tester/.vnc/passwd -AlwaysShared
