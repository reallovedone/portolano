#!/bin/bash
# Test-only xrdp (TLS security layer, no NLA). Password from TEST_PASSWORD (dev/keys/test.env).
set -eu
: "${TEST_PASSWORD:?TEST_PASSWORD is not set: run dev/servers.sh up (it generates dev/keys/test.env)}"
echo "tester:${TEST_PASSWORD}" | chpasswd
rm -f /var/run/xrdp/*.pid
service dbus start
/usr/sbin/xrdp-sesman
exec /usr/sbin/xrdp --nodaemon
