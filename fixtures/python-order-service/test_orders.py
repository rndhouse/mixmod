import unittest

from discounts import discount_rate
from orders import summarize_order


class OrderSummaryTests(unittest.TestCase):
    def test_standard_customer_has_no_discount(self):
        order = summarize_order(
            [{"sku": "book", "price": 20.0, "quantity": 2}],
            customer_tier="standard",
        )
        self.assertEqual(order, {"subtotal": 40.0, "discount": 0.0, "total": 40.0})

    def test_vip_discount_rate_requires_minimum_subtotal(self):
        self.assertEqual(discount_rate("vip", 99.99), 0.0)
        self.assertEqual(discount_rate("vip", 100.0), 0.10)

    def test_vip_order_applies_discount(self):
        order = summarize_order(
            [
                {"sku": "desk", "price": 80.0, "quantity": 1},
                {"sku": "lamp", "price": 25.0, "quantity": 2},
            ],
            customer_tier="vip",
        )
        self.assertEqual(order, {"subtotal": 130.0, "discount": 13.0, "total": 117.0})


if __name__ == "__main__":
    unittest.main()
