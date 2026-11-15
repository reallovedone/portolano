FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server vim htop procps ncurses-term coreutils \
 && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /run/sshd && rm -f /etc/ssh/ssh_host_* && useradd -m -s /bin/bash tester
COPY sshd_config /etc/ssh/sshd_config.d/portolano.conf
RUN printf 'export LANG=C.UTF-8\nexport LC_ALL=C.UTF-8\n' > /etc/profile.d/lang.sh
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
