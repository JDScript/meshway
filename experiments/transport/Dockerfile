# Native Linux only; override with another digest-pinned base when updating.
ARG BASE_IMAGE=alpine:3.22.1@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1
FROM ${BASE_IMAGE}
RUN apk add --no-cache iperf3 iproute2 python3
COPY node.py /opt/transport-lab/node.py
ARG LAB_RUN_ID=manual
LABEL io.meshway.experiment="transport" io.meshway.lab.run="${LAB_RUN_ID}"
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
CMD ["python3", "/opt/transport-lab/node.py", "serve"]
