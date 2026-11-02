FROM rust:1.98.1-bookworm
ENV CARGO_INCREMENTAL=1
RUN apt-get update && apt-get install -y --no-install-recommends libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*
COPY cli/container/hosted-account-pool.py /opt/nyxid/hosted-account-pool.py
RUN python3 /opt/nyxid/hosted-account-pool.py && rm /opt/nyxid/hosted-account-pool.py
RUN groupadd --gid 1000 agent && useradd --uid 1000 --gid 1000 --home-dir /home/agent --shell /bin/sh agent \
    && groupadd --gid 1001 browser && useradd --uid 1001 --gid 1001 --home-dir /home/browser --shell /usr/sbin/nologin browser \
    && groupadd --gid 1002 devbrowser && useradd --uid 1002 --gid 1002 --home-dir /home/devbrowser --shell /usr/sbin/nologin devbrowser
