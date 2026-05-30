class NumberProcessor:
    def __init__(self):
        self.numbers = []

    def load(self, data):
        self.numbers = list(data)

    def sum(self):
        return sum(self.numbers)

    def average(self):
        if not self.numbers:
            return 0.0
        return sum(self.numbers) / len(self.numbers)

    def median(self):
        if not self.numbers:
            return 0.0
        sorted_nums = sorted(self.numbers)
        n = len(sorted_nums)
        mid = n // 2
        if n % 2 == 0:
            return (sorted_nums[mid - 1] + sorted_nums[mid]) / 2
        return float(sorted_nums[mid])

    def __str__(self):
        return (
            f"Numbers: {self.numbers}\n"
            f"Sum: {self.sum()}\n"
            f"Average: {self.average():.2f}\n"
            f"Median: {self.median():.1f}"
        )


def main():
    processor = NumberProcessor()
    processor.load([1, 5, 3, 9, 2, 7, 4, 8, 6])
    print(processor)


if __name__ == "__main__":
    main()
