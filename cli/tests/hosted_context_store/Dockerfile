FROM rust:1.98.1-bookworm
ENV CARGO_INCREMENTAL=1
RUN apt-get update && apt-get install -y --no-install-recommends libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*
COPY cli/container/hosted-account-pool.py /opt/nyxid/hosted-account-pool.py
RUN python3 /opt/nyxid/hosted-account-pool.py && rm /opt/nyxid/hosted-account-pool.py
