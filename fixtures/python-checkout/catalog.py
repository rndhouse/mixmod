PRODUCTS = {
    "notebook": {"price": 6.0, "stock": 20},
    "marker": {"price": 2.5, "stock": 30},
    "tote": {"price": 12.0, "stock": 2},
}


def product_for(sku):
    return PRODUCTS.get(sku)
