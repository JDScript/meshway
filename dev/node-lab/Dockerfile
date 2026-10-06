ARG BASE
FROM ${BASE}
ARG SOURCE_DIGEST
LABEL io.meshway.node-lab.image="1" io.meshway.node-lab.source=${SOURCE_DIGEST}
COPY debs/ /tmp/debs/
COPY packages.json /usr/share/meshway-lab/packages.json
RUN printf '#!/bin/sh\nexit 101\n' >/usr/sbin/policy-rc.d && chmod 755 /usr/sbin/policy-rc.d && \
    DEBIAN_FRONTEND=noninteractive apt-get --no-remove --no-install-recommends -y install /tmp/debs/*.deb && \
    dpkg-query -W -f='${binary:Package}\t${Version}\n' >/usr/share/meshway-lab/installed.tsv && \
    rm -rf /tmp/debs /var/lib/apt/lists/* && \
    mkdir -p /var/lib/meshway /run/meshway-lab && chmod 700 /var/lib/meshway
COPY entrypoint.sh runtime.py /usr/local/lib/meshway-lab/
RUN chmod 755 /usr/local/lib/meshway-lab/entrypoint.sh && \
    python3 -c 'import json,subprocess; p=json.load(open("/usr/share/meshway-lab/packages.json")); assert all(subprocess.check_output(["dpkg-query","-W","-f=${Version}",i["name"]],text=True)==i["version"] for i in p)' && \
    . /etc/os-release && test "$ID" = ubuntu && test "$VERSION_ID" = 24.04 && \
    ping -V && traceroute --version && iperf --version && iperf3 --version && ip -Version && dig -v && curl --version && resolvectl --version
ENTRYPOINT ["/usr/local/lib/meshway-lab/entrypoint.sh"]
