import unittest

import crypto
import crypto_analysis


class CryptoReportTests(unittest.TestCase):
    def test_quantiles_preserve_operation_and_length_boundaries(self):
        samples = [dict(operation=operation, bytes=size, iterations=100, cpuNs=cpu, wallNs=cpu * 2)
                   for operation, size, cpu in [("seal", 52, 1000), ("seal", 52, 3000),
                                                 ("open", 52, 4000), ("seal", 1280, 5000)]]
        groups = {(row["operation"], row["bytes"]): row for row in crypto.aggregate(samples)}
        self.assertEqual(groups["seal", 52]["cpuNsPerOp"]["median"], 20)
        self.assertEqual(groups["seal", 1280]["cpuNsPerOp"]["median"], 50)
        self.assertEqual(groups["open", 52]["wallNsPerOp"]["median"], 80)

    def test_missing_or_invalid_cpu_is_not_zero(self):
        for value in [None, 0, -1, float("nan")]:
            with self.assertRaises(ValueError):
                crypto.aggregate([dict(operation="seal", bytes=52, iterations=100, cpuNs=value, wallNs=1)])

    def test_nested_categories_are_disjoint(self):
        self.assertEqual(crypto_analysis.category(["sha2::compress", "session::ignored", "meshway_dataplane::secure::session::authenticate"]), "hmac")
        self.assertEqual(crypto_analysis.category(["meshway_dataplane::secure::codec::Frame::validate", "meshway_dataplane::secure::session::authenticate"]), "frame-codec")
        self.assertEqual(crypto_analysis.category(["memcpy", "chacha20poly1305::seal"]), "buffer-copy")
        self.assertEqual(crypto_analysis.category([
            "core::ops::FnOnce<(meshway_dataplane::packet::Packet,)>::call_once",
            "<meshway_dataplane::engine::Engine>::admit_inner",
        ]), "flow-map-replay-policy")

    def test_native_self_cpu_uses_leaf_and_cpu_delta_not_stack_depth(self):
        symbols = dict(string_table=["sha2::compress", "meshway_dataplane::secure::session::authenticate"], data=[dict(
            debug_name="bench", debug_id="ec4622d3-9f6f-3f68-b562-843175b0508b", symbol_table=[dict(symbol=0), dict(symbol=1)],
            known_addresses=[[16, 0], [32, 1]])])
        thread = dict(name="meshway-circuit-perf", isMainThread=True,
                      frameTable=dict(func=[0, 1], address=[16, 32]),
                      funcTable=dict(resource=[0, 0], name=[0, 1]), resourceTable=dict(lib=[0]),
                      stringArray=["0x10", "0x20"], stackTable=dict(frame=[1, 0], prefix=[None, 0]),
                      samples=dict(time=[100, 300, 400, 800], stack=[1, 1, 0, 1], threadCPUDelta=[1, 3, 1, 1]))
        result = crypto_analysis.sampled(dict(libs=[dict(debugName="bench", breakpadId="EC4622D39F6F3F68B562843175B0508B0")], threads=[thread]), symbols, 20)
        self.assertEqual(result["samples"], 2)
        self.assertEqual(result["groups"]["hmac"]["cpuUsPerPacket"], 20)
        self.assertEqual(result["top15"][0]["cpuUsPerPacket"], 15)
        self.assertEqual(result["unresolvedLeafSamples"], 0)


if __name__ == "__main__":
    unittest.main()
