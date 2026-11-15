#!/bin/sh
# Test-only VNC server with VeNCrypt (security types X509Vnc and X509Plain). Both use the
# VNC_PASSWORD from dev/keys/test.env (X509Plain: user "tester", checked through PAM).
# The self-signed certificate is created here at every start and never leaves the container,
# so a restarted container presents a NEW certificate.
set -eu
: "${VNC_PASSWORD:?VNC_PASSWORD is not set: run dev/servers.sh up (it generates dev/keys/test.env)}"
install -d -o tester -g tester -m 700 /home/tester/.vnc
printf '%s\n' "$VNC_PASSWORD" | runuser -u tester -- vncpasswd -f > /home/tester/.vnc/passwd
chown tester:tester /home/tester/.vnc/passwd
chmod 600 /home/tester/.vnc/passwd
# X509Plain: the PAM stack checks the Unix password of "tester".
printf 'tester:%s\n' "$VNC_PASSWORD" | chpasswd
# Throwaway self-signed certificate (RSA, valid for a day).
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=portolano-dev-vnc-tls" \
  -keyout /home/tester/.vnc/x509.key -out /home/tester/.vnc/x509.pem 2>/dev/null
chown tester:tester /home/tester/.vnc/x509.key /home/tester/.vnc/x509.pem
chmod 600 /home/tester/.vnc/x509.key
export HOME=/home/tester
# The tests abandon connections on purpose (certificate prompts, wrong passwords); the default
# blacklist would lock the test client out after a few of them.
# Remove the X lock left by a previous run (see entrypoint.sh).
rm -f /tmp/.X0-lock /tmp/.X11-unix/X0
runuser -u tester -- sh -c 'sleep 2; DISPLAY=:0 openbox >/dev/null 2>&1 & DISPLAY=:0 xterm -geometry 100x30+20+20 >/dev/null 2>&1 &' &
exec runuser -u tester -- Xvnc :0 -geometry 1280x800 -depth 24 \
  -rfbport 5900 -SecurityTypes X509Vnc,X509Plain -PlainUsers tester \
  -X509Cert /home/tester/.vnc/x509.pem -X509Key /home/tester/.vnc/x509.key \
  -PasswordFile /home/tester/.vnc/passwd -AlwaysShared \
  -BlacklistThreshold 1000 -BlacklistTimeout 1
