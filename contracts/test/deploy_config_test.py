"""Exercise deployment configuration using a mock forge; never broadcast transactions."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class DeploymentConfigTest(unittest.TestCase):
    def run_script(self, key, rpc="http://127.0.0.1:8545", broadcast=False):
        with tempfile.TemporaryDirectory(prefix="red-leaves-config-test-") as directory:
            root = Path(directory)
            (root / "script").mkdir()
            shutil.copy(Path(__file__).resolve().parents[1] / "script/deploy.sh", root / "script/deploy.sh")
            (root / ".env.profile").write_text(
                f"BASE_SEPOLIA_RPC_URL={rpc}\nDEPLOYER_PRIVATE_KEY={key}\n"
            )
            mock = root / "forge"
            # Public synthetic test key; no user configuration is read.
            mock.write_text(
                '#!/usr/bin/env python3\nimport os, sys\n'
                'assert os.environ["DEPLOYER_PRIVATE_KEY"] == "0x" + "0" * 63 + "1"\n'
                'assert sys.argv[1:5] == ["script", "script/DeployUserRegistry.s.sol:DeployUserRegistry", "--rpc-url", "base_sepolia"]\n'
                'assert sys.argv[5:] == (["--broadcast"] if os.environ["EXPECT_BROADCAST"] == "1" else [])\n'
                'print("Mock deployment invoked")\n'
            )
            mock.chmod(0o700)
            environment = {**os.environ, "PATH": str(root) + os.pathsep + os.environ["PATH"],
                           "EXPECT_BROADCAST": "1" if broadcast else "0"}
            return subprocess.run(
                ["bash", str(root / "script/deploy.sh"), *(["--broadcast"] if broadcast else [])],
                env=environment, capture_output=True, text=True, check=False,
            )

    def test_prefix_formats_are_normalized(self):
        for prefix in ("", "0x", "0X"):
            for broadcast in (False, True):
                with self.subTest(prefix=prefix, broadcast=broadcast):
                    result = self.run_script(prefix + "0" * 63 + "1", broadcast=broadcast)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("Mock deployment invoked", result.stdout)

    def test_invalid_or_missing_keys_are_rejected(self):
        for key in ("", "a" * 63, "g" * 64, "0x" + "a" * 65):
            with self.subTest(length=len(key)):
                result = self.run_script(key)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("exactly 64 hexadecimal characters", result.stderr)
                self.assertNotIn("Mock deployment invoked", result.stdout)
                if key:
                    self.assertNotIn(key, result.stderr)

    def test_zero_key_is_rejected(self):
        for prefix in ("", "0x", "0X"):
            result = self.run_script(prefix + "0" * 64)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must not be zero", result.stderr)

    def test_missing_rpc_is_reported_separately(self):
        result = self.run_script("0" * 63 + "1", rpc="")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Set BASE_SEPOLIA_RPC_URL", result.stderr)


if __name__ == "__main__":
    unittest.main()
