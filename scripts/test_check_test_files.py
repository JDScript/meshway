"""The size gate must reject new/growing files and retired exceptions."""

import unittest

from check_test_files import is_test_file, violations


class TestFileLimits(unittest.TestCase):
    def test_support_and_private_modules_are_in_scope(self):
        for path in [
            "crate/tests/suite/support/mod.rs",
            "crate/src/tests.rs",
            "crate/src/owner/service_tests.rs",
            "crate/src/owner/service_tests/streams.rs",
            "crate/src/owner/tests/mod.rs",
        ]:
            self.assertTrue(is_test_file(path), path)
        self.assertFalse(is_test_file("crate/src/owner.rs"))
        self.assertFalse(is_test_file("crate/tests/data.json"))

    def test_new_file_limit_and_existing_ceiling(self):
        old = "crate/src/old_tests.rs"
        new = "crate/tests/new.rs"
        self.assertEqual(violations({old: 900, new: 800}, {old: 900}), [])
        self.assertEqual(len(violations({old: 901, new: 801}, {old: 900})), 2)

    def test_obsolete_and_owned_exceptions_are_rejected(self):
        old = "crate/src/old_tests.rs"
        self.assertTrue(violations({old: 800}, {old: 900}))
        self.assertTrue(violations({}, {old: 900}))
        for path in [
            "apps/meshway-server/tests/large.rs",
            "crates/meshway-transport/src/session/tests.rs",
        ]:
            self.assertTrue(violations({path: 900}, {path: 900}))
