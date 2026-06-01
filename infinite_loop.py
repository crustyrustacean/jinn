#!/usr/bin/env python3
import time

print("Starting infinite loop with 1 second delay...")
while True:
    print(f"Iteration at {time.strftime('%H:%M:%S')}")
    time.sleep(1)
