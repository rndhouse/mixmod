import unittest

from calculator import divide


class DivideTests(unittest.TestCase):
    def test_divides_numbers(self):
        self.assertEqual(divide(8, 2), 4)

    def test_divide_by_zero_raises_value_error(self):
        with self.assertRaises(ValueError):
            divide(1, 0)


if __name__ == "__main__":
    unittest.main()
