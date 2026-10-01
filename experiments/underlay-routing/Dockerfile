FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a
RUN apt-get update && apt-get install -y --no-install-recommends python3 iproute2 openssl \
    && rm -rf /var/lib/apt/lists/* \
    && openssl req -x509 -newkey rsa:2048 -nodes -keyout /fixture.key -out /fixture.crt \
       -days 30 -subj /CN=underlay.fixture -addext subjectAltName=DNS:underlay.fixture
COPY helper.py fixture.py /lab/
ENTRYPOINT ["python3", "/lab/fixture.py"]
