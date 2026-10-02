# Agent underlay acceptance

The normal acceptance exercises Controller enrollment and generated configuration,
the Agent lifecycle, actual Linux TUN traffic and lease withdrawal. The mechanism
runners additionally exercise protected HTTP/TLS, native carriers, DNS alternatives,
namespace cookies and exact journal recovery in disposable dual-stack namespaces.

```sh
python3 -m unittest discover -s apps/agent/tests/underlay_fixture -p 'test_*.py' -v
python3 apps/agent/tests/underlay_fixture/main_loop.py
# Reuse the image ID in that run's result.json:
python3 apps/agent/tests/underlay_fixture/run.py --skip-build --image <image-id>
python3 apps/agent/tests/underlay_fixture/review.py --skip-build --image <image-id>
```

`main_loop.py` compiles in a non-root, cap-drop-ALL container with two CPUs and
bounded memory, then builds only the runtime layer with the ordinary Docker
builder. All Docker calls retain the coordinator's `scripts/performance_lock.py`
descriptor. No host networking, privileged builder or host routing/DNS/firewall
change is used. NET_ADMIN and TUN access are confined to owned test namespaces;
UID/GID 65534 checks really lose SO_MARK permission.

The image carries a complete source-input manifest, including nested modules,
extracted crates, manifests and runtime helpers. Reuse verifies the exact path set
and hashes. The image has two test executables: `/underlay-tests` for App HTTP,
bootstrap and carrier orchestration, and `/underlay-core-tests` for private
`agent-underlay` journal/socket tests. Every selected test must actually collect
one test; process-death cases additionally verify the intended exit boundary.

App fixtures obtain capabilities from the public Linux owner and the real system
DNS snapshot. Private mark/alias/DNS-alternative reproductions stay in underlay's
unit-test modules; no production constructor exposes a fabricated reservation or
TUN identity. Low-level device-open cancellation is kernel ownership evidence;
it is separate from the normal Controller/Agent/TUN acceptance.

Artifacts and all failed attempts remain under the approved private TMP in
`meshway-underlay-main-loop-runs/` or `meshway-underlay-artifacts/`. Syscall audits
require successful mark readback before protected bind/connect/listen. The remote
observer and actual capture TUN independently verify foreign VPN bits, ordinary
same-destination controls, inherited accepts and retirement fences. App-owned
reject-only captures also verify that ordinary traffic has no fall-through.
