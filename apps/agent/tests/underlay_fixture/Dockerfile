FROM rust:1.98-trixie AS build
RUN rustup component add clippy
WORKDIR /src
ENV CARGO_TARGET_DIR=/src/target/native-underlay
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/controller ./apps/controller
COPY apps/agent ./apps/agent
COPY apps/apple/core ./apps/apple/core
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    CARGO_BUILD_JOBS=2 cargo clippy --locked -p meshway-agent --all-features --lib --tests -- -D warnings \
    && CARGO_BUILD_JOBS=2 cargo test --locked -p meshway-agent --all-features --lib --no-run --message-format=json > /tests.json \
    && python3 -c 'import json,pathlib,shutil; a=[j["executable"] for l in pathlib.Path("/tests.json").read_text().splitlines() if (j:=json.loads(l)).get("reason")=="compiler-artifact" and j.get("executable") and j["target"]["name"]=="meshway_agent"]; assert len(a)==1,a; shutil.copyfile(a[0],"/underlay-tests")'

FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
COPY apps/agent/tests/underlay_fixture/tls.cnf /tls.cnf
RUN apt-get update && apt-get install -y --no-install-recommends python3 iproute2 openssl strace \
    && rm -rf /var/lib/apt/lists/* \
    && openssl req -x509 -newkey rsa:2048 -nodes -keyout /root.key -out /fixture-root.crt \
       -days 30 -subj /CN=isolated-underlay-root -addext 'basicConstraints=critical,CA:TRUE' \
    && openssl req -new -newkey rsa:2048 -nodes -keyout /fixture.key -out /fixture.csr -subj /CN=underlay.fixture \
    && openssl x509 -req -in /fixture.csr -CA /fixture-root.crt -CAkey /root.key -CAcreateserial \
       -out /fixture.crt -days 30 -extfile /tls.cnf \
    && rm /root.key /fixture.csr \
    && chmod 644 /fixture-root.crt /fixture.crt
COPY --from=build --chmod=755 /underlay-tests /underlay-tests
COPY --from=build /src/apps/agent/src/underlay /sources/underlay
COPY experiments/underlay-routing/helper.py experiments/underlay-routing/fixture.py /lab/
COPY apps/agent/tests/underlay_fixture/namespace.py /lab/production.py
ENTRYPOINT ["python3", "/lab/production.py"]
