FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    dbus xrdp xorgxrdp xfce4 xfce4-terminal dbus-x11 xdotool x11-apps x11-utils \
    xterm sudo procps ca-certificates \
 && apt-get clean && rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /bin/bash tester
RUN echo 'xfce4-session' > /home/tester/.xsession && chown tester:tester /home/tester/.xsession
# Keep 32 bpp
RUN sed -i 's/^max_bpp=.*/max_bpp=32/' /etc/xrdp/xrdp.ini
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 3389
CMD ["/entrypoint.sh"]
