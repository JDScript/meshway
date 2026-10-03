# syntax=docker/dockerfile:1
# Runtime-only recipe. build.py verifies the native binary image and source manifest.
ARG MESHWAY_BINARY_IMAGE
FROM ${MESHWAY_BINARY_IMAGE} AS binaries
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl iproute2 iptables iputils-ping nftables python3 util-linux \
    && rm -rf /var/lib/apt/lists/* \
    && if [ -f /etc/iproute2/rt_tables.d/README ]; then \
         mv /etc/iproute2/rt_tables.d/README /usr/share/doc/iproute2/rt_tables.d.README; \
       fi
COPY --from=binaries /usr/local/bin/meshway-agent /usr/local/bin/meshway-controller /usr/local/bin/
COPY --from=binaries /source-manifest.json /source-manifest.json
COPY --chmod=755 runtime-bootstrap /usr/local/bin/setpriv
RUN ln -s /usr/local/bin/meshway-agent /usr/local/bin/meshway
RUN --network=none meshway-agent --help >/dev/null && meshway-controller --help >/dev/null
ENV MESHWAY_STATE=/state/agent.json
COPY container.py /opt/meshway-managed/container.py
COPY runtime-manifest.json /runtime-manifest.json
ENTRYPOINT ["sleep", "infinity"]
