#!/bin/sh
# Test-only sshd. The password comes from TEST_PASSWORD (dev/keys/test.env, git-ignored) and
# the authorized key from the mounted /keys/authorized_keys.
set -eu
: "${TEST_PASSWORD:?TEST_PASSWORD is not set: run dev/servers.sh up (it generates dev/keys/test.env)}"
echo "tester:${TEST_PASSWORD}" | chpasswd
mkdir -p /home/tester/.ssh
if [ -f /keys/authorized_keys ]; then
  cp /keys/authorized_keys /home/tester/.ssh/authorized_keys
else
  : > /home/tester/.ssh/authorized_keys
fi
chown -R tester:tester /home/tester/.ssh
chmod 700 /home/tester/.ssh
chmod 600 /home/tester/.ssh/authorized_keys
ssh-keygen -A
exec /usr/sbin/sshd -D -e
