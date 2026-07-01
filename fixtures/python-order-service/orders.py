from discounts import discount_rate


def summarize_order(items, customer_tier="standard"):
    subtotal = sum(item["price"] * item.get("quantity", 1) for item in items)
    discount = 0.0
    return {
        "subtotal": round(subtotal, 2),
        "discount": round(discount, 2),
        "total": round(subtotal - discount, 2),
    }
