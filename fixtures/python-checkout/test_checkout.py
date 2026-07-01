import unittest

from checkout import summarize_cart
from promotions import line_discount, order_discount


class CheckoutTests(unittest.TestCase):
    def test_standard_cart_without_discounts(self):
        summary = summarize_cart(
            [
                {"sku": "notebook", "quantity": 2},
                {"sku": "marker", "quantity": 3},
            ]
        )
        self.assertEqual(
            summary,
            {
                "lines": [
                    {
                        "sku": "notebook",
                        "quantity": 2,
                        "subtotal": 12.0,
                        "discount": 0.0,
                        "total": 12.0,
                    },
                    {
                        "sku": "marker",
                        "quantity": 3,
                        "subtotal": 7.5,
                        "discount": 0.0,
                        "total": 7.5,
                    },
                ],
                "subtotal": 19.5,
                "discount": 0.0,
                "total": 19.5,
            },
        )

    def test_marker_bulk_discount(self):
        self.assertEqual(line_discount("marker", 9, 2.5), 0.0)
        self.assertEqual(line_discount("marker", 10, 2.5), 2.5)

        summary = summarize_cart([{"sku": "marker", "quantity": 10}])
        self.assertEqual(summary["lines"][0]["subtotal"], 25.0)
        self.assertEqual(summary["lines"][0]["discount"], 2.5)
        self.assertEqual(summary["lines"][0]["total"], 22.5)
        self.assertEqual(summary["discount"], 2.5)
        self.assertEqual(summary["total"], 22.5)

    def test_vip_order_discount_after_line_discounts(self):
        self.assertEqual(order_discount("standard", 60.0), 0.0)
        self.assertEqual(order_discount("vip", 49.99), 0.0)
        self.assertEqual(order_discount("vip", 50.0), 2.5)

        summary = summarize_cart(
            [
                {"sku": "notebook", "quantity": 8},
                {"sku": "marker", "quantity": 10},
            ],
            customer_tier="vip",
        )
        self.assertEqual(summary["subtotal"], 73.0)
        self.assertEqual(summary["discount"], 6.03)
        self.assertEqual(summary["total"], 66.97)

    def test_invalid_sku_and_stock_errors(self):
        with self.assertRaisesRegex(ValueError, "unknown sku: missing"):
            summarize_cart([{"sku": "missing", "quantity": 1}])

        with self.assertRaisesRegex(ValueError, "insufficient stock: tote"):
            summarize_cart([{"sku": "tote", "quantity": 3}])


if __name__ == "__main__":
    unittest.main()
