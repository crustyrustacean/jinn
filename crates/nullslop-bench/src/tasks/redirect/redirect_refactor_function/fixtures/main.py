def calculate_area(length, width):
    return length * width

def calculate_perimeter(length, width):
    return 2 * (length + width)

def main():
    rooms = [
        (5.0, 4.0),
        (3.0, 3.5),
        (6.0, 2.0),
    ]

    for length, width in rooms:
        area = calculate_area(length, width)
        perimeter = calculate_perimeter(length, width)
        print(f"Room {length}x{width}: area={area}, perimeter={perimeter}")
