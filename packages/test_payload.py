"""Exercise manual completeness and mutable-state rejection."""

import tempfile
import unittest
from pathlib import Path

from stage import stage_data
from validate import validate_data


class PayloadTests(unittest.TestCase):
    """Check real declared manuals and reject incomplete staged output."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        stage_data(sorted(Path("docs/man").glob("*")), self.root)

    def test_manuals_pass(self) -> None:
        validate_data(self.root)

    def test_missing_language_reference_is_rejected(self) -> None:
        (self.root / "usr/share/man/man5/gcsl.5.gz").unlink()
        with self.assertRaisesRegex(ValueError, "missing manual"):
            validate_data(self.root)

    def test_mutable_state_is_rejected(self) -> None:
        (self.root / "var").mkdir()
        with self.assertRaisesRegex(ValueError, "mutable state"):
            validate_data(self.root)


if __name__ == "__main__":
    unittest.main()
