import assert from "node:assert/strict";
import { test } from "node:test";
import { ControlClient, getNetworkState, watchNetwork } from "../src/index.ts";

const id = "11111111-1111-4111-8111-111111111111";
const time = "2026-10-05T00:00:00Z";
const path = { networkId: id };
const auth = { token: "numeric-test", maxReconnects: 0 };
const nonIntegerTokens = ["1.0000000000000001", "9007199254740991.1", "1e0", "1.0", "-0"];

function state(sequence = "1", nodes = "{}", links = "[]") {
  return `{"asOf":"${time}","configurationRevision":1,"links":${links},"networkId":"${id}","nodes":${nodes},"paths":[],"sequence":${sequence},"stateEpoch":"${id}"}`;
}

function nodes(reportSequence = "null", ipInfo = "{}") {
  return `{"${id}":{"derived":{"ipInfo":${ipInfo}},"freshness":"current","hostname":"numeric-test","reportSequence":${reportSequence}}}`;
}

function ipInfo(data: string) {
  return `{"asn":{"status":"unavailable","reason":"datasetUnavailable"},"geoip":{"status":"available","datasetId":"${id}","lookedUpAt":"${time}","data":${data}}}`;
}

function client(text: string, stream = false, status = 200) {
  return new ControlClient({ baseUrl: "https://controller.example", fetch: async () => new Response(text, {
    status, headers: { "content-type": stream ? "text/event-stream" : "application/json" },
  }) });
}

// Raw bodies enter the public generated operations and the production transport;
// there is no test-only decoder or replacement response schema in this path.
async function checkState(text: string, accepted: boolean) {
  const http = await getNetworkState(client(text), path, auth);
  const event = `{"type":"state.snapshot","batch":"numeric-test","state":${text}}`;
  const stream = watchNetwork(client(`id: numeric-test\ndata: ${event}\n\n`, true), path, auth);
  try {
    const sse = await stream.next();
    assert.equal(sse.done, false);
    if (accepted) {
      const expected: unknown = JSON.parse(text);
      const expectedEvent: unknown = JSON.parse(event);
      assert.deepEqual(http, { ok: true, data: expected });
      assert.deepEqual(sse.value, { ok: true, data: { id: "numeric-test", data: expectedEvent } });
    } else {
      const failure = { ok: false, error: { kind: "decode", status: 200 } };
      assert.deepEqual(http, failure);
      assert.deepEqual(sse.value, failure);
      assert.equal((await stream.next()).done, true);
    }
  } finally {
    await stream.return(undefined);
  }
}

test("generated numeric HTTP/SSE reject rounded decimal, exponent and signed-zero integer tokens", async () => {
  for (const token of nonIntegerTokens) {
    await checkState(state(token), false);
    await checkState(state("1", nodes(token)), false);
  }
});

test("generated numeric HTTP/SSE preserve safe integer boundaries and nullable map values", async () => {
  for (const token of ["0", "1", "9007199254740991"]) {
    await checkState(state(token, nodes(token)), true);
  }
  await checkState(state("1", nodes()), true);
  for (const token of ["9007199254740992", "9007199254740993", "-1", "1e309"]) {
    await checkState(state(token), false);
  }
});

test("generated numeric HTTP/SSE never convert numeric markers to empty objects or arrays", async () => {
  for (const token of nonIntegerTokens) {
    await checkState(state("1", token), false);
    await checkState(state("1", "{}", token), false);
    // GeoIP data has only optional fields: a boxed-number marker would wrongly
    // pass this strict object as {}, including inside the generated typed map.
    await checkState(state("1", nodes("null", `{"2":${ipInfo(token)}}`)), false);
  }
});

test("generated numeric HTTP/SSE still reject duplicate decoded keys and malformed syntax", async () => {
  for (const text of [
    state('1,"sequence":2'),
    state('1,"\\u0073equence":1.0'),
    state("1", nodes('1,"\\u0072eportSequence":2')),
    state("01"),
    state("1e"),
  ]) await checkState(text, false);
});

test("generated number HTTP/SSE preserve floats in nested numeric-key maps and nullable fields", async () => {
  for (const token of [...nonIntegerTokens, "0.125", "1e-9999", "null"]) {
    const first = ipInfo(`{"position":{"latitude":1.25,"longitude":-0,"accuracyRadiusKm":${token}}}`);
    const second = ipInfo('{"position":{"latitude":-2.5e1,"longitude":1e0,"accuracyRadiusKm":null}}');
    await checkState(state("1", nodes("null", `{"10":${first},"2":${second},"1":${ipInfo('{"position":null}')}}`)), true);
  }
  for (const position of [
    '{"latitude":90.5,"longitude":0}',
    '{"latitude":0,"longitude":180.5}',
    '{"latitude":0,"longitude":0,"accuracyRadiusKm":-0.5}',
    '{"latitude":0,"longitude":0,"accuracyRadiusKm":1e309}',
    '{"latitude":0,"longitude":0,"accuracyRadiusKm":"1.0"}',
    '{"latitude":0,"longitude":0,"accuracyRadiusKm":{}}',
  ]) await checkState(state("1", nodes("null", `{"2":${ipInfo(`{"position":${position}}`)}}`)), false);
});

test("generated numeric HTTP/SSE problems require integer tokens and actual empty params objects", async () => {
  const valid = '{"status":400,"requestId":null,"problem":{"code":"invalid_request","message":"invalid","params":{}}}';
  for (const text of [valid, ...nonIntegerTokens.flatMap(token => [
    valid.replace('"status":400', `"status":${token}`),
    valid.replace('"params":{}', `"params":${token}`),
  ])]) {
    const expectedProblem: unknown = JSON.parse(valid);
    const expected = { ok: false, error: { kind: "http", status: 400, ...(text === valid ? { problem: expectedProblem } : {}) } };
    assert.deepEqual(await getNetworkState(client(text, false, 400), path, auth), expected);
    const stream = watchNetwork(client(text, false, 400), path, auth);
    try {
      assert.deepEqual((await stream.next()).value, expected);
      assert.equal((await stream.next()).done, true);
    } finally {
      await stream.return(undefined);
    }
  }
});
