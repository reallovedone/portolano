FROM debian:bookworm-slim
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    tigervnc-standalone-server tigervnc-tools openbox xterm x11-utils procps \
 && rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /bin/bash tester
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 5900
ENTRYPOINT ["/entrypoint.sh"]
