def process_items(items):
    item_count = 0
    for item in items:
        if item > 0:
            item_count += 1
            print(f"Item {item_count}: {item}")
    return item_count


def summarize(data):
    item_count = len([x for x in data if x > 0])
    print(f"Positive items: {item_count}")
    for item_count, value in enumerate(data):
        if value > 0:
            print(f"  [{item_count}] = {value}")
    return item_count


def build_report(entries):
    item_count = 0
    results = []
    while item_count < len(entries):
        entry = entries[item_count]
        if entry.startswith("valid"):
            results.append(entry)
        item_count += 1
    print(f"Valid entries found: {item_count}")
    return results


def count_by_category(items, category_fn):
    item_count = {}
    for item in items:
        cat = category_fn(item)
        if cat not in item_count:
            item_count[cat] = 0
        item_count[cat] += 1
    return item_count


def main():
    data = [3, -1, 7, 0, 5, -2, 8, 1, -3, 4]
    item_count = process_items(data)
    print(f"Total positive: {item_count}")

    item_count = summarize(data)
    print(f"Last item_count value: {item_count}")

    entries = ["valid_a", "invalid_b", "valid_c", "valid_d", "invalid_e"]
    results = build_report(entries)
    print(f"Results: {results}")

    categories = count_by_category(data, lambda x: "positive" if x > 0 else "non_positive")
    print(f"Categories: {categories}")

    # A item_count variable used in a comprehension
    squares = {item_count: item_count ** 2 for item_count in range(5)}
    print(f"Squares: {squares}")


if __name__ == "__main__":
    main()
