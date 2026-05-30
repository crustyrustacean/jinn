def process_items(items):
    counter = 0
    for item in items:
        if item > 0:
            counter += 1
            print(f"Item {counter}: {item}")
    return counter


def summarize(data):
    counter = len([x for x in data if x > 0])
    print(f"Positive items: {counter}")
    for counter, value in enumerate(data):
        if value > 0:
            print(f"  [{counter}] = {value}")
    return counter


def build_report(entries):
    counter = 0
    results = []
    while counter < len(entries):
        entry = entries[counter]
        if entry.startswith("valid"):
            results.append(entry)
        counter += 1
    print(f"Valid entries found: {counter}")
    return results


def count_by_category(items, category_fn):
    counter = {}
    for item in items:
        cat = category_fn(item)
        if cat not in counter:
            counter[cat] = 0
        counter[cat] += 1
    return counter


def main():
    data = [3, -1, 7, 0, 5, -2, 8, 1, -3, 4]
    counter = process_items(data)
    print(f"Total positive: {counter}")

    counter = summarize(data)
    print(f"Last counter value: {counter}")

    entries = ["valid_a", "invalid_b", "valid_c", "valid_d", "invalid_e"]
    results = build_report(entries)
    print(f"Results: {results}")

    categories = count_by_category(data, lambda x: "positive" if x > 0 else "non_positive")
    print(f"Categories: {categories}")

    # A counter variable used in a comprehension
    squares = {counter: counter ** 2 for counter in range(5)}
    print(f"Squares: {squares}")


if __name__ == "__main__":
    main()
