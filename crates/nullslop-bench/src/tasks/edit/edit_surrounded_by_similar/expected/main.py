def check_temperature(city, temp):
    if city == "New York":
        if temp > 35:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 25:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "London":
        if temp > 30:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 20:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Paris":
        if temp > 33:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 22:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Berlin":
        if temp > 32:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 23:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Sydney":
        if temp > 40:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 30:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Moscow":
        if temp > 30:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 20:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Tokyo":
        if temp > 32:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 22:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Dubai":
        if temp > 45:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 35:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Singapore":
        if temp > 35:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 30:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")

    if city == "Toronto":
        if temp > 33:
            print(f"{city}: Extreme heat alert! ({temp}C)")
        elif temp > 25:
            print(f"{city}: Warm weather ({temp}C)")
        else:
            print(f"{city}: Normal ({temp}C)")


def main():
    cities = {
        "New York": 28,
        "London": 22,
        "Paris": 26,
        "Berlin": 24,
        "Sydney": 35,
        "Moscow": 21,
        "Tokyo": 30,
        "Dubai": 42,
        "Singapore": 32,
        "Toronto": 27,
    }
    for city, temp in cities.items():
        check_temperature(city, temp)


if __name__ == "__main__":
    main()
