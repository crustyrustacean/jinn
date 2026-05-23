# A simple word counter

def count_words(text):
    return len(text.split())

def count_chars(text):
    return len(text)

def main():
    with open("input.txt") as f:
        text = f.read()

    print(f"Words: {count_words(text)}")
    print(f"Characters: {count_chars(text)}")

if __name__ == "__main__":
    main()
