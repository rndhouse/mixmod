from catalog import product_for


def summarize_cart(lines, customer_tier="standard"):
    result_lines = []
    subtotal = 0.0

    for line in lines:
        sku = line["sku"]
        quantity = line.get("quantity", 1)
        product = product_for(sku)
        line_subtotal = product["price"] * quantity
        subtotal += line_subtotal
        result_lines.append(
            {
                "sku": sku,
                "quantity": quantity,
                "subtotal": round(line_subtotal, 2),
                "discount": 0.0,
                "total": round(line_subtotal, 2),
            }
        )

    return {
        "lines": result_lines,
        "subtotal": round(subtotal, 2),
        "discount": 0.0,
        "total": round(subtotal, 2),
    }
